//! Replay buffer: ffmpeg captures the screen (+ mixed audio from stdin) into
//! rolling 2-second MPEG-TS segments on disk. Saving a clip = joining the
//! newest segments with stream copy (no re-encode, takes well under a second).

use crate::audio::{self, AudioConfig, Levels, SourceCfg};
use crate::ffmpeg::{self, Tools};
use crate::platform;
use crate::settings::{self, CaptureMethod, Settings};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const SEGMENT_SECONDS: u32 = 2;

pub struct Recorder {
    child: Child,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    seg_dir: PathBuf,
    pub buffer_seconds: u32,
    pub started: Instant,
    pub encoder: String,
    pub audio_errors: Arc<Mutex<Vec<String>>>,
}

fn segment_number(p: &Path) -> Option<u64> {
    let name = p.file_name()?.to_str()?;
    name.strip_prefix("seg_")?.strip_suffix(".ts")?.parse().ok()
}

fn list_segments(dir: &Path) -> Vec<(u64, PathBuf)> {
    let mut v: Vec<(u64, PathBuf)> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    segment_number(&p).map(|n| (n, p))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|(n, _)| *n);
    v
}

pub fn buffer_dir() -> PathBuf {
    settings::cache_dir().join("buffer")
}

impl Recorder {
    pub fn start(s: &Settings, tools: &Tools, encoder: &str, levels: Arc<Levels>) -> Result<Self, String> {
        let seg_dir = buffer_dir();
        let _ = fs::remove_dir_all(&seg_dir);
        fs::create_dir_all(&seg_dir).map_err(|e| format!("Can't create buffer folder: {e}"))?;

        let has_audio = s.record_system_audio || s.record_mic;
        let fps = s.fps.clamp(10, 240);
        let mut a: Vec<String> = ["-hide_banner", "-loglevel", "warning", "-y"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let mut vf: Vec<String> = Vec::new();
        match s.capture {
            CaptureMethod::DesktopDuplication => {
                a.extend(["-f".into(), "lavfi".into(), "-i".into()]);
                a.push(format!(
                    "ddagrab=output_idx={}:framerate={}:draw_mouse={}",
                    s.monitor_index,
                    fps,
                    if s.draw_mouse { 1 } else { 0 }
                ));
                vf.push("hwdownload".into());
                vf.push("format=bgra".into());
            }
            CaptureMethod::Gdi => {
                a.extend([
                    "-f".into(),
                    "gdigrab".into(),
                    "-framerate".into(),
                    fps.to_string(),
                    "-draw_mouse".into(),
                    if s.draw_mouse { "1" } else { "0" }.into(),
                ]);
                if let Some(m) = platform::monitors().get(s.monitor_index as usize) {
                    a.extend([
                        "-offset_x".into(),
                        m.x.to_string(),
                        "-offset_y".into(),
                        m.y.to_string(),
                        "-video_size".into(),
                        format!("{}x{}", m.w, m.h),
                    ]);
                }
                a.extend(["-i".into(), "desktop".into()]);
            }
        }
        if has_audio {
            a.extend(
                [
                    "-f", "f32le", "-ar", "48000", "-ac", "2", "-thread_queue_size", "4096", "-i", "pipe:0",
                ]
                .iter()
                .map(|s| s.to_string()),
            );
        }
        if let Some(h) = s.resolution.height() {
            vf.push(format!("scale=-2:{h}:flags=bilinear"));
        }
        vf.push(format!("format={}", ffmpeg::pixel_format_for(encoder)));
        a.extend(["-vf".into(), vf.join(",")]);
        a.extend(ffmpeg::video_encoder_args(encoder, s.bitrate_kbps.max(500), true));
        a.extend(["-g".into(), (fps * SEGMENT_SECONDS).to_string()]);
        a.extend(["-map".into(), "0:v".into()]);
        if has_audio {
            a.extend([
                "-map".into(),
                "1:a".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                format!("{}k", s.audio_bitrate_kbps.clamp(64, 320)),
            ]);
        }
        a.extend([
            "-f".into(),
            "segment".into(),
            "-segment_time".into(),
            SEGMENT_SECONDS.to_string(),
            "-segment_format".into(),
            "mpegts".into(),
            seg_dir.join("seg_%06d.ts").to_string_lossy().into_owned(),
        ]);

        let log = fs::File::create(seg_dir.join("ffmpeg.log")).map_err(|e| e.to_string())?;
        let mut child = ffmpeg::cmd(&tools.ffmpeg)
            .args(&a)
            .stdin(if has_audio { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .map_err(|e| format!("Couldn't start ffmpeg: {e}"))?;

        let stop = Arc::new(AtomicBool::new(false));
        let audio_errors = Arc::new(Mutex::new(Vec::new()));
        let mut threads = Vec::new();

        if has_audio {
            let stdin = child.stdin.take().expect("piped stdin");
            let cfg = AudioConfig {
                system: s.record_system_audio.then(|| SourceCfg {
                    device: s.system_device.clone(),
                    volume: s.system_volume,
                }),
                mic: s.record_mic.then(|| SourceCfg {
                    device: s.mic_device.clone(),
                    volume: s.mic_volume,
                }),
            };
            threads.push(audio::spawn(cfg, stdin, stop.clone(), levels, audio_errors.clone()));
        }

        // Cleaner: keep only the newest segments on disk.
        {
            let stop = stop.clone();
            let dir = seg_dir.clone();
            let keep = (s.buffer_seconds / SEGMENT_SECONDS + 5) as usize;
            threads.push(thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let segs = list_segments(&dir);
                    if segs.len() > keep {
                        for (_, p) in &segs[..segs.len() - keep] {
                            let _ = fs::remove_file(p);
                        }
                    }
                    for _ in 0..10 {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }));
        }

        let mut rec = Recorder {
            child,
            stop,
            threads,
            seg_dir,
            buffer_seconds: s.buffer_seconds,
            started: Instant::now(),
            encoder: encoder.to_string(),
            audio_errors,
        };

        // Fail fast if ffmpeg rejected the arguments / capture device.
        thread::sleep(Duration::from_millis(900));
        if !rec.is_alive() {
            let tail = rec.log_tail();
            rec.stop();
            return Err(format!("Capture failed to start.\n{tail}"));
        }
        Ok(rec)
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn log_tail(&self) -> String {
        let text = fs::read_to_string(self.seg_dir.join("ffmpeg.log")).unwrap_or_default();
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(4)..].join("\n")
    }

    /// Newest segments covering at least the buffer length (incl. the one being written).
    pub fn recent_segments(&self) -> Vec<PathBuf> {
        let segs = list_segments(&self.seg_dir);
        let n = (self.buffer_seconds / SEGMENT_SECONDS + 2) as usize;
        segs[segs.len().saturating_sub(n)..]
            .iter()
            .map(|(_, p)| p.clone())
            .collect()
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{stem}.{ext}"));
    let mut i = 2;
    while p.exists() {
        p = dir.join(format!("{stem} ({i}).{ext}"));
        i += 1;
    }
    p
}

pub fn unique_clip_path(dir: &Path, stem: &str) -> PathBuf {
    unique_path(dir, stem, "mp4")
}

fn run_ffmpeg(tools: &Tools, args: &[String]) -> Result<(), String> {
    let out = ffmpeg::cmd(&tools.ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(3).collect();
        Err(tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
    }
}

/// Join segments into an mp4 in `dir`, trimmed to roughly the last `buffer` seconds.
pub fn write_clip(tools: &Tools, segs: &[PathBuf], buffer: u32, dir: &Path) -> Result<PathBuf, String> {
    if segs.is_empty() {
        return Err("Nothing recorded yet — wait a couple of seconds.".into());
    }
    fs::create_dir_all(dir).map_err(|e| format!("Can't create clips folder: {e}"))?;
    let stem = chrono::Local::now().format("Clip %Y-%m-%d %H-%M-%S").to_string();
    let out = unique_clip_path(dir, &stem);
    let tmp = dir.join(format!(".{stem}.part.mp4"));
    let concat = format!(
        "concat:{}",
        segs.iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("|")
    );
    let s = |v: &str| v.to_string();
    run_ffmpeg(
        tools,
        &[
            s("-hide_banner"), s("-loglevel"), s("error"), s("-y"),
            s("-i"), concat,
            s("-map"), s("0"), s("-c"), s("copy"),
            s("-movflags"), s("+faststart"), s("-f"), s("mp4"),
            tmp.to_string_lossy().into_owned(),
        ],
    )
    .map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Saving clip failed: {e}")
    })?;

    let dur = ffmpeg::probe(&tools.ffprobe, &tmp).map(|p| p.duration).unwrap_or(0.0);
    if dur > buffer as f64 + 1.0 {
        let res = run_ffmpeg(
            tools,
            &[
                s("-hide_banner"), s("-loglevel"), s("error"), s("-y"),
                s("-ss"), format!("{:.3}", dur - buffer as f64),
                s("-i"), tmp.to_string_lossy().into_owned(),
                s("-map"), s("0"), s("-c"), s("copy"),
                s("-avoid_negative_ts"), s("make_zero"),
                s("-movflags"), s("+faststart"),
                out.to_string_lossy().into_owned(),
            ],
        );
        if res.is_ok() {
            let _ = fs::remove_file(&tmp);
            return Ok(out);
        }
    }
    fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
    Ok(out)
}

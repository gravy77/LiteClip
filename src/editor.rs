//! Editor back-end: preview frames, thumbnails, lightweight playback, export.

use crate::ffmpeg::{self, Probe, Tools};
use eframe::egui::{self, ColorImage};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const PREVIEW_W: u32 = 640;
pub const PREVIEW_H: u32 = 360;
pub const THUMB_W: u32 = 160;
pub const THUMB_H: u32 = 90;

fn to_image(bytes: &[u8], w: u32, h: u32) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([w as usize, h as usize], bytes)
}

/// Background worker returning a single preview frame; only the newest request is served.
pub struct FrameGrabber {
    tx: Sender<(u64, PathBuf, f64)>,
    pub rx: Receiver<(u64, ColorImage)>,
    next_id: u64,
}

impl FrameGrabber {
    pub fn new(tools: Tools, ctx: egui::Context) -> Self {
        let (tx, req_rx) = mpsc::channel::<(u64, PathBuf, f64)>();
        let (res_tx, rx) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(mut req) = req_rx.recv() {
                while let Ok(r) = req_rx.try_recv() {
                    req = r;
                }
                let (id, path, t) = req;
                if let Some(b) = ffmpeg::grab_frame(&tools.ffmpeg, &path, t, PREVIEW_W, PREVIEW_H) {
                    let _ = res_tx.send((id, to_image(&b, PREVIEW_W, PREVIEW_H)));
                    ctx.request_repaint();
                }
            }
        });
        FrameGrabber { tx, rx, next_id: 0 }
    }

    pub fn request(&mut self, path: PathBuf, t: f64) -> u64 {
        self.next_id += 1;
        let _ = self.tx.send((self.next_id, path, t));
        self.next_id
    }
}

pub type ThumbResult = (PathBuf, Option<Probe>, Option<ColorImage>);

/// Background worker producing clip metadata + thumbnail.
pub struct ThumbWorker {
    tx: Sender<PathBuf>,
    pub rx: Receiver<ThumbResult>,
}

impl ThumbWorker {
    pub fn new(tools: Tools, ctx: egui::Context) -> Self {
        let (tx, req_rx) = mpsc::channel::<PathBuf>();
        let (res_tx, rx) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(path) = req_rx.recv() {
                let probe = ffmpeg::probe(&tools.ffprobe, &path);
                let t = probe.as_ref().map(|p| (p.duration * 0.3).min(2.0)).unwrap_or(0.0);
                let img = ffmpeg::grab_frame(&tools.ffmpeg, &path, t, THUMB_W, THUMB_H)
                    .map(|b| to_image(&b, THUMB_W, THUMB_H));
                if res_tx.send((path, probe, img)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        ThumbWorker { tx, rx }
    }

    pub fn request(&self, path: PathBuf) {
        let _ = self.tx.send(path);
    }
}

/// Low-res silent video playback via ffmpeg → raw RGBA, plus optional audio via ffplay.
pub struct Player {
    pub start_t: f64,
    rx: Receiver<ColorImage>,
    t0: Arc<Mutex<Option<Instant>>>,
    child: Arc<Mutex<Option<Child>>>,
    audio: Option<Child>,
    stop: Arc<AtomicBool>,
    pub finished: Arc<AtomicBool>,
}

impl Player {
    pub fn start(tools: &Tools, path: PathBuf, t: f64, with_audio: bool, ctx: egui::Context) -> Option<Self> {
        let vf = format!(
            "fps=30,scale={PREVIEW_W}:{PREVIEW_H}:force_original_aspect_ratio=decrease:flags=bilinear,pad={PREVIEW_W}:{PREVIEW_H}:(ow-iw)/2:(oh-ih)/2"
        );
        let mut child = ffmpeg::cmd(&tools.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{t:.3}"), "-i"])
            .arg(&path)
            .args(["-an", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let child = Arc::new(Mutex::new(Some(child)));
        let (tx, rx) = mpsc::sync_channel(3);
        let t0: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        {
            let (t0, stop, finished) = (t0.clone(), stop.clone(), finished.clone());
            thread::spawn(move || {
                let size = (PREVIEW_W * PREVIEW_H * 4) as usize;
                let mut buf = vec![0u8; size];
                let mut k: u32 = 0;
                while !stop.load(Ordering::Relaxed) {
                    if stdout.read_exact(&mut buf).is_err() {
                        break;
                    }
                    let start = *t0.lock().unwrap().get_or_insert_with(Instant::now);
                    let due = start + Duration::from_secs_f64(k as f64 / 30.0);
                    let now = Instant::now();
                    if due > now {
                        thread::sleep(due - now);
                    }
                    k += 1;
                    if tx.send(to_image(&buf, PREVIEW_W, PREVIEW_H)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
                finished.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            });
        }
        let audio = if with_audio {
            tools.ffplay.as_ref().and_then(|ffplay| {
                ffmpeg::cmd(ffplay)
                    .args(["-nodisp", "-autoexit", "-loglevel", "quiet", "-vn", "-ss", &format!("{t:.3}")])
                    .arg(&path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .ok()
            })
        } else {
            None
        };
        Some(Player {
            start_t: t,
            rx,
            t0,
            child,
            audio,
            stop,
            finished,
        })
    }

    pub fn position(&self) -> f64 {
        match *self.t0.lock().unwrap() {
            Some(t0) => self.start_t + t0.elapsed().as_secs_f64(),
            None => self.start_t,
        }
    }

    pub fn latest_frame(&self) -> Option<ColorImage> {
        let mut last = None;
        while let Ok(f) = self.rx.try_recv() {
            last = Some(f);
        }
        last
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(mut a) = self.audio.take() {
            let _ = a.kill();
            let _ = a.wait();
        }
    }
}

pub struct ExportParams {
    pub input: PathBuf,
    pub output: PathBuf,
    pub keep: Vec<(f64, f64)>,
    pub speed: f64,
    pub volume: f32,
    pub mute: bool,
    pub height: Option<u32>,
    pub fps: Option<u32>,
    pub encoder: String,
    pub bitrate_kbps: u32,
    pub has_audio: bool,
    pub fast: bool,
}

pub struct ExportJob {
    progress: Arc<AtomicU32>,
    pub result: Arc<Mutex<Option<Result<PathBuf, String>>>>,
    child: Arc<Mutex<Option<Child>>>,
    cancelled: Arc<AtomicBool>,
}

impl ExportJob {
    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(Ordering::Relaxed))
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }
}

fn atempo_chain(mut s: f64) -> Vec<f64> {
    let mut v = Vec::new();
    while s > 2.0 {
        v.push(2.0);
        s /= 2.0;
    }
    while s < 0.5 {
        v.push(0.5);
        s /= 0.5;
    }
    v.push(s);
    v
}

fn build_args(p: &ExportParams) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-y", "-loglevel", "error", "-progress", "pipe:1", "-nostats"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let inp = p.input.to_string_lossy().into_owned();
    let out = p.output.to_string_lossy().into_owned();
    if p.fast {
        let (s, e) = p.keep[0];
        a.extend([
            "-ss".into(), format!("{s:.3}"), "-i".into(), inp,
            "-t".into(), format!("{:.3}", e - s),
            "-map".into(), "0".into(), "-c".into(), "copy".into(),
            "-avoid_negative_ts".into(), "make_zero".into(),
            "-movflags".into(), "+faststart".into(), out,
        ]);
        return a;
    }
    let k = p.keep.len();
    let audio = p.has_audio && !p.mute;
    let mut f = format!("[0:v]split={k}");
    for i in 0..k {
        f += &format!("[vs{i}]");
    }
    f += ";";
    if audio {
        f += &format!("[0:a]asplit={k}");
        for i in 0..k {
            f += &format!("[as{i}]");
        }
        f += ";";
    }
    for (i, (s, e)) in p.keep.iter().enumerate() {
        f += &format!("[vs{i}]trim=start={s:.3}:end={e:.3},setpts=PTS-STARTPTS[v{i}];");
        if audio {
            f += &format!("[as{i}]atrim=start={s:.3}:end={e:.3},asetpts=PTS-STARTPTS[a{i}];");
        }
    }
    for i in 0..k {
        f += &format!("[v{i}]");
        if audio {
            f += &format!("[a{i}]");
        }
    }
    f += &format!("concat=n={k}:v=1:a={}[vc]", audio as u8);
    if audio {
        f += "[ac]";
    }
    let mut vpost: Vec<String> = Vec::new();
    if (p.speed - 1.0).abs() > 1e-3 {
        vpost.push(format!("setpts=PTS/{:.4}", p.speed));
    }
    if let Some(fps) = p.fps {
        vpost.push(format!("fps={fps}"));
    }
    if let Some(h) = p.height {
        vpost.push(format!("scale=-2:{h}"));
    }
    vpost.push(format!("format={}", ffmpeg::pixel_format_for(&p.encoder)));
    f += &format!(";[vc]{}[vo]", vpost.join(","));
    if audio {
        let mut apost: Vec<String> = Vec::new();
        if (p.speed - 1.0).abs() > 1e-3 {
            for t in atempo_chain(p.speed) {
                apost.push(format!("atempo={t:.4}"));
            }
        }
        if (p.volume - 1.0).abs() > 1e-3 {
            apost.push(format!("volume={:.3}", p.volume));
        }
        if apost.is_empty() {
            apost.push("anull".into());
        }
        f += &format!(";[ac]{}[ao]", apost.join(","));
    }
    a.extend(["-i".into(), inp, "-filter_complex".into(), f, "-map".into(), "[vo]".into()]);
    if audio {
        a.extend(["-map".into(), "[ao]".into(), "-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
    } else {
        a.push("-an".into());
    }
    a.extend(ffmpeg::video_encoder_args(&p.encoder, p.bitrate_kbps.max(500), false));
    a.extend(["-movflags".into(), "+faststart".into(), out]);
    a
}

pub fn export(tools: &Tools, p: ExportParams, ctx: egui::Context) -> ExportJob {
    let total: f64 = (p.keep.iter().map(|(s, e)| e - s).sum::<f64>() / p.speed).max(0.01);
    let progress = Arc::new(AtomicU32::new(0f32.to_bits()));
    let result = Arc::new(Mutex::new(None));
    let child_slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let cancelled = Arc::new(AtomicBool::new(false));
    let job = ExportJob {
        progress: progress.clone(),
        result: result.clone(),
        child: child_slot.clone(),
        cancelled: cancelled.clone(),
    };
    let args = build_args(&p);
    let spawned = ffmpeg::cmd(&tools.ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            *result.lock().unwrap() = Some(Err(e.to_string()));
            return job;
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    *child_slot.lock().unwrap() = Some(child);
    let err_text = Arc::new(Mutex::new(String::new()));
    if let Some(mut se) = stderr {
        let err_text = err_text.clone();
        thread::spawn(move || {
            let mut s = String::new();
            let _ = se.read_to_string(&mut s);
            *err_text.lock().unwrap() = s;
        });
    }
    thread::spawn(move || {
        if let Some(so) = stdout {
            for line in BufReader::new(so).lines().map_while(Result::ok) {
                if let Some(v) = line.strip_prefix("out_time_us=") {
                    if let Ok(us) = v.trim().parse::<f64>() {
                        let frac = (us / 1e6 / total).clamp(0.0, 1.0) as f32;
                        progress.store(frac.to_bits(), Ordering::Relaxed);
                        ctx.request_repaint();
                    }
                }
            }
        }
        let status = child_slot.lock().unwrap().take().and_then(|mut c| c.wait().ok());
        thread::sleep(Duration::from_millis(50));
        let res = if cancelled.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(&p.output);
            Err("Export cancelled".to_string())
        } else if status.map(|s| s.success()).unwrap_or(false) {
            Ok(p.output.clone())
        } else {
            let _ = std::fs::remove_file(&p.output);
            let e = err_text.lock().unwrap().clone();
            let tail: Vec<&str> = e.lines().rev().take(3).collect();
            Err(format!(
                "Export failed: {}",
                tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
            ))
        };
        *result.lock().unwrap() = Some(res);
        ctx.request_repaint();
    });
    job
}

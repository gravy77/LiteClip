//! Locating and driving the external ffmpeg / ffprobe / ffplay binaries.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// CREATE_NO_WINDOW: never flash a console window.
pub const NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub ffplay: Option<PathBuf>,
}

fn search_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
}

fn winget_ffmpeg() -> Option<PathBuf> {
    // winget installs ffmpeg under %LOCALAPPDATA%\Microsoft\WinGet\Packages\<id>\<ver>\bin
    let root = dirs::data_local_dir()?.join("Microsoft").join("WinGet").join("Packages");
    for pkg in std::fs::read_dir(root).ok()?.flatten() {
        let name = pkg.file_name().to_string_lossy().to_lowercase();
        if !name.contains("ffmpeg") {
            continue;
        }
        for ver in std::fs::read_dir(pkg.path()).ok()?.flatten() {
            let p = ver.path().join("bin").join("ffmpeg.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Find ffmpeg: custom path from settings, next to our exe, PATH, then winget.
pub fn locate(custom: &str) -> Option<Tools> {
    let custom = custom.trim().trim_matches('"');
    let ffmpeg = if !custom.is_empty() {
        let p = PathBuf::from(custom);
        let p = if p.is_dir() { p.join("ffmpeg.exe") } else { p };
        p.is_file().then_some(p)?
    } else {
        let beside = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("ffmpeg.exe")))
            .filter(|p| p.is_file());
        beside
            .or_else(|| search_path("ffmpeg.exe"))
            .or_else(winget_ffmpeg)?
    };
    let dir = ffmpeg.parent().map(Path::to_path_buf).unwrap_or_default();
    let ffprobe = dir.join("ffprobe.exe");
    let ffprobe = if ffprobe.is_file() {
        ffprobe
    } else {
        search_path("ffprobe.exe")?
    };
    let ffplay = Some(dir.join("ffplay.exe"))
        .filter(|p| p.is_file())
        .or_else(|| search_path("ffplay.exe"));
    Some(Tools {
        ffmpeg,
        ffprobe,
        ffplay,
    })
}

pub fn cmd(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    c.creation_flags(NO_WINDOW);
    c
}

/// Try hardware encoders in order and return the first that works.
pub fn detect_encoder(ffmpeg: &Path) -> String {
    for enc in ["h264_nvenc", "h264_amf", "h264_qsv"] {
        let ok = cmd(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=640x360:r=30:d=0.5",
                "-vf",
                "format=nv12",
                "-c:v",
                enc,
                "-f",
                "null",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return enc.to_string();
        }
    }
    "libx264".to_string()
}

/// Encoder-specific args. `realtime` = live capture (favour speed), else export (favour quality).
pub fn video_encoder_args(encoder: &str, kbps: u32, realtime: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["-c:v".into(), encoder.into()];
    let extra: &[&str] = match (encoder, realtime) {
        ("h264_nvenc", true) => &["-preset", "p2", "-tune", "ll", "-rc", "vbr"],
        ("h264_nvenc", false) => &["-preset", "p5", "-rc", "vbr"],
        ("h264_qsv", true) => &["-preset", "veryfast"],
        ("h264_qsv", false) => &["-preset", "medium"],
        ("h264_amf", true) => &["-quality", "speed", "-rc", "vbr_peak"],
        ("h264_amf", false) => &["-quality", "balanced", "-rc", "vbr_peak"],
        (_, true) => &["-preset", "ultrafast", "-tune", "zerolatency"],
        (_, false) => &["-preset", "veryfast"],
    };
    a.extend(extra.iter().map(|s| s.to_string()));
    a.extend([
        "-b:v".into(),
        format!("{kbps}k"),
        "-maxrate".into(),
        format!("{}k", kbps * 3 / 2),
        "-bufsize".into(),
        format!("{}k", kbps * 2),
    ]);
    a
}

pub fn pixel_format_for(encoder: &str) -> &'static str {
    if encoder == "libx264" {
        "yuv420p"
    } else {
        "nv12"
    }
}

#[derive(Clone, Debug, Default)]
pub struct Probe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
}

pub fn probe(ffprobe: &Path, file: &Path) -> Option<Probe> {
    let out = cmd(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(file)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let mut p = Probe {
        duration: v["format"]["duration"].as_str()?.parse().ok()?,
        ..Default::default()
    };
    for s in v["streams"].as_array().into_iter().flatten() {
        match s["codec_type"].as_str() {
            Some("video") if p.width == 0 => {
                p.width = s["width"].as_u64().unwrap_or(0) as u32;
                p.height = s["height"].as_u64().unwrap_or(0) as u32;
                if let Some((n, d)) = s["avg_frame_rate"].as_str().and_then(|r| r.split_once('/')) {
                    let (n, d): (f64, f64) = (n.parse().unwrap_or(0.0), d.parse().unwrap_or(1.0));
                    if d > 0.0 {
                        p.fps = n / d;
                    }
                }
            }
            Some("audio") => p.has_audio = true,
            _ => {}
        }
    }
    Some(p)
}

/// Grab a single frame as RGBA, letterboxed into exactly w×h.
pub fn grab_frame(ffmpeg: &Path, file: &Path, t: f64, w: u32, h: u32) -> Option<Vec<u8>> {
    let vf = format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease:flags=bilinear,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2"
    );
    let out = cmd(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{t:.3}"), "-i"])
        .arg(file)
        .args([
            "-frames:v", "1", "-an", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    (out.stdout.len() == (w * h * 4) as usize).then_some(out.stdout)
}

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resolution {
    Native,
    P1440,
    P1080,
    P720,
    P480,
}

impl Resolution {
    pub const ALL: [Resolution; 5] = [
        Resolution::Native,
        Resolution::P1440,
        Resolution::P1080,
        Resolution::P720,
        Resolution::P480,
    ];
    pub fn height(self) -> Option<u32> {
        match self {
            Resolution::Native => None,
            Resolution::P1440 => Some(1440),
            Resolution::P1080 => Some(1080),
            Resolution::P720 => Some(720),
            Resolution::P480 => Some(480),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Resolution::Native => "Native (no scaling)",
            Resolution::P1440 => "1440p",
            Resolution::P1080 => "1080p",
            Resolution::P720 => "720p",
            Resolution::P480 => "480p",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum EncoderChoice {
    Auto,
    Nvidia,
    Intel,
    Amd,
    Cpu,
}

impl EncoderChoice {
    pub const ALL: [EncoderChoice; 5] = [
        EncoderChoice::Auto,
        EncoderChoice::Nvidia,
        EncoderChoice::Intel,
        EncoderChoice::Amd,
        EncoderChoice::Cpu,
    ];
    pub fn ffmpeg_name(self) -> Option<&'static str> {
        match self {
            EncoderChoice::Auto => None,
            EncoderChoice::Nvidia => Some("h264_nvenc"),
            EncoderChoice::Intel => Some("h264_qsv"),
            EncoderChoice::Amd => Some("h264_amf"),
            EncoderChoice::Cpu => Some("libx264"),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            EncoderChoice::Auto => "Auto (best available)",
            EncoderChoice::Nvidia => "NVIDIA NVENC (GPU)",
            EncoderChoice::Intel => "Intel Quick Sync (GPU)",
            EncoderChoice::Amd => "AMD AMF (GPU)",
            EncoderChoice::Cpu => "CPU x264 (heavy)",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptureMethod {
    DesktopDuplication,
    Gdi,
}

impl CaptureMethod {
    pub fn label(self) -> &'static str {
        match self {
            CaptureMethod::DesktopDuplication => "Desktop Duplication (fast, recommended)",
            CaptureMethod::Gdi => "GDI (compatibility fallback)",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClipSound {
    Off,
    Chime,
    Custom,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    // General
    pub clips_dir: PathBuf,
    pub start_buffer_on_launch: bool,
    pub start_minimized: bool,
    pub ffmpeg_path: String,

    // Video
    pub buffer_seconds: u32,
    pub fps: u32,
    pub resolution: Resolution,
    pub bitrate_kbps: u32,
    pub encoder: EncoderChoice,
    pub detected_encoder: String,
    pub capture: CaptureMethod,
    pub monitor_index: u32,
    pub draw_mouse: bool,

    // Audio
    pub record_system_audio: bool,
    pub system_device: Option<String>,
    pub system_volume: f32,
    pub record_mic: bool,
    pub mic_device: Option<String>,
    pub mic_volume: f32,
    pub audio_bitrate_kbps: u32,

    // Hotkeys
    pub hotkey_clip: String,
    pub hotkey_toggle: String,

    // Feedback
    pub clip_sound: ClipSound,
    pub clip_sound_path: String,

    // Editor export
    pub export_bitrate_kbps: u32,
}

impl Default for Settings {
    fn default() -> Self {
        let videos = dirs::video_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        Settings {
            clips_dir: videos.join("LiteClip"),
            start_buffer_on_launch: true,
            start_minimized: false,
            ffmpeg_path: String::new(),
            buffer_seconds: 30,
            fps: 30,
            resolution: Resolution::P1080,
            bitrate_kbps: 8000,
            encoder: EncoderChoice::Auto,
            detected_encoder: String::new(),
            capture: CaptureMethod::DesktopDuplication,
            monitor_index: 0,
            draw_mouse: true,
            record_system_audio: true,
            system_device: None,
            system_volume: 1.0,
            record_mic: false,
            mic_device: None,
            mic_volume: 1.0,
            audio_bitrate_kbps: 160,
            hotkey_clip: "F8".into(),
            hotkey_toggle: "Alt+F8".into(),
            clip_sound: ClipSound::Chime,
            clip_sound_path: String::new(),
            export_bitrate_kbps: 10000,
        }
    }
}

/// Roaming config folder: %APPDATA%\LiteClip
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("LiteClip")
}

/// Local cache folder (replay buffer segments): %LOCALAPPDATA%\LiteClip
pub fn cache_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("LiteClip")
}

impl Settings {
    fn file() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        std::fs::create_dir_all(config_dir()).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(Self::file(), json).map_err(|e| e.to_string())
    }

    /// True when a change between `self` and `other` requires restarting capture.
    pub fn capture_differs(&self, other: &Settings) -> bool {
        self.buffer_seconds != other.buffer_seconds
            || self.fps != other.fps
            || self.resolution != other.resolution
            || self.bitrate_kbps != other.bitrate_kbps
            || self.encoder != other.encoder
            || self.capture != other.capture
            || self.monitor_index != other.monitor_index
            || self.draw_mouse != other.draw_mouse
            || self.record_system_audio != other.record_system_audio
            || self.system_device != other.system_device
            || self.system_volume != other.system_volume
            || self.record_mic != other.record_mic
            || self.mic_device != other.mic_device
            || self.mic_volume != other.mic_volume
            || self.audio_bitrate_kbps != other.audio_bitrate_kbps
            || self.ffmpeg_path != other.ffmpeg_path
    }
}

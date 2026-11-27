//! Small Windows helpers: monitor list, notification sound, recycle bin, explorer.

use crate::settings::{ClipSound, Settings};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows_sys::Win32::Media::Audio::{
    PlaySoundW, SND_ASYNC, SND_FILENAME, SND_MEMORY, SND_NODEFAULT,
};
use windows_sys::Win32::UI::Shell::{
    SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
    SHFILEOPSTRUCTW,
};

#[derive(Clone, Debug)]
pub struct Monitor {
    #[allow(dead_code)]
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub primary: bool,
}

impl Monitor {
    pub fn label(&self, idx: usize) -> String {
        format!(
            "Monitor {} — {}×{}{}",
            idx + 1,
            self.w,
            self.h,
            if self.primary { " (primary)" } else { "" }
        )
    }
}

unsafe extern "system" fn monitor_cb(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = &mut *(data as *mut Vec<Monitor>);
    let mut info: MONITORINFOEXW = std::mem::zeroed();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(h, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) != 0 {
        let r = info.monitorInfo.rcMonitor;
        let end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(32);
        list.push(Monitor {
            name: String::from_utf16_lossy(&info.szDevice[..end]),
            x: r.left,
            y: r.top,
            w: r.right - r.left,
            h: r.bottom - r.top,
            primary: info.monitorInfo.dwFlags & 1 != 0,
        });
    }
    1
}

pub fn monitors() -> Vec<Monitor> {
    let mut v: Vec<Monitor> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(monitor_cb),
            &mut v as *mut Vec<Monitor> as LPARAM,
        );
    }
    // Desktop Duplication enumerates the primary output first on most systems.
    v.sort_by_key(|m| !m.primary);
    v
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// A short two-tone chime rendered to an in-memory WAV.
fn chime() -> &'static [u8] {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    WAV.get_or_init(|| {
        let sr = 22_050u32;
        let mut pcm: Vec<i16> = Vec::new();
        for (freq, dur) in [(880.0f32, 0.07f32), (1318.5, 0.13)] {
            let n = (sr as f32 * dur) as usize;
            for i in 0..n {
                let t = i as f32 / sr as f32;
                let fade_in = (i as f32 / (sr as f32 * 0.004)).min(1.0);
                let fade_out = (1.0 - i as f32 / n as f32).powi(2);
                let s = (t * freq * std::f32::consts::TAU).sin() * fade_in * fade_out * 0.35;
                pcm.push((s * i16::MAX as f32) as i16);
            }
        }
        let data_len = (pcm.len() * 2) as u32;
        let mut w = Vec::with_capacity(44 + data_len as usize);
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes()); // PCM
        w.extend_from_slice(&1u16.to_le_bytes()); // mono
        w.extend_from_slice(&sr.to_le_bytes());
        w.extend_from_slice(&(sr * 2).to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        for s in pcm {
            w.extend_from_slice(&s.to_le_bytes());
        }
        w
    })
}

pub fn play_clip_sound(s: &Settings) {
    unsafe {
        match s.clip_sound {
            ClipSound::Off => {}
            ClipSound::Chime => {
                PlaySoundW(
                    chime().as_ptr() as *const u16,
                    std::ptr::null_mut(),
                    SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
                );
            }
            ClipSound::Custom => {
                let p = wide(std::ffi::OsStr::new(s.clip_sound_path.trim()));
                if PlaySoundW(
                    p.as_ptr(),
                    std::ptr::null_mut(),
                    SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
                ) == 0
                {
                    PlaySoundW(
                        chime().as_ptr() as *const u16,
                        std::ptr::null_mut(),
                        SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
                    );
                }
            }
        }
    }
}

/// Move a file to the Recycle Bin.
pub fn recycle(path: &Path) -> Result<(), String> {
    let mut from = wide(path.as_os_str());
    from.push(0); // double-null terminated list
    unsafe {
        let mut op: SHFILEOPSTRUCTW = std::mem::zeroed();
        op.wFunc = FO_DELETE as _;
        op.pFrom = from.as_ptr();
        op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as _;
        let r = SHFileOperationW(&mut op);
        if r == 0 {
            Ok(())
        } else {
            Err(format!("could not delete (code {r})"))
        }
    }
}

pub fn open_path(path: &Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

pub fn reveal_path(path: &Path) {
    let _ = std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn();
}

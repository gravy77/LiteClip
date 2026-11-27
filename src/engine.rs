//! Shared app core, used by the UI and by the global-hotkey / watchdog threads
//! (which keep working while the window is minimized).

use crate::audio::Levels;
use crate::ffmpeg::{self, Tools};
use crate::platform;
use crate::recorder::{self, Recorder};
use crate::settings::Settings;
use eframe::egui;
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const OFF: u8 = 0;
pub const STARTING: u8 = 1;
pub const ON: u8 = 2;

pub struct Notice {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub struct Engine {
    pub settings: Mutex<Settings>,
    pub tools: Mutex<Option<Tools>>,
    recorder: Mutex<Option<Recorder>>,
    pub state: AtomicU8,
    want_recording: AtomicBool,
    pub saving: AtomicBool,
    pub levels: Arc<Levels>,
    pub notices: Mutex<Vec<Notice>>,
    pub clips_changed: AtomicBool,
    pub hotkey_ids: Mutex<[Option<u32>; 2]>,
    pub rec_info: Mutex<Option<(Instant, String)>>,
    ctx: egui::Context,
}

impl Engine {
    pub fn new(settings: Settings, ctx: egui::Context) -> Arc<Self> {
        let tools = ffmpeg::locate(&settings.ffmpeg_path);
        Arc::new(Engine {
            settings: Mutex::new(settings),
            tools: Mutex::new(tools),
            recorder: Mutex::new(None),
            state: AtomicU8::new(OFF),
            want_recording: AtomicBool::new(false),
            saving: AtomicBool::new(false),
            levels: Arc::new(Levels::default()),
            notices: Mutex::new(Vec::new()),
            clips_changed: AtomicBool::new(false),
            hotkey_ids: Mutex::new([None, None]),
            rec_info: Mutex::new(None),
            ctx,
        })
    }

    pub fn notify(&self, text: impl Into<String>, error: bool) {
        let mut n = self.notices.lock().unwrap();
        n.push(Notice {
            text: text.into(),
            error,
            at: Instant::now(),
        });
        if n.len() > 6 {
            n.remove(0);
        }
        drop(n);
        self.ctx.request_repaint();
    }

    pub fn state(&self) -> u8 {
        self.state.load(Ordering::Relaxed)
    }

    pub fn tools(&self) -> Option<Tools> {
        self.tools.lock().unwrap().clone()
    }

    fn encoder_for(&self, s: &Settings, tools: &Tools) -> String {
        if let Some(n) = s.encoder.ffmpeg_name() {
            return n.to_string();
        }
        if !s.detected_encoder.is_empty() {
            return s.detected_encoder.clone();
        }
        let enc = ffmpeg::detect_encoder(&tools.ffmpeg);
        let mut g = self.settings.lock().unwrap();
        g.detected_encoder = enc.clone();
        let _ = g.save();
        enc
    }

    pub fn redetect_encoder(&self) -> Option<String> {
        let tools = self.tools()?;
        let enc = ffmpeg::detect_encoder(&tools.ffmpeg);
        let mut g = self.settings.lock().unwrap();
        g.detected_encoder = enc.clone();
        let _ = g.save();
        Some(enc)
    }

    pub fn start_buffer(&self) {
        if self
            .state
            .compare_exchange(OFF, STARTING, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        self.ctx.request_repaint();
        let s = self.settings.lock().unwrap().clone();
        let Some(tools) = self.tools() else {
            self.notify("FFmpeg not found. Set its location in Settings → General.", true);
            self.state.store(OFF, Ordering::SeqCst);
            return;
        };
        let enc = self.encoder_for(&s, &tools);
        let mut result = Recorder::start(&s, &tools, &enc, self.levels.clone());
        if result.is_err() && s.capture == crate::settings::CaptureMethod::DesktopDuplication {
            // Desktop Duplication can be denied (lock screen, some GPU drivers). Fall back to GDI.
            let mut gdi = s.clone();
            gdi.capture = crate::settings::CaptureMethod::Gdi;
            if let Ok(r) = Recorder::start(&gdi, &tools, &enc, self.levels.clone()) {
                self.notify("Desktop Duplication unavailable — using GDI capture for now.", true);
                result = Ok(r);
            }
        }
        match result {
            Ok(rec) => {
                for e in rec.audio_errors.lock().unwrap().iter() {
                    self.notify(format!("Audio problem: {e}"), true);
                }
                *self.rec_info.lock().unwrap() = Some((rec.started, rec.encoder.clone()));
                *self.recorder.lock().unwrap() = Some(rec);
                self.want_recording.store(true, Ordering::SeqCst);
                self.state.store(ON, Ordering::SeqCst);
                self.notify(format!("Replay buffer on — last {}s kept", s.buffer_seconds), false);
            }
            Err(e) => {
                self.state.store(OFF, Ordering::SeqCst);
                self.notify(e, true);
            }
        }
        self.ctx.request_repaint();
    }

    pub fn stop_buffer(&self) {
        self.want_recording.store(false, Ordering::SeqCst);
        let rec = self.recorder.lock().unwrap().take();
        if let Some(r) = rec {
            r.stop();
        }
        *self.rec_info.lock().unwrap() = None;
        self.state.store(OFF, Ordering::SeqCst);
        self.ctx.request_repaint();
    }

    pub fn toggle_buffer(&self) {
        match self.state() {
            ON => {
                self.stop_buffer();
                self.notify("Replay buffer off", false);
            }
            OFF => self.start_buffer(),
            _ => {}
        }
    }

    pub fn restart_buffer(&self) {
        if self.state() == ON {
            self.stop_buffer();
            self.start_buffer();
        }
    }

    pub fn save_clip(&self) {
        if self.state() != ON {
            self.notify("Replay buffer is off — start it first.", true);
            return;
        }
        if self.saving.swap(true, Ordering::SeqCst) {
            return;
        }
        self.ctx.request_repaint();
        let segs = {
            let g = self.recorder.lock().unwrap();
            g.as_ref().map(|r| (r.recent_segments(), r.buffer_seconds))
        };
        let s = self.settings.lock().unwrap().clone();
        let res = match (segs, self.tools()) {
            (Some((segs, buf)), Some(tools)) => recorder::write_clip(&tools, &segs, buf, &s.clips_dir),
            _ => Err("Replay buffer is not running.".into()),
        };
        self.saving.store(false, Ordering::SeqCst);
        match res {
            Ok(p) => {
                platform::play_clip_sound(&s);
                self.clips_changed.store(true, Ordering::SeqCst);
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.notify(format!("Clip saved: {name}"), false);
            }
            Err(e) => self.notify(e, true),
        }
    }

    /// Hotkey listener, watchdog, and optional auto-start.
    pub fn spawn_background(self: &Arc<Self>) {
        // Global hotkeys (events are delivered even when the window is minimized).
        let eng = self.clone();
        thread::spawn(move || {
            let rx = GlobalHotKeyEvent::receiver();
            while let Ok(ev) = rx.recv() {
                if ev.state != HotKeyState::Pressed {
                    continue;
                }
                let ids = *eng.hotkey_ids.lock().unwrap();
                let e = eng.clone();
                if Some(ev.id) == ids[0] {
                    thread::spawn(move || e.save_clip());
                } else if Some(ev.id) == ids[1] {
                    thread::spawn(move || e.toggle_buffer());
                }
            }
        });

        // Startup: detect encoder once, then auto-start the buffer.
        let eng = self.clone();
        thread::spawn(move || {
            let (auto, need_detect) = {
                let s = eng.settings.lock().unwrap();
                (s.start_buffer_on_launch, s.detected_encoder.is_empty())
            };
            if need_detect {
                eng.redetect_encoder();
            }
            if auto {
                eng.start_buffer();
            }
        });

        // Watchdog: restart capture if ffmpeg dies (e.g. display mode change, UAC prompt).
        let eng = self.clone();
        thread::spawn(move || {
            let mut failures = 0u32;
            let mut healthy_since = Instant::now();
            loop {
                thread::sleep(Duration::from_secs(2));
                if !eng.want_recording.load(Ordering::SeqCst) || eng.state() != ON {
                    continue;
                }
                let dead = {
                    let mut g = eng.recorder.lock().unwrap();
                    match g.as_mut() {
                        Some(r) => (!r.is_alive()).then(|| r.log_tail()),
                        None => None,
                    }
                };
                match dead {
                    None => {
                        if healthy_since.elapsed() > Duration::from_secs(60) {
                            failures = 0;
                        }
                    }
                    Some(log) => {
                        failures += 1;
                        eng.stop_buffer();
                        if failures <= 5 {
                            eng.notify(format!("Capture stopped, restarting… {log}"), true);
                            thread::sleep(Duration::from_secs(1));
                            eng.start_buffer();
                            healthy_since = Instant::now();
                        } else {
                            eng.notify(format!("Capture keeps failing, buffer turned off.\n{log}"), true);
                        }
                    }
                }
            }
        });
    }
}

//! The window: Clips library, Editor and Settings.

use crate::editor::{self, ExportJob, ExportParams, FrameGrabber, Player, ThumbWorker};
use crate::engine::{Engine, OFF, ON, STARTING};
use crate::ffmpeg::{self, Probe};
use crate::platform::{self, Monitor};
use crate::recorder;
use crate::settings::*;
use crate::audio;
use eframe::egui::{
    self, Align2, Color32, FontId, Key, RichText, Rounding, Sense, Stroke, TextureHandle,
    TextureOptions, Vec2,
};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const ACCENT: Color32 = Color32::from_rgb(64, 156, 255);
const REC_RED: Color32 = Color32::from_rgb(235, 64, 64);
const CUT_COLOR: Color32 = Color32::from_rgb(90, 40, 40);

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Clips,
    Editor,
    Settings,
}

struct ClipEntry {
    path: PathBuf,
    name: String,
    size: u64,
    modified: SystemTime,
}

struct EditorState {
    path: PathBuf,
    probe: Probe,
    splits: Vec<f64>,
    cut: Vec<bool>,
    playhead: f64,
    tex: Option<TextureHandle>,
    shown_id: u64,
    wanted_id: u64,
    last_grab: Instant,
    pending_grab: Option<f64>,
    player: Option<Player>,
    preview_audio: bool,
    speed: f64,
    volume: f32,
    mute: bool,
    out_res: Resolution,
    fps_cap: u32,
    fast: bool,
    out_name: String,
    job: Option<ExportJob>,
}

impl EditorState {
    fn bounds(&self) -> Vec<f64> {
        let mut b = vec![0.0];
        b.extend(self.splits.iter().copied());
        b.push(self.probe.duration);
        b
    }
    fn segment_at(&self, t: f64) -> usize {
        self.splits.iter().take_while(|&&s| s <= t).count()
    }
    fn kept(&self) -> Vec<(f64, f64)> {
        let b = self.bounds();
        let mut out: Vec<(f64, f64)> = Vec::new();
        for i in 0..self.cut.len() {
            if self.cut[i] {
                continue;
            }
            let (s, e) = (b[i], b[i + 1]);
            match out.last_mut() {
                Some(last) if (last.1 - s).abs() < 1e-6 => last.1 = e,
                _ => out.push((s, e)),
            }
        }
        out
    }
    fn split_at(&mut self, t: f64) {
        let d = self.probe.duration;
        if t < 0.1 || t > d - 0.1 || self.splits.iter().any(|s| (s - t).abs() < 0.1) {
            return;
        }
        let i = self.segment_at(t);
        self.splits.insert(i, t);
        let c = self.cut[i];
        self.cut.insert(i + 1, c);
    }
    fn simple_edit(&self) -> bool {
        (self.speed - 1.0).abs() < 1e-3
            && (self.volume - 1.0).abs() < 1e-3
            && !self.mute
            && self.out_res == Resolution::Native
            && self.fps_cap == 0
            && self.kept().len() == 1
    }
}

pub struct LiteApp {
    engine: Arc<Engine>,
    tab: Tab,
    draft: Settings,
    first_frame: bool,
    hk_manager: Option<GlobalHotKeyManager>,
    hk_registered: Vec<HotKey>,
    capturing: Option<usize>,
    hk_error: Option<String>,
    out_devs: Vec<String>,
    in_devs: Vec<String>,
    monitors: Vec<Monitor>,
    devices_loaded: bool,
    clips: Vec<ClipEntry>,
    filter: String,
    meta: HashMap<PathBuf, Probe>,
    thumbs: HashMap<PathBuf, TextureHandle>,
    requested: HashSet<PathBuf>,
    thumb_worker: Option<ThumbWorker>,
    grabber: Option<FrameGrabber>,
    rename: Option<(PathBuf, String)>,
    confirm_delete: Option<PathBuf>,
    editor: Option<EditorState>,
    ctx: egui::Context,
}

fn fmt_time(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0) as u64;
    let s = t - (m * 60) as f64;
    format!("{m}:{s:04.1}")
}

fn fmt_size(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.2} GB", b as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.1} MB", b as f64 / (1u64 << 20) as f64)
    }
}

fn setup_style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.selection.bg_fill = ACCENT.linear_multiply(0.6);
    v.hyperlink_color = ACCENT;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.panel_fill = Color32::from_rgb(24, 25, 29);
    v.window_fill = Color32::from_rgb(30, 31, 36);
    v.extreme_bg_color = Color32::from_rgb(16, 17, 20);
    ctx.set_visuals(v);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    ctx.set_style(style);
}

fn key_combo(mods: egui::Modifiers, key: Key) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if mods.ctrl {
        parts.push("Ctrl");
    }
    if mods.alt {
        parts.push("Alt");
    }
    if mods.shift {
        parts.push("Shift");
    }
    let name = key.name();
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(name);
    s
}

impl LiteApp {
    pub fn new(cc: &eframe::CreationContext, settings: Settings) -> Self {
        setup_style(&cc.egui_ctx);
        let engine = Engine::new(settings.clone(), cc.egui_ctx.clone());
        engine.spawn_background();
        let tools = engine.tools();
        let mut app = LiteApp {
            engine,
            tab: Tab::Clips,
            draft: settings,
            first_frame: true,
            hk_manager: GlobalHotKeyManager::new().ok(),
            hk_registered: Vec::new(),
            capturing: None,
            hk_error: None,
            out_devs: Vec::new(),
            in_devs: Vec::new(),
            monitors: platform::monitors(),
            devices_loaded: false,
            clips: Vec::new(),
            filter: String::new(),
            meta: HashMap::new(),
            thumbs: HashMap::new(),
            requested: HashSet::new(),
            thumb_worker: tools.clone().map(|t| ThumbWorker::new(t, cc.egui_ctx.clone())),
            grabber: tools.map(|t| FrameGrabber::new(t, cc.egui_ctx.clone())),
            rename: None,
            confirm_delete: None,
            editor: None,
            ctx: cc.egui_ctx.clone(),
        };
        if app.hk_manager.is_none() {
            app.engine.notify("Global hotkeys unavailable on this system.", true);
        }
        app.register_hotkeys();
        app.reload_clips();
        app
    }

    fn register_hotkeys(&mut self) {
        let Some(mgr) = &self.hk_manager else { return };
        for hk in self.hk_registered.drain(..) {
            let _ = mgr.unregister(hk);
        }
        let s = self.engine.settings.lock().unwrap().clone();
        let mut ids = [None, None];
        for (slot, text) in [(0usize, &s.hotkey_clip), (1, &s.hotkey_toggle)] {
            if text.trim().is_empty() {
                continue;
            }
            match HotKey::from_str(text) {
                Ok(hk) => match mgr.register(hk) {
                    Ok(()) => {
                        ids[slot] = Some(hk.id());
                        self.hk_registered.push(hk);
                    }
                    Err(e) => self.engine.notify(format!("Hotkey {text} is taken: {e}"), true),
                },
                Err(e) => self.engine.notify(format!("Bad hotkey {text}: {e}"), true),
            }
        }
        *self.engine.hotkey_ids.lock().unwrap() = ids;
    }

    fn reload_clips(&mut self) {
        let dir = self.engine.settings.lock().unwrap().clips_dir.clone();
        let mut v: Vec<ClipEntry> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        let name = p.file_name()?.to_string_lossy().into_owned();
                        let ext = p.extension()?.to_string_lossy().to_lowercase();
                        if name.starts_with('.') || !["mp4", "mkv", "mov"].contains(&ext.as_str()) {
                            return None;
                        }
                        let md = e.metadata().ok()?;
                        Some(ClipEntry {
                            path: p,
                            name,
                            size: md.len(),
                            modified: md.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by(|a, b| b.modified.cmp(&a.modified));
        self.clips = v;
    }

    fn poll_workers(&mut self, ctx: &egui::Context) {
        if let Some(w) = &self.thumb_worker {
            while let Ok((path, probe, img)) = w.rx.try_recv() {
                if let Some(p) = probe {
                    self.meta.insert(path.clone(), p);
                }
                if let Some(img) = img {
                    let tex = ctx.load_texture(path.to_string_lossy(), img, TextureOptions::LINEAR);
                    self.thumbs.insert(path, tex);
                }
            }
        }
        if let (Some(g), Some(ed)) = (&self.grabber, &mut self.editor) {
            while let Ok((id, img)) = g.rx.try_recv() {
                if id < ed.shown_id || ed.player.is_some() {
                    continue;
                }
                ed.shown_id = id;
                match &mut ed.tex {
                    Some(t) => t.set(img, TextureOptions::LINEAR),
                    None => ed.tex = Some(ctx.load_texture("preview", img, TextureOptions::LINEAR)),
                }
            }
        }
    }

    fn open_editor(&mut self, path: PathBuf) {
        let Some(tools) = self.engine.tools() else { return };
        let Some(probe) = self.meta.get(&path).cloned().or_else(|| ffmpeg::probe(&tools.ffprobe, &path)) else {
            self.engine.notify("Couldn't read that clip.", true);
            return;
        };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut ed = EditorState {
            path: path.clone(),
            probe,
            splits: Vec::new(),
            cut: vec![false],
            playhead: 0.0,
            tex: None,
            shown_id: 0,
            wanted_id: 0,
            last_grab: Instant::now() - Duration::from_secs(1),
            pending_grab: None,
            player: None,
            preview_audio: true,
            speed: 1.0,
            volume: 1.0,
            mute: false,
            out_res: Resolution::Native,
            fps_cap: 0,
            fast: false,
            out_name: format!("{stem} (edit)"),
            job: None,
        };
        if let Some(g) = &mut self.grabber {
            ed.wanted_id = g.request(path, 0.0);
        }
        self.editor = Some(ed);
        self.tab = Tab::Editor;
    }

    // ───────────────────────────── top bar ─────────────────────────────
    fn ui_top(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("LiteClip").size(20.0).strong().color(ACCENT));
            ui.separator();
            let state = self.engine.state();
            match state {
                ON => {
                    let (started, enc) = self
                        .engine
                        .rec_info
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or((Instant::now(), String::new()));
                    ui.label(RichText::new("● BUFFERING").color(REC_RED).strong());
                    ui.label(RichText::new(fmt_time(started.elapsed().as_secs_f64()).split('.').next().unwrap_or("").to_string()).monospace())
                        .on_hover_text(format!("Encoder: {enc}"));
                    if ui.button("Stop").clicked() {
                        let e = self.engine.clone();
                        thread::spawn(move || e.stop_buffer());
                    }
                }
                STARTING => {
                    ui.spinner();
                    ui.label("Starting…");
                }
                _ => {
                    ui.label(RichText::new("Buffer off").weak());
                    if ui.button("Start buffer").clicked() {
                        let e = self.engine.clone();
                        thread::spawn(move || e.start_buffer());
                    }
                }
            }
            let hk = self.engine.settings.lock().unwrap().hotkey_clip.clone();
            let saving = self.engine.saving.load(std::sync::atomic::Ordering::Relaxed);
            let btn = egui::Button::new(RichText::new(if saving { "Saving…" } else { "Save clip" }).strong())
                .fill(if state == ON { ACCENT.linear_multiply(0.45) } else { Color32::from_gray(50) });
            if ui
                .add_enabled(state == ON && !saving, btn)
                .on_hover_text(format!("Hotkey: {hk}"))
                .clicked()
            {
                let e = self.engine.clone();
                thread::spawn(move || e.save_clip());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.selectable_value(&mut self.tab, Tab::Settings, "Settings");
                ui.selectable_value(&mut self.tab, Tab::Editor, "Editor");
                ui.selectable_value(&mut self.tab, Tab::Clips, "Clips");
            });
        });
    }

    // ───────────────────────────── clips ─────────────────────────────
    fn ui_clips(&mut self, ui: &mut egui::Ui) {
        let dir = self.engine.settings.lock().unwrap().clips_dir.clone();
        ui.horizontal(|ui| {
            ui.heading("Your clips");
            let total: u64 = self.clips.iter().map(|c| c.size).sum();
            ui.label(RichText::new(format!("{} clips · {}", self.clips.len(), fmt_size(total))).weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Open folder").clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    platform::open_path(&dir);
                }
                if ui.button("Refresh").clicked() {
                    self.reload_clips();
                }
                ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Search…").desired_width(180.0));
            });
        });
        ui.separator();
        if self.clips.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let hk = self.engine.settings.lock().unwrap().hotkey_clip.clone();
                ui.label(RichText::new("No clips yet").size(18.0));
                ui.label(RichText::new(format!("Start the buffer, play something, then press {hk} to save the last moments.")).weak());
            });
            return;
        }
        let filter = self.filter.to_lowercase();
        let mut action: Option<(u8, PathBuf)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for c in self.clips.iter().filter(|c| filter.is_empty() || c.name.to_lowercase().contains(&filter)) {
                if !self.requested.contains(&c.path) {
                    if let Some(w) = &self.thumb_worker {
                        w.request(c.path.clone());
                        self.requested.insert(c.path.clone());
                    }
                }
                egui::Frame::group(ui.style())
                    .fill(Color32::from_rgb(32, 33, 39))
                    .rounding(Rounding::same(6.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let size = Vec2::new(editor::THUMB_W as f32, editor::THUMB_H as f32);
                            let resp = match self.thumbs.get(&c.path) {
                                Some(t) => ui.add(egui::Image::new((t.id(), size)).rounding(4.0).sense(Sense::click())),
                                None => {
                                    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
                                    ui.painter().rect_filled(r, 4.0, Color32::from_gray(20));
                                    ui.painter().text(r.center(), Align2::CENTER_CENTER, "…", FontId::proportional(16.0), Color32::GRAY);
                                    resp
                                }
                            };
                            if resp.on_hover_text("Open in editor").clicked() {
                                action = Some((0, c.path.clone()));
                            }
                            ui.vertical(|ui| {
                                if let Some((p, text)) = &mut self.rename {
                                    if *p == c.path {
                                        ui.horizontal(|ui| {
                                            let r = ui.text_edit_singleline(text);
                                            if ui.button("OK").clicked()
                                                || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)))
                                            {
                                                action = Some((5, c.path.clone()));
                                            }
                                            if ui.button("Cancel").clicked() {
                                                action = Some((6, c.path.clone()));
                                            }
                                        });
                                    } else {
                                        ui.label(RichText::new(&c.name).strong());
                                    }
                                } else {
                                    ui.label(RichText::new(&c.name).strong());
                                }
                                let meta = self.meta.get(&c.path);
                                let date: chrono::DateTime<chrono::Local> = c.modified.into();
                                let mut info = vec![];
                                if let Some(m) = meta {
                                    info.push(fmt_time(m.duration));
                                    info.push(format!("{}×{}", m.width, m.height));
                                    if m.fps > 0.0 {
                                        info.push(format!("{:.0} fps", m.fps));
                                    }
                                }
                                info.push(fmt_size(c.size));
                                info.push(date.format("%d %b %Y %H:%M").to_string());
                                ui.label(RichText::new(info.join("  ·  ")).weak());
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    if ui.button("Edit").clicked() {
                                        action = Some((0, c.path.clone()));
                                    }
                                    if ui.button("Play").clicked() {
                                        action = Some((1, c.path.clone()));
                                    }
                                    if ui.button("Show in folder").clicked() {
                                        action = Some((2, c.path.clone()));
                                    }
                                    if ui.button("Rename").clicked() {
                                        action = Some((3, c.path.clone()));
                                    }
                                    if ui.button(RichText::new("Delete").color(Color32::from_rgb(230, 110, 110))).clicked() {
                                        action = Some((4, c.path.clone()));
                                    }
                                });
                            });
                        });
                    });
            }
        });
        if let Some((a, p)) = action {
            match a {
                0 => self.open_editor(p),
                1 => platform::open_path(&p),
                2 => platform::reveal_path(&p),
                3 => {
                    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    self.rename = Some((p, stem));
                }
                4 => self.confirm_delete = Some(p),
                5 => {
                    if let Some((p, name)) = self.rename.take() {
                        let name = name.trim().replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_");
                        if !name.is_empty() {
                            let ext = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or("mp4".into());
                            let np = p.with_file_name(format!("{name}.{ext}"));
                            if np.exists() && np != p {
                                self.engine.notify("A clip with that name already exists.", true);
                            } else if let Err(e) = std::fs::rename(&p, &np) {
                                self.engine.notify(format!("Rename failed: {e}"), true);
                            } else {
                                if let Some(m) = self.meta.remove(&p) {
                                    self.meta.insert(np.clone(), m);
                                }
                                if let Some(t) = self.thumbs.remove(&p) {
                                    self.thumbs.insert(np.clone(), t);
                                }
                                self.requested.insert(np);
                                self.reload_clips();
                            }
                        }
                    }
                }
                _ => self.rename = None,
            }
        }
    }

    fn ui_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(p) = self.confirm_delete.clone() else { return };
        let mut close = false;
        egui::Window::new("Delete clip?")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                ui.label(RichText::new("It goes to the Recycle Bin.").weak());
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Delete").color(Color32::from_rgb(240, 120, 120))).clicked() {
                        if self.editor.as_ref().map(|e| e.path == p).unwrap_or(false) {
                            self.editor = None;
                        }
                        match platform::recycle(&p) {
                            Ok(()) => {
                                self.thumbs.remove(&p);
                                self.meta.remove(&p);
                                self.reload_clips();
                            }
                            Err(e) => self.engine.notify(e, true),
                        }
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.confirm_delete = None;
        }
    }

    // ───────────────────────────── editor ─────────────────────────────
    fn ui_editor(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(tools) = self.engine.tools() else {
            ui.label("FFmpeg not found — set it in Settings.");
            return;
        };
        if self.editor.is_none() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No clip open").size(18.0));
                ui.label(RichText::new("Pick a clip in the Clips tab and press Edit.").weak());
            });
            return;
        }
        let engine = self.engine.clone();
        let settings = engine.settings.lock().unwrap().clone();
        let mut export_done: Option<Result<PathBuf, String>> = None;
        let mut close = false;
        {
            let grabber = &mut self.grabber;
            let ed = self.editor.as_mut().unwrap();
            let dur = ed.probe.duration.max(0.01);

            // Playback tick
            let mut stop_play = false;
            if let Some(pl) = &ed.player {
                if let Some(img) = pl.latest_frame() {
                    match &mut ed.tex {
                        Some(t) => t.set(img, TextureOptions::LINEAR),
                        None => ed.tex = Some(ctx.load_texture("preview", img, TextureOptions::LINEAR)),
                    }
                }
                ed.playhead = pl.position().min(dur);
                if pl.finished.load(std::sync::atomic::Ordering::Relaxed) || ed.playhead >= dur - 0.02 {
                    stop_play = true;
                }
                ctx.request_repaint_after(Duration::from_millis(33));
            }
            let mut restart_at: Option<f64> = None;
            if ed.player.is_some() && !stop_play {
                let i = ed.segment_at(ed.playhead);
                if ed.cut[i] {
                    let b = ed.bounds();
                    let next = (i..ed.cut.len()).find(|&j| !ed.cut[j]);
                    match next {
                        Some(j) => restart_at = Some(b[j] + 0.01),
                        None => stop_play = true,
                    }
                }
            }
            if stop_play {
                ed.player = None;
            }
            if let Some(t) = restart_at {
                ed.playhead = t;
                ed.player = Player::start(&tools, ed.path.clone(), t, ed.preview_audio, ctx.clone());
            }

            ui.horizontal(|ui| {
                ui.heading(ed.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                ui.label(RichText::new(format!(
                    "{} · {}×{}",
                    fmt_time(ed.probe.duration),
                    ed.probe.width,
                    ed.probe.height
                ))
                .weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });

            // Preview
            let avail = ui.available_width();
            let w = avail.min((ui.available_height() - 230.0).max(160.0) * 16.0 / 9.0);
            let size = Vec2::new(w, w * 9.0 / 16.0);
            ui.vertical_centered(|ui| match &ed.tex {
                Some(t) => {
                    ui.add(egui::Image::new((t.id(), size)).rounding(4.0));
                }
                None => {
                    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                    ui.painter().rect_filled(r, 4.0, Color32::BLACK);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, "Loading…", FontId::proportional(14.0), Color32::GRAY);
                }
            });

            // Timeline
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 4.0, Color32::from_rgb(16, 17, 20));
            let x_of = |t: f64| rect.left() + (t / dur) as f32 * rect.width();
            let b = ed.bounds();
            for i in 0..ed.cut.len() {
                let r = egui::Rect::from_min_max(
                    egui::pos2(x_of(b[i]) + 1.0, rect.top() + 6.0),
                    egui::pos2(x_of(b[i + 1]) - 1.0, rect.bottom() - 6.0),
                );
                if ed.cut[i] {
                    painter.rect_filled(r, 3.0, CUT_COLOR);
                    painter.text(r.center(), Align2::CENTER_CENTER, "cut", FontId::proportional(11.0), Color32::from_rgb(220, 140, 140));
                } else {
                    painter.rect_filled(r, 3.0, ACCENT.linear_multiply(0.55));
                }
            }
            for &s in &ed.splits {
                let x = x_of(s);
                painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], Stroke::new(2.0_f32, Color32::from_rgb(255, 210, 80)));
            }
            let px = x_of(ed.playhead);
            painter.line_segment([egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())], Stroke::new(2.0_f32, Color32::WHITE));
            painter.circle_filled(egui::pos2(px, rect.top() + 3.0), 4.0, Color32::WHITE);
            if resp.clicked() || resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let t = (((pos.x - rect.left()) / rect.width()) as f64 * dur).clamp(0.0, dur);
                    ed.player = None;
                    ed.playhead = t;
                    ed.pending_grab = Some(t);
                }
            }

            // Throttled frame grabs while scrubbing
            if let Some(t) = ed.pending_grab {
                if ed.last_grab.elapsed() >= Duration::from_millis(120) {
                    if let Some(g) = grabber.as_mut() {
                        ed.wanted_id = g.request(ed.path.clone(), t);
                    }
                    ed.last_grab = Instant::now();
                    ed.pending_grab = None;
                } else {
                    ctx.request_repaint_after(Duration::from_millis(120));
                }
            }

            // Transport + edit controls
            let no_focus = ctx.memory(|m| m.focused().is_none());
            let (k_space, k_split, k_toggle, k_left, k_right) = ctx.input(|i| {
                (
                    no_focus && i.key_pressed(Key::Space),
                    no_focus && i.key_pressed(Key::S),
                    no_focus && i.key_pressed(Key::D),
                    no_focus && i.key_pressed(Key::ArrowLeft),
                    no_focus && i.key_pressed(Key::ArrowRight),
                )
            });
            let mut seek: Option<f64> = None;
            ui.horizontal(|ui| {
                let playing = ed.player.is_some();
                if ui.button(if playing { "Pause" } else { "Play" }).on_hover_text("Space").clicked() || k_space {
                    if playing {
                        ed.player = None;
                        ed.pending_grab = Some(ed.playhead);
                    } else {
                        let t = if ed.playhead >= dur - 0.05 { 0.0 } else { ed.playhead };
                        ed.player = Player::start(&tools, ed.path.clone(), t, ed.preview_audio, ctx.clone());
                    }
                }
                if ui.button("-1s").on_hover_text("Back 1s (Left arrow)").clicked() || k_left {
                    seek = Some(ed.playhead - 1.0);
                }
                if ui.button("+1s").on_hover_text("Forward 1s (Right arrow)").clicked() || k_right {
                    seek = Some(ed.playhead + 1.0);
                }
                ui.label(RichText::new(format!("{} / {}", fmt_time(ed.playhead), fmt_time(dur))).monospace());
                ui.checkbox(&mut ed.preview_audio, "Preview sound");
                ui.separator();
                if ui.button("Split here").on_hover_text("S").clicked() || k_split {
                    ed.split_at(ed.playhead);
                }
                let i = ed.segment_at(ed.playhead);
                let lbl = if ed.cut[i] { "Keep this part" } else { "Cut this part" };
                if ui.button(lbl).on_hover_text("D").clicked() || k_toggle {
                    ed.cut[i] = !ed.cut[i];
                    if ed.cut.iter().all(|&c| c) {
                        ed.cut[i] = false;
                    }
                }
                if ui.button("Trim start").on_hover_text("Cut everything before the playhead").clicked() {
                    ed.split_at(ed.playhead);
                    let i = ed.segment_at(ed.playhead + 0.001);
                    for j in 0..i {
                        ed.cut[j] = true;
                    }
                }
                if ui.button("Trim end").on_hover_text("Cut everything after the playhead").clicked() {
                    ed.split_at(ed.playhead);
                    let i = ed.segment_at(ed.playhead - 0.001);
                    for j in (i + 1)..ed.cut.len() {
                        ed.cut[j] = true;
                    }
                }
                if ui.button("Reset").clicked() {
                    ed.splits.clear();
                    ed.cut = vec![false];
                }
            });
            if let Some(t) = seek {
                let t = t.clamp(0.0, dur);
                let was_playing = ed.player.is_some();
                ed.player = None;
                ed.playhead = t;
                if was_playing {
                    ed.player = Player::start(&tools, ed.path.clone(), t, ed.preview_audio, ctx.clone());
                } else {
                    ed.pending_grab = Some(t);
                }
            }

            let kept = ed.kept();
            let kept_len: f64 = kept.iter().map(|(s, e)| e - s).sum();
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Export").strong());
                ui.label(RichText::new(format!(
                    "{} part(s) · output length {}",
                    kept.len(),
                    fmt_time(kept_len / ed.speed)
                ))
                .weak());
            });
            egui::Grid::new("export_grid").num_columns(4).spacing([18.0, 6.0]).show(ui, |ui| {
                ui.label("Speed");
                ui.add(egui::Slider::new(&mut ed.speed, 0.25..=4.0).logarithmic(true).suffix("×"));
                ui.label("Volume");
                ui.add_enabled(
                    !ed.mute && ed.probe.has_audio,
                    egui::Slider::new(&mut ed.volume, 0.0..=3.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
                ui.end_row();
                ui.label("Resolution");
                egui::ComboBox::from_id_salt("exp_res")
                    .selected_text(if ed.out_res == Resolution::Native { "Original" } else { ed.out_res.label() })
                    .show_ui(ui, |ui| {
                        for r in Resolution::ALL {
                            ui.selectable_value(&mut ed.out_res, r, if r == Resolution::Native { "Original" } else { r.label() });
                        }
                    });
                ui.label("Frame rate");
                egui::ComboBox::from_id_salt("exp_fps")
                    .selected_text(if ed.fps_cap == 0 { "Original".to_string() } else { format!("{} fps", ed.fps_cap) })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut ed.fps_cap, 0, "Original");
                        for f in [60, 30, 24] {
                            ui.selectable_value(&mut ed.fps_cap, f, format!("{f} fps"));
                        }
                    });
                ui.end_row();
                ui.label("Audio");
                ui.add_enabled(ed.probe.has_audio, egui::Checkbox::new(&mut ed.mute, "Mute"));
                ui.label("Mode");
                let simple = ed.simple_edit();
                if !simple {
                    ed.fast = false;
                }
                ui.add_enabled(simple, egui::Checkbox::new(&mut ed.fast, "Instant (no re-encode)"))
                    .on_hover_text("Only for a single trimmed part with no other changes. Cuts snap to keyframes (~2s).");
                ui.end_row();
                ui.label("File name");
                ui.add(egui::TextEdit::singleline(&mut ed.out_name).desired_width(260.0));
                ui.end_row();
            });

            ui.horizontal(|ui| {
                if let Some(job) = &ed.job {
                    let p = job.progress();
                    ui.add(egui::ProgressBar::new(p).desired_width(300.0).show_percentage());
                    if ui.button("Cancel").clicked() {
                        job.cancel();
                    }
                    if let Some(r) = job.result.lock().unwrap().take() {
                        export_done = Some(r);
                    }
                } else if ui
                    .add(egui::Button::new(RichText::new("Export clip").strong()).fill(ACCENT.linear_multiply(0.5)))
                    .clicked()
                {
                    let name = ed.out_name.trim().replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_");
                    let name = if name.is_empty() { "Edited clip".to_string() } else { name };
                    let out = recorder::unique_clip_path(&settings.clips_dir, &name);
                    let enc = settings
                        .encoder
                        .ffmpeg_name()
                        .map(String::from)
                        .unwrap_or_else(|| if settings.detected_encoder.is_empty() { "libx264".into() } else { settings.detected_encoder.clone() });
                    ed.player = None;
                    ed.job = Some(editor::export(
                        &tools,
                        ExportParams {
                            input: ed.path.clone(),
                            output: out,
                            keep: kept.clone(),
                            speed: ed.speed,
                            volume: ed.volume,
                            mute: ed.mute,
                            height: ed.out_res.height(),
                            fps: (ed.fps_cap > 0).then_some(ed.fps_cap),
                            encoder: enc,
                            bitrate_kbps: settings.export_bitrate_kbps,
                            has_audio: ed.probe.has_audio,
                            fast: ed.fast,
                        },
                        ctx.clone(),
                    ));
                }
                ui.label(RichText::new("Keys: Space play, S split, D cut/keep, Left/Right seek").weak().small());
            });
            if export_done.is_some() {
                ed.job = None;
            }
        }
        if close {
            self.editor = None;
        }
        match export_done {
            Some(Ok(p)) => {
                self.engine.notify(
                    format!("Exported: {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                    false,
                );
                self.reload_clips();
            }
            Some(Err(e)) => self.engine.notify(e, true),
            None => {}
        }
    }

    // ───────────────────────────── settings ─────────────────────────────
    fn ui_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.devices_loaded {
            self.out_devs = audio::output_devices();
            self.in_devs = audio::input_devices();
            self.monitors = platform::monitors();
            self.devices_loaded = true;
        }
        let saved = self.engine.settings.lock().unwrap().clone();
        let dirty = saved != self.draft;

        ui.horizontal(|ui| {
            ui.heading("Settings");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(dirty, egui::Button::new(RichText::new("Save & apply").strong()).fill(if dirty { ACCENT.linear_multiply(0.5) } else { Color32::from_gray(45) })).clicked() {
                    self.apply_settings(saved.clone());
                }
                if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
                    self.draft = saved.clone();
                }
                if ui.button("Defaults").clicked() {
                    let keep_enc = self.draft.detected_encoder.clone();
                    self.draft = Settings::default();
                    self.draft.detected_encoder = keep_enc;
                }
                if dirty {
                    ui.label(RichText::new("Unsaved changes").color(Color32::from_rgb(255, 200, 90)));
                }
            });
        });
        ui.separator();

        let recording = self.engine.state() == ON;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let d = &mut self.draft;
            let section = |ui: &mut egui::Ui, title: &str| {
                ui.add_space(6.0);
                ui.label(RichText::new(title).size(16.0).strong().color(ACCENT));
            };

            section(ui, "General");
            egui::Grid::new("g_general").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                ui.label("Clips folder");
                ui.horizontal(|ui| {
                    let mut s = d.clips_dir.to_string_lossy().into_owned();
                    if ui.add(egui::TextEdit::singleline(&mut s).desired_width(360.0)).changed() {
                        d.clips_dir = PathBuf::from(s);
                    }
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().set_directory(&d.clips_dir).pick_folder() {
                            d.clips_dir = p;
                        }
                    }
                });
                ui.end_row();
                ui.label("On launch");
                ui.vertical(|ui| {
                    ui.checkbox(&mut d.start_buffer_on_launch, "Start replay buffer automatically");
                    ui.checkbox(&mut d.start_minimized, "Start minimized");
                });
                ui.end_row();
                ui.label("FFmpeg");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.ffmpeg_path).hint_text("auto-detect").desired_width(360.0));
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("ffmpeg", &["exe"]).pick_file() {
                            d.ffmpeg_path = p.to_string_lossy().into_owned();
                        }
                    }
                });
                ui.end_row();
                ui.label("");
                match self.engine.tools() {
                    Some(t) => ui.label(RichText::new(format!("Using {}", t.ffmpeg.display())).weak().small()),
                    None => ui.label(RichText::new("FFmpeg not found! Install it: winget install Gyan.FFmpeg.Essentials").color(REC_RED)),
                };
                ui.end_row();
            });

            section(ui, "Replay buffer & video");
            egui::Grid::new("g_video").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                ui.label("Clip length");
                ui.add(egui::Slider::new(&mut d.buffer_seconds, 10..=300).suffix(" s"));
                ui.end_row();
                ui.label("Screen");
                let cur = self
                    .monitors
                    .get(d.monitor_index as usize)
                    .map(|m| m.label(d.monitor_index as usize))
                    .unwrap_or(format!("Monitor {}", d.monitor_index + 1));
                egui::ComboBox::from_id_salt("monitor").selected_text(cur).width(300.0).show_ui(ui, |ui| {
                    for (i, m) in self.monitors.iter().enumerate() {
                        ui.selectable_value(&mut d.monitor_index, i as u32, m.label(i));
                    }
                });
                ui.end_row();
                ui.label("Capture method");
                egui::ComboBox::from_id_salt("capture").selected_text(d.capture.label()).width(300.0).show_ui(ui, |ui| {
                    for c in [CaptureMethod::DesktopDuplication, CaptureMethod::Gdi] {
                        ui.selectable_value(&mut d.capture, c, c.label());
                    }
                });
                ui.end_row();
                ui.label("Resolution");
                egui::ComboBox::from_id_salt("res").selected_text(d.resolution.label()).show_ui(ui, |ui| {
                    for r in Resolution::ALL {
                        ui.selectable_value(&mut d.resolution, r, r.label());
                    }
                });
                ui.end_row();
                ui.label("Frame rate");
                egui::ComboBox::from_id_salt("fps").selected_text(format!("{} fps", d.fps)).show_ui(ui, |ui| {
                    for f in [24, 30, 45, 60, 120] {
                        ui.selectable_value(&mut d.fps, f, format!("{f} fps"));
                    }
                });
                ui.end_row();
                ui.label("Video bitrate");
                ui.add(egui::Slider::new(&mut d.bitrate_kbps, 1000..=50000).logarithmic(true).suffix(" kbps"));
                ui.end_row();
                ui.label("Encoder");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("enc").selected_text(d.encoder.label()).width(220.0).show_ui(ui, |ui| {
                        for e in EncoderChoice::ALL {
                            ui.selectable_value(&mut d.encoder, e, e.label());
                        }
                    });
                    let det = self.engine.settings.lock().unwrap().detected_encoder.clone();
                    ui.label(RichText::new(if det.is_empty() { "detecting…".to_string() } else { format!("auto = {det}") }).weak());
                    if ui.small_button("Re-detect").clicked() {
                        let e = self.engine.clone();
                        thread::spawn(move || {
                            if let Some(enc) = e.redetect_encoder() {
                                e.notify(format!("Best encoder: {enc}"), false);
                            }
                        });
                    }
                });
                ui.end_row();
                ui.label("Mouse cursor");
                ui.checkbox(&mut d.draw_mouse, "Show cursor in clips");
                ui.end_row();
                ui.label("Estimated clip size");
                let mb = (d.bitrate_kbps + d.audio_bitrate_kbps) as f64 * d.buffer_seconds as f64 / 8.0 / 1024.0;
                ui.label(RichText::new(format!("≈ {mb:.0} MB per clip")).weak());
                ui.end_row();
            });

            section(ui, "Audio");
            let levels = self.engine.levels.clone();
            let meter = |ui: &mut egui::Ui, v: f32| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(140.0, 8.0), Sense::hover());
                ui.painter().rect_filled(r, 3.0, Color32::from_gray(35));
                let mut f = r;
                f.set_width(r.width() * v.clamp(0.0, 1.0).sqrt());
                let c = if v > 0.9 { REC_RED } else { Color32::from_rgb(80, 200, 120) };
                ui.painter().rect_filled(f, 3.0, c);
            };
            let device_combo = |ui: &mut egui::Ui, id: &str, sel: &mut Option<String>, list: &[String]| {
                egui::ComboBox::from_id_salt(id)
                    .selected_text(sel.clone().unwrap_or("System default".into()))
                    .width(300.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(sel, None, "System default");
                        for n in list {
                            ui.selectable_value(sel, Some(n.clone()), n);
                        }
                    });
            };
            egui::Grid::new("g_audio").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                ui.label("Game / system sound");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut d.record_system_audio, "Record");
                    if recording && saved.record_system_audio {
                        meter(ui, levels.system());
                    }
                });
                ui.end_row();
                ui.label("   Output device");
                ui.add_enabled_ui(d.record_system_audio, |ui| device_combo(ui, "sysdev", &mut d.system_device, &self.out_devs));
                ui.end_row();
                ui.label("   Volume");
                ui.add_enabled(d.record_system_audio, egui::Slider::new(&mut d.system_volume, 0.0..=2.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                ui.end_row();
                ui.label("Microphone");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut d.record_mic, "Record");
                    if recording && saved.record_mic {
                        meter(ui, levels.mic());
                    }
                });
                ui.end_row();
                ui.label("   Input device");
                ui.add_enabled_ui(d.record_mic, |ui| device_combo(ui, "micdev", &mut d.mic_device, &self.in_devs));
                ui.end_row();
                ui.label("   Volume");
                ui.add_enabled(d.record_mic, egui::Slider::new(&mut d.mic_volume, 0.0..=3.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                ui.end_row();
                ui.label("Audio quality");
                egui::ComboBox::from_id_salt("abr").selected_text(format!("{} kbps", d.audio_bitrate_kbps)).show_ui(ui, |ui| {
                    for b in [96, 128, 160, 192, 256, 320] {
                        ui.selectable_value(&mut d.audio_bitrate_kbps, b, format!("{b} kbps"));
                    }
                });
                ui.end_row();
                ui.label("");
                if ui.small_button("Refresh device list").clicked() {
                    self.devices_loaded = false;
                }
                ui.end_row();
            });
            if recording && (saved.record_mic || saved.record_system_audio) {
                ctx.request_repaint_after(Duration::from_millis(80));
            }

            section(ui, "Hotkeys");
            egui::Grid::new("g_hk").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                for (slot, label) in [(0usize, "Save clip"), (1, "Start / stop buffer")] {
                    ui.label(label);
                    ui.horizontal(|ui| {
                        let val = if slot == 0 { &mut d.hotkey_clip } else { &mut d.hotkey_toggle };
                        let text = if self.capturing == Some(slot) {
                            "Press a key… (Esc = cancel)".to_string()
                        } else if val.is_empty() {
                            "Not set".into()
                        } else {
                            val.clone()
                        };
                        if ui.add(egui::Button::new(RichText::new(text).monospace()).min_size(Vec2::new(220.0, 0.0))).clicked() {
                            self.capturing = Some(slot);
                            self.hk_error = None;
                        }
                        if ui.small_button("Clear").clicked() {
                            val.clear();
                        }
                    });
                    ui.end_row();
                }
            });
            if let Some(e) = &self.hk_error {
                ui.label(RichText::new(e).color(REC_RED));
            }
            ui.label(RichText::new("Hotkeys work everywhere, even in games, while LiteClip runs.").weak().small());

            section(ui, "Clip saved sound");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut d.clip_sound, ClipSound::Off, "Off");
                ui.selectable_value(&mut d.clip_sound, ClipSound::Chime, "Chime");
                ui.selectable_value(&mut d.clip_sound, ClipSound::Custom, "Custom .wav");
                if ui.button("Test").clicked() {
                    platform::play_clip_sound(d);
                }
            });
            if d.clip_sound == ClipSound::Custom {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.clip_sound_path).hint_text("C:\\sounds\\ding.wav").desired_width(360.0));
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("WAV", &["wav"]).pick_file() {
                            d.clip_sound_path = p.to_string_lossy().into_owned();
                        }
                    }
                });
            }

            section(ui, "Editor export");
            ui.horizontal(|ui| {
                ui.label("Export bitrate");
                ui.add(egui::Slider::new(&mut d.export_bitrate_kbps, 1000..=60000).logarithmic(true).suffix(" kbps"));
            });
            ui.add_space(20.0);
            ui.label(RichText::new(format!("Settings file: {}", config_dir().join("settings.json").display())).weak().small());
        });
    }

    fn apply_settings(&mut self, old: Settings) {
        let mut new = self.draft.clone();
        new.detected_encoder = old.detected_encoder.clone();
        if new.ffmpeg_path != old.ffmpeg_path {
            new.detected_encoder.clear();
            *self.engine.tools.lock().unwrap() = ffmpeg::locate(&new.ffmpeg_path);
            if let Some(t) = self.engine.tools() {
                self.thumb_worker = Some(ThumbWorker::new(t.clone(), self.engine_ctx()));
                self.grabber = Some(FrameGrabber::new(t, self.engine_ctx()));
                self.requested.clear();
            }
        }
        if let Err(e) = new.save() {
            self.engine.notify(format!("Couldn't save settings: {e}"), true);
        }
        let restart = new.capture_differs(&old);
        let dir_changed = new.clips_dir != old.clips_dir;
        *self.engine.settings.lock().unwrap() = new.clone();
        self.draft = new;
        self.register_hotkeys();
        if dir_changed {
            self.reload_clips();
        }
        if restart && self.engine.state() == ON {
            let e = self.engine.clone();
            thread::spawn(move || e.restart_buffer());
            self.engine.notify("Settings saved — restarting buffer", false);
        } else {
            self.engine.notify("Settings saved", false);
        }
    }

    fn engine_ctx(&self) -> egui::Context {
        self.ctx.clone()
    }

    fn ui_toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut n = self.engine.notices.lock().unwrap();
        n.retain(|x| now.duration_since(x.at) < Duration::from_secs(if x.error { 8 } else { 4 }));
        if n.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, [-14.0, -14.0])
            .show(ctx, |ui| {
                for x in n.iter() {
                    egui::Frame::popup(ui.style())
                        .fill(if x.error { Color32::from_rgb(70, 30, 32) } else { Color32::from_rgb(30, 45, 62) })
                        .show(ui, |ui| {
                            ui.set_max_width(380.0);
                            ui.label(RichText::new(&x.text).color(Color32::WHITE));
                        });
                }
            });
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn handle_hotkey_capture(&mut self, ctx: &egui::Context) {
        let Some(slot) = self.capturing else { return };
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                if key == Key::Escape {
                    self.capturing = None;
                    return;
                }
                let combo = key_combo(modifiers, key);
                match HotKey::from_str(&combo) {
                    Ok(_) => {
                        if slot == 0 {
                            self.draft.hotkey_clip = combo;
                        } else {
                            self.draft.hotkey_toggle = combo;
                        }
                    }
                    Err(_) => self.hk_error = Some(format!("“{combo}” can't be used as a hotkey")),
                }
                self.capturing = None;
                return;
            }
        }
    }
}

impl eframe::App for LiteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
            if self.draft.start_minimized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
        self.poll_workers(ctx);
        if self.engine.clips_changed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.reload_clips();
        }
        self.handle_hotkey_capture(ctx);

        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::default().fill(Color32::from_rgb(18, 19, 23)).inner_margin(egui::Margin::symmetric(12.0, 8.0)))
            .show(ctx, |ui| self.ui_top(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(ctx.style().visuals.panel_fill).inner_margin(egui::Margin::same(14.0)))
            .show(ctx, |ui| match self.tab {
                Tab::Clips => self.ui_clips(ui),
                Tab::Editor => self.ui_editor(ui, ctx),
                Tab::Settings => self.ui_settings(ui, ctx),
            });
        self.ui_delete_dialog(ctx);
        self.ui_toasts(ctx);

        if self.engine.state() == ON || self.engine.state() == STARTING {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        let _ = OFF;
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(ed) = &mut self.editor {
            ed.player = None;
            if let Some(j) = &ed.job {
                j.cancel();
            }
        }
        self.engine.stop_buffer();
    }
}

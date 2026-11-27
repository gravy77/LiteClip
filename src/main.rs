#![windows_subsystem = "windows"]

mod app;
mod audio;
mod editor;
mod engine;
mod ffmpeg;
mod platform;
mod recorder;
mod settings;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let settings = settings::Settings::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LiteClip")
            .with_inner_size([1040.0, 720.0])
            .with_min_inner_size([780.0, 520.0]),
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "LiteClip",
        options,
        Box::new(move |cc| Ok(Box::new(app::LiteApp::new(cc, settings)))),
    )
}

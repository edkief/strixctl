mod app;
mod backend;
mod profiles;
mod state;
mod watcher;
mod widgets;

use std::sync::mpsc;

use app::App;
use state::AppState;

fn main() -> eframe::Result<()> {
    let (tx, rx) = mpsc::channel();
    watcher::spawn_watcher(tx);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("strixctl")
            .with_inner_size([640.0, 560.0])
            .with_min_inner_size([480.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "strixctl",
        options,
        Box::new(|_cc| Ok(Box::new(App::new(AppState::default(), rx)))),
    )
}

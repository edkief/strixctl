mod app;
mod backend;
mod profiles;
mod state;
mod theme;
mod views;
mod watcher;
mod widgets;

use app::App;

fn main() -> iced::Result {
    iced::application("strixctl", App::update, App::view)
        .theme(App::theme)
        .subscription(App::subscription)
        .window_size((1100.0, 760.0))
        .run_with(App::new)
}

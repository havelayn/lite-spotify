// Release builds on Windows are GUI apps: no console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod app;
mod auth;
mod config;
mod player;
mod theme;
mod widgets;

use eframe::egui;

fn main() -> eframe::Result {
    init_logging();

    // Two worker threads are plenty for a handful of network requests and keep memory low.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("lite-spotify-worker")
        .enable_all()
        .build()
        .expect("could not start the async runtime");

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("lite-spotify")
            .with_inner_size([1000.0, 680.0])
            .with_min_inner_size([680.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "lite-spotify",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, rt)))),
    )
}

/// Logs go to `lite-spotify.log` in the settings folder (there is no console
/// on Windows). Set the `RUST_LOG` environment variable for more detail.
fn init_logging() {
    let mut builder = env_logger::Builder::new();
    builder.parse_filters("info,librespot_audio=warn");
    if let Ok(filters) = std::env::var("RUST_LOG") {
        builder.parse_filters(&filters);
    }
    let _ = std::fs::create_dir_all(config::app_dir());
    if let Ok(file) = std::fs::File::create(config::log_file()) {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    builder.init();

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("Crash: {info}");
        default_hook(info);
    }));
    log::info!("lite-spotify {} starting", env!("CARGO_PKG_VERSION"));
}

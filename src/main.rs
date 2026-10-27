// Release builds on Windows are GUI apps: no console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod app;
mod auth;
mod autostart;
mod config;
mod discord;
mod files;
mod fun;
mod media;
mod player;
mod session;
mod stats;
mod theme;
mod thumbs;
mod tray;
mod update;
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

    // Open where the window was last time, at the size it had.
    let saved = config::Config::load().window;
    let (size, position) = match saved {
        Some([x, y, w, h]) if (-50.0..8000.0).contains(&x) && (-50.0..8000.0).contains(&y) => {
            ([w.max(860.0), h.max(540.0)], Some([x, y]))
        }
        _ => ([1200.0, 760.0], None),
    };
    let mut viewport = egui::ViewportBuilder::default();
    if let Some(position) = position {
        viewport = viewport.with_position(position);
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: viewport
            .with_title("lite-spotify")
            .with_inner_size(size)
            .with_min_inner_size([860.0, 540.0])
            .with_icon(std::sync::Arc::new(egui::IconData {
                rgba: tray::icon_rgba(64, tray::AMBER),
                width: 64,
                height: 64,
            })),
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
    let logger = builder.build();
    log::set_max_level(logger.filter());
    let _ = log::set_boxed_logger(Box::new(player::KeyWatch(logger)));

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("Crash: {info}");
        default_hook(info);
    }));
    log::info!("lite-spotify {} starting", env!("CARGO_PKG_VERSION"));
}

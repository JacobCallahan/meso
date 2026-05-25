/*
 * Meso: Rust/GTK4 Linux desktop weather application.
 *
 * Entry point: initializes tracing, creates the GTK4 application,
 * builds the main window, and runs the event loop.
 */

mod alerts_pane;
mod app;
mod config;
mod forecast_pane;
mod location_panel;
mod models_pane;
mod national_pane;
mod soundings_pane;
mod observations_pane;
mod panel;
mod radar_overlay_dialog;
mod radar_pane;
mod runtime;
mod satellite_pane;
mod settings_panel;
mod spc_pane;
mod text_pane;
mod ui;
mod updraft_settings;

use app::WxApplication;

fn main() {
    // Initialize structured logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("meso=info".parse().unwrap()),
        )
        .init();

    tracing::info!("meso starting");

    let app = WxApplication::new();
    let exit_code = app.run();
    std::process::exit(exit_code);
}

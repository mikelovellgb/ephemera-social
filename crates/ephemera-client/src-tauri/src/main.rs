//! Ephemera desktop entry point (Tauri 2.x).
//!
//! On desktop this binary is the main executable. On mobile the library
//! entry point (`start_app`) in `lib.rs` is used instead.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Initialize tracing for desktop (stdout).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Ephemera Tauri v{}", env!("CARGO_PKG_VERSION"));

    ephemera_tauri_lib::run();
}

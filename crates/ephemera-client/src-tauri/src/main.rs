//! Ephemera desktop entry point (Tauri 2.x).
//!
//! On desktop this binary is the main executable. On mobile the library
//! entry point (`start_app`) in `lib.rs` is used instead.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ephemera_tauri_lib::run();
}

//! FeroSNMP native Windows GUI.
//!
//! The window code (in `ui`) only exists on Windows. The modules it uses for
//! settings, searching and the details pane are plain Rust, so they are built
//! and unit tested on every platform. On Linux and macOS this binary only says
//! that it needs Windows, so `cargo build --workspace` still works there.

// Release builds on Windows are GUI-subsystem programs (no console window).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// Outside Windows, nothing calls the shared modules except their tests.
#![cfg_attr(not(windows), allow(dead_code))]

/// Version of the program, for the About box.
const VERSION: &str = env!("CARGO_PKG_VERSION");

mod diagnostics;
mod file_stamps;
mod find;
mod loading;
mod node_info;
mod rich_text;
mod settings;
#[cfg(test)]
mod test_support;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
fn main() {
    ui::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ferosnmp-gui only runs on Windows; use the `ferosnmp` CLI instead.");
    std::process::exit(1);
}

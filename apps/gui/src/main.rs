//! FeroSNMP native Windows GUI.
//!
//! A placeholder until milestone M3, when the Win32 main window arrives. The
//! GUI only targets Windows; on other platforms this binary just says so, so
//! that `cargo build --workspace` still works on Linux and macOS.

// Release builds on Windows are GUI-subsystem programs (no console window).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    // The Win32 message loop arrives in M3.
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ferosnmp-gui only runs on Windows; use the `ferosnmp` CLI instead.");
    std::process::exit(1);
}

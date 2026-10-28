//! Embeds the Windows application manifest into `ferosnmp-gui.exe`.
//!
//! The manifest tells Windows how to run the program:
//! - use version 6 of the common controls (the modern look of tree views,
//!   list views and buttons, instead of the Windows 95 look);
//! - per-monitor DPI awareness v2: draw sharply at each monitor's scaling
//!   instead of being stretched as a bitmap;
//! - UTF-8 as the code page, and paths longer than 260 characters;
//! - it supports Windows 10 and 11 (one id covers both).

use embed_manifest::manifest::{DpiAwareness, MaxVersionTested, SupportedOS};
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    // Build scripts run on the machine doing the build, so ask Cargo what the
    // program is being built *for*.
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let manifest = new_manifest("FeroSNMP.Gui")
            .dpi_awareness(DpiAwareness::PerMonitorV2Only)
            .supported_os(SupportedOS::Windows10..=SupportedOS::Windows10)
            .max_version_tested(MaxVersionTested::Windows11Version22H2);
        embed_manifest(manifest).expect("could not embed the application manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}

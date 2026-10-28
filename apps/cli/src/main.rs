//! `ferosnmp` command-line tool.
//!
//! Only prints its version for now; the `mib` and `lint` commands arrive in
//! milestone M2.

fn main() {
    println!(
        "ferosnmp {} (mib-core {})",
        env!("CARGO_PKG_VERSION"),
        mib_core::VERSION
    );
}

//! The MIB layer of FeroSNMP: load a library of MIB modules and answer
//! "what is this OID or name?". Implemented in milestone M1.

#![forbid(unsafe_code)]

/// Version of this crate, for "about" boxes and bug reports.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

//! Helpers shared by the unit tests.

use std::path::PathBuf;
use std::sync::OnceLock;

use mib_core::{MibLibrary, MibSet};

/// The committed IETF MIBs from `corpus/ietf`, loaded once for all tests.
pub fn ietf_set() -> MibSet {
    static SET: OnceLock<MibSet> = OnceLock::new();
    SET.get_or_init(|| {
        let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/ietf");
        MibLibrary::new()
            .with_system_folders(false)
            .with_folder(folder)
            .load()
            .expect("the committed corpus loads")
    })
    .clone()
}

/// FEROSNMP-TEST-MIB, the MIB of the test agent in snmp-core, which has
/// tables with every kind of INDEX and writable objects of every type.
pub fn test_mib() -> MibSet {
    static SET: OnceLock<MibSet> = OnceLock::new();
    SET.get_or_init(|| {
        let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/snmp-core/tests/agent/mibs");
        MibLibrary::new()
            .with_system_folders(false)
            .with_folder(folder)
            .with_strictness(mib_core::Strictness::Permissive)
            .load()
            .expect("the test MIB loads")
    })
    .clone()
}

/// The test MIBs of trap-rx, which define the notifications in the
/// datagrams [`recorded`] returns.
pub fn trap_mibs() -> MibSet {
    static SET: OnceLock<MibSet> = OnceLock::new();
    SET.get_or_init(|| {
        let folder =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/trap-rx/tests/mibs");
        MibLibrary::new()
            .with_system_folders(false)
            .with_folder(folder)
            .load()
            .expect("the trap test MIBs load")
    })
    .clone()
}

/// A datagram net-snmp sent, from `crates/snmp-core/tests/notifications`.
pub fn recorded(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/snmp-core/tests/notifications")
        .join(format!("{name}.hex"));
    let text = std::fs::read_to_string(path).expect("the recording exists");
    snmp_core::parse_hex(text.trim()).expect("the recording is hex")
}

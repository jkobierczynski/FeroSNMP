//! Value formatting with the MIB the test agent serves
//! (`tests/agent/mibs/FEROSNMP-TEST-MIB`): enum labels, display hints, units
//! and BITS names.

use std::path::Path;

use mib_core::{MibLibrary, Oid, Strictness};
use snmp_core::{Formatter, Value, Varbind};

fn varbind(oid: &str, value: Value) -> Varbind {
    Varbind {
        oid: oid.parse::<Oid>().unwrap(),
        value,
    }
}

#[test]
fn values_use_the_mib() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agent/mibs");
    let mibs = MibLibrary::new()
        .with_system_folders(false)
        .with_folder(folder)
        .with_strictness(Strictness::Permissive)
        .load()
        .unwrap();
    let mut formatter = Formatter::new(Some(&mibs));

    let base = "1.3.6.1.4.1.8072.9999.1.1";
    let cases = [
        (
            1,
            Value::OctetString(b"FeroSNMP test agent".to_vec()),
            "FEROSNMP-TEST-MIB::fstString.0 = STRING: \"FeroSNMP test agent\"",
        ),
        (
            2,
            Value::OctetString(vec![0x00, 0x01, 0x7f, 0x80, 0xff]),
            "FEROSNMP-TEST-MIB::fstBinary.0 = Hex-STRING: 00 01 7F 80 FF",
        ),
        (
            3,
            Value::Integer(3),
            "FEROSNMP-TEST-MIB::fstStatus.0 = INTEGER: testing(3)",
        ),
        (
            4,
            Value::Integer(-125),
            "FEROSNMP-TEST-MIB::fstTemperature.0 = INTEGER: -12.5 degrees Celsius",
        ),
        (
            5,
            Value::Counter64(u64::MAX),
            "FEROSNMP-TEST-MIB::fstOctets.0 = Counter64: 18446744073709551615 octets",
        ),
        (
            8,
            Value::ObjectIdentifier("1.3.6.1.4.1.8072.9999.1.2".parse().unwrap()),
            "FEROSNMP-TEST-MIB::fstPointer.0 = OID: FEROSNMP-TEST-MIB::fstTable",
        ),
        (
            9,
            Value::OctetString(vec![0x00, 0x1b, 0x21, 0x0a, 0xff, 0x09]),
            "FEROSNMP-TEST-MIB::fstMac.0 = STRING: \"00:1b:21:0a:ff:09\"",
        ),
        (
            10,
            Value::OctetString(vec![
                0x07, 0xea, 0x0a, 0x1d, 0x0d, 0x2d, 0x1e, 0x05, b'+', 1, 0,
            ]),
            "FEROSNMP-TEST-MIB::fstClock.0 = STRING: \"2026-10-29,13:45:30.5,+1:0\"",
        ),
        (
            13,
            Value::OctetString(vec![0xa0, 0x40]),
            "FEROSNMP-TEST-MIB::fstFlags.0 = BITS: A0 40 alpha(0) gamma(2) delta(9)",
        ),
    ];
    for (arc, value, expected) in cases {
        let line = formatter.line(&varbind(&format!("{base}.{arc}.0"), value));
        assert_eq!(line, expected);
    }

    // A table cell, and an OID no MIB defines.
    let cell = varbind("1.3.6.1.4.1.8072.9999.1.2.1.3.7", Value::Counter32(7000));
    assert_eq!(
        formatter.line(&cell),
        "FEROSNMP-TEST-MIB::fstCount.7 = Counter32: 7000"
    );
    let unknown = varbind("1.3.6.1.4.1.99999.1.0", Value::Integer(1));
    assert_eq!(
        formatter.line(&unknown),
        "SNMPv2-SMI::enterprises.99999.1.0 = INTEGER: 1"
    );

    // Numeric OIDs keep the MIB's help for values.
    let mut numeric = Formatter::new(Some(&mibs)).with_numeric_oids(true);
    let status = varbind(&format!("{base}.3.0"), Value::Integer(1));
    assert_eq!(
        numeric.line(&status),
        format!("{base}.3.0 = INTEGER: up(1)")
    );
}

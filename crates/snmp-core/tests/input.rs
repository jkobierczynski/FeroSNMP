//! Building SET values from text with the MIB the test agent serves
//! (`tests/agent/mibs/FEROSNMP-TEST-MIB`): enumerations, ranges, sizes,
//! display hints, BITS, and the checks before sending.

use std::path::Path;

use mib_core::{MibLibrary, MibSet, Oid, Strictness};
use snmp_core::{InputError, Value, ValueInput, ValueType};

fn mibs() -> MibSet {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agent/mibs");
    MibLibrary::new()
        .with_system_folders(false)
        .with_folder(folder)
        .with_strictness(Strictness::Permissive)
        .load()
        .unwrap()
}

fn input(mibs: &MibSet, object: &str) -> ValueInput {
    let oid: Oid = snmp_core::resolve_object(mibs, object).unwrap();
    ValueInput::new(Some(mibs), oid)
}

/// Build with checks; the value, or the error message.
fn build(mibs: &MibSet, object: &str, text: &str) -> Result<Value, String> {
    input(mibs, object)
        .build(text, None, true)
        .map(|varbind| varbind.value)
        .map_err(|error| error.to_string())
}

#[test]
fn values_come_from_the_mib_type() {
    let mibs = mibs();
    let ok = |object: &str, text: &str| build(&mibs, object, text).unwrap();
    assert_eq!(
        ok("fstContact.0", " Jurgen "),
        Value::OctetString(b" Jurgen ".to_vec())
    );
    assert_eq!(ok("fstContact.0", ""), Value::OctetString(vec![]));
    assert_eq!(ok("fstMode.0", "auto"), Value::Integer(3));
    assert_eq!(ok("fstMode.0", "On"), Value::Integer(2));
    assert_eq!(ok("fstMode.0", "off(1)"), Value::Integer(1));
    assert_eq!(ok("fstMode.0", "2"), Value::Integer(2));
    assert_eq!(ok("fstLimit.0", "100"), Value::Integer(100));
    assert_eq!(
        ok("fstGateway.0", "10.1.2.3"),
        Value::IpAddress([10, 1, 2, 3])
    );
    assert_eq!(
        ok("fstMacAddress.0", "00:1b:21:0a:ff:0a"),
        Value::OctetString(vec![0x00, 0x1b, 0x21, 0x0a, 0xff, 0x0a])
    );
    assert_eq!(
        ok("fstOptions.0", "alpha delta"),
        Value::OctetString(vec![0x80, 0x40])
    );
    assert_eq!(
        ok("fstOptions.0", "gamma,1"),
        Value::OctetString(vec![0x60])
    );
    assert_eq!(ok("fstOptions.0", ""), Value::OctetString(vec![]));
    assert_eq!(
        ok("fstTarget.0", "fstTable"),
        Value::ObjectIdentifier("1.3.6.1.4.1.8072.9999.1.2".parse().unwrap())
    );
    assert_eq!(
        ok("fstTarget.0", ".1.3.6.1"),
        Value::ObjectIdentifier("1.3.6.1".parse().unwrap())
    );
    assert_eq!(ok("fstThreshold.0", "1000"), Value::Gauge32(1000));
}

#[test]
fn the_mib_constraints_are_checked() {
    let mibs = mibs();
    let error = |object: &str, text: &str| build(&mibs, object, text).unwrap_err();
    assert_eq!(
        error("fstMode.0", "maybe"),
        "FEROSNMP-TEST-MIB::fstMode.0: \"maybe\" is not one of off(1), on(2), auto(3) (or a number)"
    );
    assert_eq!(
        error("fstMode.0", "7"),
        "FEROSNMP-TEST-MIB::fstMode.0: 7 is not one of off(1), on(2), auto(3)"
    );
    assert_eq!(
        error("fstLimit.0", "101"),
        "FEROSNMP-TEST-MIB::fstLimit.0: 101 is outside the allowed range 0..100"
    );
    assert_eq!(
        error("fstThreshold.0", "0"),
        "FEROSNMP-TEST-MIB::fstThreshold.0: 0 is outside the allowed range 1..1000"
    );
    assert_eq!(
        error("fstThreshold.0", "-1"),
        "FEROSNMP-TEST-MIB::fstThreshold.0: \"-1\" is not a number that fits in a Unsigned32"
    );
    assert_eq!(
        error("fstContact.0", &"x".repeat(33)),
        "FEROSNMP-TEST-MIB::fstContact.0: 33 bytes is not an allowed size (SIZE 0..32)"
    );
    assert_eq!(
        error("fstMacAddress.0", "00:1b"),
        "FEROSNMP-TEST-MIB::fstMacAddress.0: 2 bytes is not an allowed size (SIZE 6)"
    );
    assert!(error("fstMacAddress.0", "00:1b:zz").contains("does not fit the display hint"));
    assert_eq!(
        error("fstOptions.0", "epsilon"),
        "FEROSNMP-TEST-MIB::fstOptions.0: \"epsilon\" is not one of the bits alpha(0), \
         beta(1), gamma(2), delta(9)"
    );
    assert_eq!(
        error("fstOptions.0", "5"),
        "FEROSNMP-TEST-MIB::fstOptions.0: bit 5 is not one of the bits alpha(0), beta(1), \
         gamma(2), delta(9)"
    );
    assert!(error("fstGateway.0", "10.1.2").contains("is not an IPv4 address"));
    assert!(error("fstTarget.0", "noSuchName").contains("unknown object"));
}

#[test]
fn only_writable_instances_are_set() {
    let mibs = mibs();
    let error = |object: &str| build(&mibs, object, "1").unwrap_err();
    assert_eq!(
        error("fstStatus.0"),
        "FEROSNMP-TEST-MIB::fstStatus.0 is read-only in the MIB, so the agent will refuse to \
         change it"
    );
    assert_eq!(
        error("fstMode"),
        "FEROSNMP-TEST-MIB::fstMode is the object itself; SET needs an instance, such as \
         FEROSNMP-TEST-MIB::fstMode.0"
    );
    assert_eq!(
        error("fstPortTable"),
        "FEROSNMP-TEST-MIB::fstPortTable is a table, not an object that holds a value"
    );

    // Unchecked, the value is still read with the MIB type, but sent as is.
    let forced = input(&mibs, "fstMode.0").build("7", None, false).unwrap();
    assert_eq!(forced.value, Value::Integer(7));
    let forced = input(&mibs, "fstStatus.0")
        .build("up", None, false)
        .unwrap();
    assert_eq!(forced.value, Value::Integer(1));
    let error = input(&mibs, "fstMode.0")
        .build("x", None, false)
        .unwrap_err();
    assert!(!error.can_force());
}

#[test]
fn a_type_can_be_given() {
    let mibs = mibs();
    // Hex for a text object, still checked against the size.
    let contact = input(&mibs, "fstContact.0");
    assert_eq!(
        contact
            .build("48 69", Some(ValueType::Hex), true)
            .unwrap()
            .value,
        Value::OctetString(b"Hi".to_vec())
    );
    // The wrong type is caught before sending, unless unchecked.
    let error = contact
        .build("5", Some(ValueType::Integer), true)
        .unwrap_err();
    assert!(matches!(error, InputError::NotAllowed { .. }), "{error}");
    assert_eq!(
        error.to_string(),
        "FEROSNMP-TEST-MIB::fstContact.0: the MIB type is OCTET STRING, but the value is an \
         INTEGER"
    );
    assert_eq!(
        contact
            .build("5", Some(ValueType::Integer), false)
            .unwrap()
            .value,
        Value::Integer(5)
    );

    // Not in any loaded MIB: the type must be given.
    let unknown = input(&mibs, "1.3.6.1.4.1.99999.1.0");
    assert!(matches!(
        unknown.build("5", None, true),
        Err(InputError::UnknownType { .. })
    ));
    assert_eq!(
        unknown
            .build("5", Some(ValueType::Unsigned32), true)
            .unwrap()
            .value,
        Value::Gauge32(5)
    );
}

#[test]
fn display_hints_are_used_both_ways() {
    let mibs = mibs();
    // fstTemperature is Tenths (DISPLAY-HINT "d-1"): -12.5 is sent as -125.
    let temperature = input(&mibs, "fstTemperature.0");
    assert_eq!(
        temperature.parse("-12.5", None).unwrap(),
        Value::Integer(-125)
    );
    assert_eq!(
        temperature.edit_text(&Value::Integer(-125)),
        (ValueType::Integer, "-12.5".to_string())
    );

    // The current value, as text that reads back to the same value.
    let cases = [
        ("fstMode.0", Value::Integer(3), ValueType::Integer, "auto"),
        (
            "fstMacAddress.0",
            Value::OctetString(vec![0, 0x1b, 0x21, 0x0a, 0xff, 9]),
            ValueType::Text,
            "00:1b:21:0a:ff:09",
        ),
        (
            "fstOptions.0",
            Value::OctetString(vec![0xa0, 0x40]),
            ValueType::Bits,
            "alpha gamma delta",
        ),
        (
            "fstTarget.0",
            Value::ObjectIdentifier("1.3.6.1.4.1.8072.9999.1.2".parse().unwrap()),
            ValueType::ObjectIdentifier,
            "FEROSNMP-TEST-MIB::fstTable",
        ),
        (
            "fstContact.0",
            Value::OctetString(b"Jurgen".to_vec()),
            ValueType::Text,
            "Jurgen",
        ),
        (
            "fstContact.0",
            Value::OctetString(vec![0, 1, 0xff]),
            ValueType::Hex,
            "00 01 ff",
        ),
        (
            "fstGateway.0",
            Value::IpAddress([192, 0, 2, 254]),
            ValueType::IpAddress,
            "192.0.2.254",
        ),
    ];
    for (object, value, value_type, text) in cases {
        let input = input(&mibs, object);
        assert_eq!(
            input.edit_text(&value),
            (value_type, text.to_string()),
            "{object}"
        );
        assert_eq!(
            input.parse(text, Some(value_type)).unwrap(),
            value,
            "{object}"
        );
    }
}

#[test]
fn what_is_allowed_is_described() {
    let mibs = mibs();
    let describe = |object: &str| input(&mibs, object).type_text();
    assert_eq!(describe("fstMode.0"), "INTEGER (off(1), on(2), auto(3))");
    assert_eq!(describe("fstLimit.0"), "Integer32 (0..100)");
    assert_eq!(describe("fstContact.0"), "DisplayString (SIZE 0..32)");
    assert_eq!(describe("fstMacAddress.0"), "MacAddress (SIZE 6)");
    assert_eq!(
        describe("fstOptions.0"),
        "BITS (bits alpha(0), beta(1), gamma(2), delta(9))"
    );
    assert_eq!(describe("fstGateway.0"), "IpAddress");
}

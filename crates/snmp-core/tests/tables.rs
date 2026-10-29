//! Golden-file tests of the table view: walks stored in `tests/tables/`
//! (in the output format of net-snmp's `snmpwalk -On`) are split into rows
//! and columns with the INDEX codec, and compared with the snapshots in
//! `tests/snapshots/`.
//!
//! - `fstPortTable`: a three-part INDEX (integer, IpAddress, string with
//!   its length), from FEROSNMP-TEST-MIB.
//! - `fstPortXTable`: AUGMENTS fstPortEntry.
//! - `fstUserTable`: INDEX { IMPLIED fstUserName }.
//! - `fstPolicyTable`: an enumeration and an IMPLIED OBJECT IDENTIFIER.
//! - `ifTable` and `ifXTable` (AUGMENTS ifEntry) from IF-MIB, and
//!   `ipNetToMediaTable` from RFC1213-MIB. These MIBs may not be
//!   redistributed, so they come from `corpus/fetch.sh`; without them the
//!   tests are skipped (the "Full MIB corpus" CI job runs them).
//!
//! To refresh a walk from a real agent:
//! `snmpwalk -m '' -v2c -c public -On HOST 1.3.6.1.2.1.2.2 > tests/tables/ifTable.walk`.

use std::path::{Path, PathBuf};

use mib_core::{MibLibrary, MibSet, Oid, Strictness};
use snmp_core::{Formatter, TableView, Value, Varbind};

fn manifest_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn test_mib() -> MibSet {
    MibLibrary::new()
        .with_system_folders(false)
        .with_folder(manifest_dir().join("tests/agent/mibs"))
        .with_strictness(Strictness::Permissive)
        .load()
        .unwrap()
}

/// The IETF modules fetched by `corpus/fetch.sh`, or `None` when they have
/// not been fetched.
fn fetched(module: &str) -> Option<MibSet> {
    let folder = manifest_dir().join("../../corpus/fetched/ietf");
    if !folder.is_dir() {
        eprintln!("skipped: run corpus/fetch.sh to test tables from {module}");
        return None;
    }
    let set = MibLibrary::new()
        .with_system_folders(false)
        .with_folder(folder)
        .with_strictness(Strictness::Permissive)
        .load_modules(&[module])
        .unwrap();
    Some(set)
}

/// Read a walk in `snmpwalk -On` format.
fn read_walk(name: &str) -> Vec<Varbind> {
    let path = manifest_dir()
        .join("tests/tables")
        .join(format!("{name}.walk"));
    let text = std::fs::read_to_string(&path).unwrap();
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| parse_line(line).unwrap_or_else(|| panic!("{name}: cannot read {line:?}")))
        .collect()
}

fn parse_line(line: &str) -> Option<Varbind> {
    let (oid, value) = line.split_once(" = ")?;
    let oid: Oid = oid.parse().ok()?;
    let value = value.trim();
    let value = if value == "\"\"" {
        Value::OctetString(Vec::new())
    } else {
        let (kind, text) = value.split_once(": ")?;
        let text = text.trim();
        // `up(1)` with MIBs loaded, `1` without.
        let number = |text: &str| -> Option<i64> {
            let text = text
                .rsplit_once('(')
                .map_or(text, |(_, n)| n.trim_end_matches(')'));
            text.parse().ok()
        };
        match kind {
            "INTEGER" => Value::Integer(number(text)?),
            "STRING" => Value::OctetString(
                text.strip_prefix('"')
                    .and_then(|t| t.strip_suffix('"'))
                    .unwrap_or(text)
                    .as_bytes()
                    .to_vec(),
            ),
            "Hex-STRING" => Value::OctetString(
                text.split_whitespace()
                    .map(|byte| u8::from_str_radix(byte, 16).ok())
                    .collect::<Option<_>>()?,
            ),
            "Gauge32" => Value::Gauge32(text.parse().ok()?),
            "Counter32" => Value::Counter32(text.parse().ok()?),
            "Counter64" => Value::Counter64(text.parse().ok()?),
            "Timeticks" => {
                Value::TimeTicks(text.strip_prefix('(')?.split_once(')')?.0.parse().ok()?)
            }
            "IpAddress" => Value::IpAddress(text.parse::<std::net::Ipv4Addr>().ok()?.octets()),
            "OID" => Value::ObjectIdentifier(text.parse().ok()?),
            _ => return None,
        }
    };
    Some(Varbind { oid, value })
}

/// The table, as the CLI prints it.
fn render(mibs: &MibSet, table: &str, walks: &[&str]) -> String {
    let node = mibs
        .find(table)
        .unwrap_or_else(|| panic!("{table} is not loaded"));
    let table = mibs.table(node).unwrap();
    let header = format!(
        "{}::{} INDEX {{ {} }}{}\n",
        table.module.as_deref().unwrap_or("?"),
        table.name,
        table
            .index
            .iter()
            .map(|i| format!(
                "{}{} [{:?}]",
                if i.implied { "IMPLIED " } else { "" },
                i.name,
                i.encoding
            ))
            .collect::<Vec<_>>()
            .join(", "),
        table
            .augments
            .as_ref()
            .map(|a| format!(" (AUGMENTS {a})"))
            .unwrap_or_default(),
    );
    let mut view = TableView::new(table);
    for walk in walks {
        view.extend(read_walk(walk));
    }
    assert_eq!(view.others(), 0, "every result is a cell of the table");
    let mut formatter = Formatter::new(Some(mibs));
    format!("{header}{}", view.text(&mut formatter))
}

#[test]
fn multi_part_index() {
    let mibs = test_mib();
    insta::assert_snapshot!(
        "fstPortTable",
        render(&mibs, "fstPortTable", &["fstPortTable"])
    );
}

#[test]
fn augments() {
    let mibs = test_mib();
    insta::assert_snapshot!(
        "fstPortXTable",
        render(&mibs, "fstPortXTable", &["fstPortXTable"])
    );
}

#[test]
fn implied_string() {
    let mibs = test_mib();
    insta::assert_snapshot!(
        "fstUserTable",
        render(&mibs, "fstUserTable", &["fstUserTable"])
    );
}

#[test]
fn enum_and_implied_oid() {
    let mibs = test_mib();
    insta::assert_snapshot!(
        "fstPolicyTable",
        render(&mibs, "fstPolicyTable", &["fstPolicyTable"])
    );
}

#[test]
fn if_table() {
    let Some(mibs) = fetched("IF-MIB") else {
        return;
    };
    insta::assert_snapshot!("ifTable", render(&mibs, "IF-MIB::ifTable", &["ifTable"]));
    insta::assert_snapshot!("ifXTable", render(&mibs, "IF-MIB::ifXTable", &["ifXTable"]));
}

#[test]
fn ip_net_to_media_table() {
    let Some(mibs) = fetched("RFC1213-MIB") else {
        return;
    };
    insta::assert_snapshot!(
        "ipNetToMediaTable",
        render(
            &mibs,
            "RFC1213-MIB::ipNetToMediaTable",
            &["ipNetToMediaTable"]
        )
    );
}

#[test]
fn rows_arrive_in_any_order() {
    let mibs = test_mib();
    let node = mibs.find("fstPortTable").unwrap();
    let mut forward = TableView::new(mibs.table(node).unwrap());
    let mut backward = forward.clone();
    let walk = read_walk("fstPortTable");
    forward.extend(walk.iter().cloned());
    backward.extend(walk.into_iter().rev());
    assert_eq!(forward.rows(), backward.rows());
}

#[test]
fn a_suffix_that_does_not_fit_is_shown_raw() {
    let mibs = test_mib();
    let node = mibs.find("fstPortTable").unwrap();
    let mut view = TableView::new(mibs.table(node).unwrap());
    let speed = |suffix: &str| Varbind {
        oid: format!("1.3.6.1.4.1.8072.9999.1.4.1.5.{suffix}")
            .parse()
            .unwrap(),
        value: Value::Gauge32(1),
    };
    // A string length that runs past the end, and a byte above 255.
    view.add(speed("1.192.0.2.1.9.101"));
    view.add(speed("1.192.0.2.300.0"));
    // Not a cell of the table at all.
    assert!(!view.add(Varbind {
        oid: "1.3.6.1.4.1.8072.9999.1.4.1.99.1".parse().unwrap(),
        value: Value::Null,
    }));
    assert_eq!(view.others(), 1);
    let mut formatter = Formatter::new(Some(&mibs));
    insta::assert_snapshot!("bad_suffixes", view.text(&mut formatter));
}

#[test]
fn csv_output() {
    let mibs = test_mib();
    let node = mibs.find("fstPortXTable").unwrap();
    let mut view = TableView::new(mibs.table(node).unwrap());
    view.extend(read_walk("fstPortXTable"));
    let mut formatter = Formatter::new(Some(&mibs));
    assert_eq!(
        view.csv(&mut formatter).lines().collect::<Vec<_>>(),
        [
            "fstPortSlot,fstPortAddress,fstPortName,fstPortAlias,fstPortErrors",
            "1,192.0.2.1,eth0,uplink,0",
            "1,192.0.2.1,eth1,,17",
            "2,198.51.100.7,mgmt,management,3",
            "16,10.0.0.1,\"\"\"\"\"\",no name,0",
        ]
    );
}

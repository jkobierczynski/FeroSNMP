//! Tests of the public API against a small fixture module.

use std::path::PathBuf;

use mib_core::{
    Access, BaseType, IndexPart, MibLibrary, MibSet, NamedNumber, NodeKind, Oid, Range, RangeBound,
    SourceKind, Status, Strictness,
};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_fixture() -> MibSet {
    MibLibrary::new()
        .with_folder(fixtures())
        .with_system_folders(false)
        .load()
        .expect("fixture loads")
}

#[test]
fn mib_set_can_be_shared_between_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MibSet>();

    let set = load_fixture();
    let shared = set.clone();
    let name = std::thread::spawn(move || shared.resolve("testMode").map(|n| n.qualified_name()))
        .join()
        .unwrap();
    assert_eq!(name.as_deref(), Some("FEROSNMP-TEST-MIB::testMode"));
}

#[test]
fn foundation_modules_load_without_any_folder() {
    let set = MibLibrary::new().with_system_folders(false).load().unwrap();
    let smi = set.module("SNMPv2-SMI").expect("built-in module");
    assert!(smi.embedded);
    assert_eq!(
        set.resolve("enterprises").unwrap().oid().to_string(),
        "1.3.6.1.4.1"
    );
}

#[test]
fn index_finds_modules_by_name_inside_files() {
    let index = MibLibrary::new()
        .with_folder(fixtures())
        .with_system_folders(false)
        .index()
        .unwrap();
    assert_eq!(index.len(), 1);
    assert_eq!(index[0].module, "FEROSNMP-TEST-MIB");
    assert_eq!(index[0].file.file_name().unwrap(), "misnamed-test.mib");
    assert_eq!(index[0].source, SourceKind::UserFolder);
    assert!(!index[0].shadowed);
}

#[test]
fn missing_user_folder_is_an_error() {
    let result = MibLibrary::new()
        .with_folder(fixtures().join("does-not-exist"))
        .with_system_folders(false)
        .load();
    assert!(matches!(result, Err(mib_core::LoadError::Folder { .. })));
}

#[test]
fn lookup_by_name_qualified_name_and_oid() {
    let set = load_fixture();

    let by_name = set.find("testMode").unwrap();
    let by_qualified = set.find("FEROSNMP-TEST-MIB::testMode").unwrap();
    let oid: Oid = "1.3.6.1.4.1.99999.1.2".parse().unwrap();
    let by_oid = set.node_at(&oid).unwrap();
    assert_eq!(by_name, by_qualified);
    assert_eq!(by_name, by_oid);
    assert_eq!(by_name.oid(), oid);
    assert_eq!(set.node(by_name.id()), by_name);

    assert!(set.find("noSuchObject").is_none());
    assert!(set.find("NO-SUCH-MIB::testMode").is_none());
    assert!(
        set.node_at(&"1.3.6.1.4.1.99999.1.2.0".parse().unwrap())
            .is_none()
    );
}

#[test]
fn resolve_accepts_what_a_user_types() {
    let set = load_fixture();
    for query in [
        "testTemperature",
        "FEROSNMP-TEST-MIB::testTemperature",
        "1.3.6.1.4.1.99999.1.1",
        ".1.3.6.1.4.1.99999.1.1",
        "testTemperature.0",
        " testTemperature ",
    ] {
        let node = set.resolve(query).unwrap_or_else(|| panic!("{query:?}"));
        assert_eq!(node.name(), "testTemperature", "{query:?}");
    }
    assert!(set.resolve("").is_none());
    assert!(set.resolve("9.9.9").is_none());
}

#[test]
fn instance_lookup_splits_off_the_suffix() {
    let set = load_fixture();
    // testPortStatus for slot 3, port "ab" (IMPLIED: no length byte).
    let oid: Oid = "1.3.6.1.4.1.99999.1.3.1.3.3.97.98".parse().unwrap();
    let lookup = set.lookup(&oid);
    assert_eq!(lookup.node.name(), "testPortStatus");
    assert_eq!(lookup.suffix, vec![3, 97, 98]);
    assert_eq!(
        set.format_oid(&oid),
        "FEROSNMP-TEST-MIB::testPortStatus.3.97.98"
    );

    let outside = set.lookup(&"9.9".parse().unwrap());
    assert!(outside.node.oid().is_empty());
    assert_eq!(outside.suffix, vec![9, 9]);
}

#[test]
fn children_and_parent() {
    let set = load_fixture();
    let objects = set.find("testObjects").unwrap();
    let names: Vec<&str> = objects.children().map(|n| n.name()).collect();
    assert_eq!(
        names,
        [
            "testTemperature",
            "testMode",
            "testPortTable",
            "testPortStatsTable"
        ]
    );
    assert!(objects.has_children());
    assert_eq!(objects.parent().unwrap().name(), "ferosnmpTestMib");
    assert!(set.root().parent().is_none());
    let top: Vec<u32> = set.root().children().map(|n| n.arc()).collect();
    assert!(top.contains(&1), "iso(1) is below the root: {top:?}");
}

#[test]
fn scalar_details_follow_the_textual_convention() {
    let set = load_fixture();
    let details = set.find("testTemperature").unwrap().details();
    assert_eq!(details.kind, NodeKind::Scalar);
    assert_eq!(details.module.as_deref(), Some("FEROSNMP-TEST-MIB"));
    assert_eq!(details.access, Some(Access::ReadOnly));
    assert_eq!(details.status, Some(Status::Current));
    assert_eq!(details.units.as_deref(), Some("0.1 degrees Celsius"));
    assert_eq!(details.description.as_deref(), Some("Current temperature."));
    let syntax = details.syntax.unwrap();
    assert_eq!(
        syntax.type_chain,
        ["TestTemperature", "Integer32", "INTEGER"]
    );
    assert_eq!(syntax.base_type, BaseType::Integer32);
    assert_eq!(syntax.display_hint.as_deref(), Some("d-1"));
    assert_eq!(
        syntax.ranges,
        [Range {
            min: RangeBound::Signed(-400),
            max: RangeBound::Signed(1250)
        }]
    );
    assert!(details.index.is_empty());
}

#[test]
fn enumeration_details() {
    let set = load_fixture();
    let details = set.find("testMode").unwrap().details();
    assert_eq!(details.access, Some(Access::ReadWrite));
    let syntax = details.syntax.unwrap();
    assert_eq!(syntax.base_type, BaseType::Integer32);
    let labels: Vec<String> = syntax.enums.iter().map(NamedNumber::to_string).collect();
    assert_eq!(labels, ["off(0)", "standby(1)", "on(2)"]);
}

#[test]
fn table_details_show_the_index() {
    let set = load_fixture();
    let expected = [
        IndexPart {
            name: "testPortSlot".into(),
            implied: false,
        },
        IndexPart {
            name: "testPortName".into(),
            implied: true,
        },
    ];
    for (name, kind) in [
        ("testPortTable", NodeKind::Table),
        ("testPortEntry", NodeKind::Row),
        ("testPortStatus", NodeKind::Column),
    ] {
        let details = set.find(name).unwrap().details();
        assert_eq!(details.kind, kind, "{name}");
        assert_eq!(details.index, expected, "{name}");
    }

    let name = set.find("testPortName").unwrap().details().syntax.unwrap();
    assert_eq!(name.type_chain, ["DisplayString", "OCTET STRING"]);
    assert_eq!(
        name.sizes.iter().map(Range::to_string).collect::<Vec<_>>(),
        ["1..32"]
    );
}

#[test]
fn augmenting_row_uses_the_index_of_the_row_it_augments() {
    let set = load_fixture();
    let details = set.find("testPortStatsEntry").unwrap().details();
    assert_eq!(details.augments.as_deref(), Some("testPortEntry"));
    let names: Vec<&str> = details
        .index
        .iter()
        .map(|part| part.name.as_str())
        .collect();
    assert_eq!(names, ["testPortSlot", "testPortName"]);
}

#[test]
fn notification_details() {
    let set = load_fixture();
    let details = set.find("testOverheat").unwrap().details();
    assert_eq!(details.kind, NodeKind::Notification);
    assert!(details.syntax.is_none());
    assert!(details.access.is_none());
}

#[test]
fn broken_module_gives_diagnostics_with_positions() {
    let text = "BROKEN-MIB DEFINITIONS ::= BEGIN\n\
                IMPORTS enterprises FROM SNMPv2-SMI\n    \
                    missingThing FROM NO-SUCH-MIB;\n\
                brokenRoot OBJECT IDENTIFIER ::= { enterprises 99998 }\n\
                END\n";
    let set = mib_core::load_module_text("BROKEN-MIB", text, Strictness::Permissive).unwrap();
    let diagnostic = set
        .diagnostics()
        .iter()
        .find(|d| d.code == "import-module-not-found")
        .expect("unresolved import is reported");
    assert!(diagnostic.severity.is_error());
    assert_eq!(diagnostic.module.as_deref(), Some("BROKEN-MIB"));
    let location = diagnostic.location.as_ref().unwrap();
    assert_eq!((location.line, location.column), (3, 5));
    // The rest of the module still loads.
    assert!(set.find("brokenRoot").is_some());
}

#[test]
fn strict_mode_reports_more_than_permissive() {
    let load = |strictness| {
        MibLibrary::new()
            .with_folder(fixtures())
            .with_system_folders(false)
            .with_strictness(strictness)
            .load()
            .unwrap()
    };
    let permissive = load(Strictness::Permissive);
    let strict = load(Strictness::Strict);
    assert_eq!(strict.strictness(), Strictness::Strict);
    assert!(strict.diagnostics().len() >= permissive.diagnostics().len());
}

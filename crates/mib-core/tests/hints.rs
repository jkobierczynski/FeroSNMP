//! Every fix hint is triggered by a fixture module in `tests/hints/`. The
//! diagnostics and hints are compared with stored snapshots, and each kind of
//! hint the milestone promises is checked to be there.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use mib_core::{Diagnostic, MibLibrary, MibSet, Strictness};

fn hints_folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/hints")
}

fn load(strictness: Strictness) -> MibSet {
    MibLibrary::new()
        .with_system_folders(false)
        .with_folder(hints_folder())
        .with_strictness(strictness)
        .load()
        .expect("the hint fixtures load")
}

/// The diagnostics about the fixtures, one per line with the hint below,
/// with paths relative to `tests/hints` so the snapshot is the same on every
/// machine.
fn report(set: &MibSet) -> String {
    let root = hints_folder();
    let mut diagnostics: Vec<&Diagnostic> = set
        .diagnostics()
        .iter()
        .filter(|d| {
            d.location
                .as_ref()
                .is_some_and(|location| location.file.starts_with(&root))
        })
        .collect();
    diagnostics.sort_by_key(|d| {
        let location = d.location.as_ref().unwrap();
        (
            location.file.clone(),
            location.line,
            location.column,
            d.code.clone(),
        )
    });
    let mut out = String::new();
    for d in diagnostics {
        let location = d.location.as_ref().unwrap();
        let file = location.file.strip_prefix(&root).unwrap_or(Path::new(""));
        writeln!(
            out,
            "{}:{}:{}: {}: {} [{}]",
            file.display().to_string().replace('\\', "/"),
            location.line,
            location.column,
            d.severity,
            d.message,
            d.code
        )
        .unwrap();
        if let Some(symbol) = &d.symbol {
            writeln!(out, "    symbol: {symbol}").unwrap();
        }
        if let Some(hint) = &d.hint {
            // The folder list in "not in any MIB folder" hints is absolute.
            let hint = hint.replace(&root.display().to_string(), "<hints>");
            writeln!(out, "    hint: {}", hint.replace('\\', "/")).unwrap();
        }
    }
    out
}

#[test]
fn hints_normal() {
    insta::assert_snapshot!("hints_normal", report(&load(Strictness::Normal)));
}

#[test]
fn hints_strict() {
    insta::assert_snapshot!("hints_strict", report(&load(Strictness::Strict)));
}

/// The hints the M4 milestone lists, each with the fixture that triggers it.
#[test]
fn every_promised_hint_has_a_fixture() {
    let normal = load(Strictness::Normal);
    let strict = load(Strictness::Strict);
    let hint_for = |set: &MibSet, module: &str, code: &str| -> String {
        set.diagnostics()
            .iter()
            .find(|d| d.module.as_deref() == Some(module) && d.code == code)
            .and_then(|d| d.hint.clone())
            .unwrap_or_else(|| panic!("no hint for {code} in {module}"))
    };

    // Missing IMPORTS, naming the module that defines the symbol.
    let hint = hint_for(&normal, "HINT-MISSING-IMPORT-MIB", "type-unknown");
    assert!(hint.contains("`HintLevel FROM HINT-DEFS-MIB`"), "{hint}");
    let hint = hint_for(&normal, "HINT-MISSING-IMPORT-MIB", "oid-orphan");
    assert!(hint.contains("`hintObjects FROM HINT-DEFS-MIB`"), "{hint}");
    let hint = hint_for(&strict, "HINT-MISSING-IMPORT-MIB", "type-unknown");
    assert!(
        hint.contains("SNMPv2-TC") || hint.contains("HINT-DEFS-MIB"),
        "{hint}"
    );
    // Imported from the wrong module.
    let hint = hint_for(&normal, "HINT-WRONG-IMPORT-MIB", "import-not-found");
    assert!(hint.contains("`HintLevel FROM HINT-DEFS-MIB`"), "{hint}");

    // Module/file name mismatch.
    let renamed = normal
        .diagnostics()
        .iter()
        .find(|d| d.code == "import-module-not-found" && d.message.contains("HINT-RENAMED\""))
        .and_then(|d| d.hint.clone())
        .unwrap();
    assert!(renamed.contains("FROM HINT-RENAMED-MIB"), "{renamed}");
    let hint = hint_for(&strict, "HINT-RENAMED-MIB", "module-file-name-mismatch");
    assert!(hint.contains("HINT-RENAMED-MIB.mib"), "{hint}");

    // Imported module not in any search path.
    let missing = normal
        .diagnostics()
        .iter()
        .find(|d| d.code == "import-module-not-found" && d.message.contains("ACME-NOWHERE-MIB"))
        .and_then(|d| d.hint.clone())
        .unwrap();
    assert!(missing.contains("not in any MIB folder"), "{missing}");
    assert!(
        missing.contains(&hints_folder().display().to_string()),
        "{missing}"
    );

    // SMIv1 constructs in SMIv2, both where mib-rs reports them and in a
    // module mixing both versions, where only our own check does.
    let hint = hint_for(&normal, "HINT-SMIV1-IN-SMIV2-MIB", "access-in-smiv2");
    assert!(hint.contains("MAX-ACCESS"), "{hint}");
    let hint = hint_for(&normal, "HINT-SMIV1-IN-SMIV2-MIB", "status-invalid-smiv2");
    assert!(hint.contains("current"), "{hint}");
    let mixed: Vec<&Diagnostic> = normal
        .diagnostics()
        .iter()
        .filter(|d| d.code == "smiv1-in-smiv2")
        .collect();
    assert_eq!(mixed.len(), 4, "{mixed:#?}");
    assert!(
        mixed
            .iter()
            .any(|d| d.hint.as_ref().unwrap().contains("NOTIFICATION-TYPE"))
    );

    // Invalid identifiers.
    let hint = hint_for(&normal, "HINT-IDENTIFIERS-MIB", "identifier-hyphen-end");
    assert!(hint.contains("badEndsWithHyphen."), "{hint}");
    let hint = hint_for(&normal, "HINT-IDENTIFIERS-MIB", "identifier-length-64");
    assert!(hint.contains("64"), "{hint}");
    let hint = hint_for(&strict, "HINT-IDENTIFIERS-MIB", "identifier-underscore");
    assert!(hint.contains("badUnderscore"), "{hint}");
    let hint = hint_for(&strict, "HINT-IDENTIFIERS-MIB", "identifier-hyphen-smiv2");
    assert!(hint.contains("badHyphen"), "{hint}");
}

#[test]
fn trace_explains_a_wrong_import() {
    let set = load(Strictness::Normal);
    let trace = set.trace("HINT-WRONG-IMPORT-MIB::HintLevel").unwrap();
    assert_eq!(trace.outcome, mib_core::TraceOutcome::Missing);
    let import = trace.import.as_ref().unwrap();
    assert_eq!(import.declared_module, "SNMPv2-TC");
    assert!(import.declared_module_loaded);
    assert_eq!(trace.definitions.len(), 1);
    assert_eq!(trace.definitions[0].module, "HINT-DEFS-MIB");
    let text = trace.to_string();
    assert!(
        text.contains("SNMPv2-TC does not define HintLevel"),
        "{text}"
    );

    let trace = set.trace("HINT-DEFS-MIB::hintLevel").unwrap();
    assert_eq!(
        trace.outcome,
        mib_core::TraceOutcome::Resolved {
            module: "HINT-DEFS-MIB".to_string(),
            by: mib_core::ResolvedBy::Local
        }
    );
    assert!(matches!(
        set.trace("NO-SUCH-MIB::x"),
        Err(mib_core::TraceError::ModuleNotFound(_))
    ));
}

#[test]
fn source_view_points_at_a_diagnostic() {
    let set = load(Strictness::Normal);
    let diagnostic = set
        .diagnostics()
        .iter()
        .find(|d| d.code == "import-not-found")
        .unwrap();
    let location = diagnostic.location.as_ref().unwrap();
    let view = set.source_view(&location.file).expect("a loaded file");
    let start = view.offset(location.line, location.column).unwrap();
    let end = view.offset(location.end_line, location.end_column).unwrap();
    assert_eq!(&view.text[start..end], "HintLevel");
    assert!(
        view.tokens
            .iter()
            .any(|token| token.class == mib_core::TokenClass::Comment)
    );
}

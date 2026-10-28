//! Load the MIB corpus in `corpus/` and compare the result with stored
//! snapshots in `tests/snapshots/`.
//!
//! Every module in the corpus must load in permissive mode. The snapshot
//! records which modules loaded and every diagnostic, so any change in how a
//! module is read shows up as a snapshot difference in review.
//!
//! When a change is intended, update the snapshots with
//! `cargo insta test --accept` (install with `cargo install cargo-insta`), or
//! run the tests with `INSTA_UPDATE=always` and review the diff in git.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use mib_core::{MibLibrary, MibSet, Severity};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

/// Load one folder of the corpus and check that every module in it loaded.
fn load_and_check(folder: &Path) -> MibSet {
    let library = MibLibrary::new()
        .with_folder(folder)
        .with_system_folders(false);
    let index = library.index().expect("corpus folder is readable");
    assert!(!index.is_empty(), "no MIB files in {}", folder.display());

    let set = library.load().expect("corpus loads in permissive mode");

    let fatal: Vec<String> = set
        .diagnostics()
        .iter()
        .filter(|d| d.severity == Severity::Fatal)
        .map(ToString::to_string)
        .collect();
    assert!(fatal.is_empty(), "fatal diagnostics:\n{}", fatal.join("\n"));

    for file in index.iter().filter(|file| !file.shadowed) {
        assert!(
            set.module(&file.module).is_some(),
            "module {} from {} did not load",
            file.module,
            file.file.display()
        );
    }
    set
}

/// A stable, platform-independent text summary of a loaded set: file paths
/// are relative to `corpus/` with forward slashes, and lines are sorted.
fn summary(set: &MibSet) -> String {
    let corpus = corpus_dir().canonicalize().unwrap();
    let relative = |path: &Path| -> String {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path = path.strip_prefix(&corpus).unwrap_or(&path);
        path.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    };

    let mut modules = set.modules();
    modules.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = String::new();
    writeln!(out, "modules: {}", modules.len()).unwrap();
    writeln!(out, "nodes: {}", set.node_count()).unwrap();
    for module in &modules {
        let file = match &module.file {
            Some(_) if module.embedded => "(built in)".to_string(),
            Some(file) => relative(file),
            None => "(no source)".to_string(),
        };
        writeln!(out, "  {} {} {}", module.name, module.smi_version, file).unwrap();
    }

    let mut diagnostics: Vec<_> = set.diagnostics().iter().collect();
    diagnostics.sort_by_key(|d| {
        let location = d
            .location
            .as_ref()
            .map(|l| (relative(&l.file), l.line, l.column));
        (location, d.code.clone(), d.message.clone())
    });
    writeln!(out, "diagnostics: {}", diagnostics.len()).unwrap();
    for d in diagnostics {
        let location = match &d.location {
            Some(l) => format!("{}:{}:{}: ", relative(&l.file), l.line, l.column),
            None => String::new(),
        };
        let module = d.module.as_deref().unwrap_or("-");
        writeln!(
            out,
            "  {location}{}: {module}: {} [{}]",
            d.severity, d.message, d.code
        )
        .unwrap();
    }
    out
}

/// The IETF modules committed to the repository. Runs everywhere, including CI.
#[test]
fn committed_corpus() {
    let set = load_and_check(&corpus_dir().join("ietf"));
    insta::assert_snapshot!("committed", summary(&set));
}

/// The whole corpus, including the vendor and older RFC modules that
/// `corpus/fetch.sh` downloads. Skipped when they have not been fetched.
#[test]
fn full_corpus() {
    let corpus = corpus_dir();
    if !corpus.join("fetched").is_dir() {
        eprintln!("skipped: run corpus/fetch.sh to test the full corpus");
        return;
    }
    let set = load_and_check(&corpus);
    insta::assert_snapshot!("full", summary(&set));
}

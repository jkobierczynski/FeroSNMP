//! Run the `ferosnmp` program the way a user does and check its output and
//! exit code. Outputs are compared with stored snapshots in `tests/snapshots/`;
//! see CONTRIBUTING.md for how to accept an intended change.
//!
//! Every command runs from the workspace root with `--no-system-mibs`, so the
//! results do not depend on MIBs installed on the machine running the tests.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Output and exit code of one run.
struct Run {
    /// Standard output with `\\` replaced by `/`.
    stdout: String,
    /// Standard output as printed.
    raw_stdout: String,
    stderr: String,
    code: i32,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const FIXTURES: &str = "crates/mib-core/tests/fixtures";

fn ferosnmp(args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_ferosnmp"))
        .args(args)
        .arg("--no-system-mibs")
        .current_dir(workspace_root())
        .env_remove("FEROSNMP_MIBDIRS")
        .output()
        .expect("ferosnmp runs");
    let raw_stdout = String::from_utf8(output.stdout).unwrap();
    Run {
        // Windows writes `corpus/ietf\FILE`; make snapshots the same everywhere.
        stdout: raw_stdout.replace('\\', "/"),
        raw_stdout,
        stderr: String::from_utf8(output.stderr).unwrap().replace('\\', "/"),
        code: output.status.code().expect("exited normally"),
    }
}

#[test]
fn lint_committed_corpus() {
    let run = ferosnmp(&["lint", "corpus/ietf"]);
    // The corpus imports modules that are not committed (IF-MIB, ...), so
    // there are errors, and the exit code says so.
    assert_eq!(run.code, 1, "{}", run.stderr);
    insta::assert_snapshot!("lint_committed_corpus", run.stdout);
}

/// The whole corpus, including the modules `corpus/fetch.sh` downloads.
/// Skipped when they have not been fetched; the "Full MIB corpus" CI job
/// fetches them.
#[test]
fn lint_full_corpus() {
    if !workspace_root().join("corpus/fetched").is_dir() {
        eprintln!("skipped: run corpus/fetch.sh to lint the full corpus");
        return;
    }
    let run = ferosnmp(&["lint", "corpus"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    insta::assert_snapshot!("lint_full_corpus", run.stdout);
}

#[test]
fn lint_json_matches_text() {
    let run = ferosnmp(&["lint", "corpus/ietf", "--format", "json"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let json: serde_json::Value = serde_json::from_str(&run.raw_stdout).expect("valid JSON");

    let text = ferosnmp(&["lint", "corpus/ietf"]).stdout;
    let summary = text.lines().last().unwrap();
    let expected = format!(
        "{} modules checked: {} errors, {} warnings",
        json["modules"].as_array().unwrap().len(),
        json["errors"],
        json["warnings"]
    );
    assert_eq!(summary, expected);

    let first = &json["diagnostics"][0];
    let file = first["file"].as_str().unwrap().replace('\\', "/");
    assert_eq!(file, "corpus/ietf/BFD-STD-MIB");
    assert_eq!(first["line"], 14);
    assert_eq!(first["column"], 6);
    assert_eq!(first["severity"], "error");
    assert_eq!(first["code"], "import-module-not-found");
}

#[test]
fn lint_clean_file_exits_with_zero() {
    let run = ferosnmp(&["lint", &format!("{FIXTURES}/misnamed-test.mib")]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(run.stdout, "1 module checked: 0 errors, 0 warnings\n");
}

#[test]
fn lint_strict_reports_style_problems() {
    let file = format!("{FIXTURES}/misnamed-test.mib");
    let run = ferosnmp(&["lint", "--strict", &file]);
    // Style problems and warnings are reported but are not errors...
    assert_eq!(run.code, 0);
    insta::assert_snapshot!("lint_strict_fixture", run.stdout);

    // ...unless asked for.
    let run = ferosnmp(&["lint", "--strict", "--fail-on", "warning", &file]);
    assert_eq!(run.code, 1);
}

#[test]
fn lint_reports_files_without_a_module() {
    let run = ferosnmp(&["lint", "Cargo.toml"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stdout
            .starts_with("Cargo.toml: error: no MIB module found")
    );
}

#[test]
fn lint_reports_duplicate_modules() {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join("duplicate-modules");
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("b")).unwrap();
    let source = workspace_root().join(FIXTURES).join("misnamed-test.mib");
    std::fs::copy(&source, folder.join("a.mib")).unwrap();
    std::fs::copy(&source, folder.join("b/copy.mib")).unwrap();

    let run = ferosnmp(&["lint", folder.to_str().unwrap()]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(
        run.stdout
            .contains("warning: FEROSNMP-TEST-MIB: module FEROSNMP-TEST-MIB is also defined in"),
        "{}",
        run.stdout
    );
}

#[test]
fn lint_missing_path_cannot_run() {
    let run = ferosnmp(&["lint", "no-such-file.mib"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr
            .starts_with("ferosnmp: cannot read no-such-file.mib")
    );
}

#[test]
fn mib_show() {
    let run = ferosnmp(&[
        "mib",
        "-M",
        FIXTURES,
        "show",
        "testTemperature",
        "FEROSNMP-TEST-MIB::testPortEntry",
        "1.3.6.1.4.1.99999.1.3.1.2.1.3.102.111.111",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    insta::assert_snapshot!("mib_show", run.stdout);
}

#[test]
fn mib_show_unknown_name() {
    let run = ferosnmp(&["mib", "-M", FIXTURES, "show", "testMode", "noSuchThing"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.starts_with("FEROSNMP-TEST-MIB::testMode\n"));
    assert!(
        run.stderr.contains("noSuchThing: not found"),
        "{}",
        run.stderr
    );
}

#[test]
fn mib_tree() {
    let run = ferosnmp(&["mib", "-M", FIXTURES, "tree", "ferosnmpTestMib"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    insta::assert_snapshot!("mib_tree", run.stdout);

    let run = ferosnmp(&[
        "mib",
        "-M",
        FIXTURES,
        "tree",
        "ferosnmpTestMib",
        "--depth",
        "1",
    ]);
    assert_eq!(run.stdout.lines().count(), 3, "{}", run.stdout);
}

#[test]
fn mib_search() {
    let run = ferosnmp(&["mib", "-M", FIXTURES, "search", "testport*"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    insta::assert_snapshot!("mib_search", run.stdout);

    let run = ferosnmp(&["mib", "-M", FIXTURES, "search", "FEROSNMP-TEST-MIB::*mode"]);
    assert_eq!(run.stdout.lines().count(), 1, "{}", run.stdout);

    let run = ferosnmp(&["mib", "-M", FIXTURES, "search", "nothing*like*this"]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
}

const HINTS: &str = "crates/mib-core/tests/hints";

/// The fix hints, one fixture module per kind of hint.
#[test]
fn lint_hints() {
    let run = ferosnmp(&["lint", HINTS]);
    assert_eq!(run.code, 1);
    // The "not in any MIB folder" hint lists the folders searched.
    assert!(
        run.stdout.contains(&format!("(searched: {HINTS})")),
        "{}",
        run.stdout
    );
    insta::assert_snapshot!("lint_hints", run.stdout);

    let run = ferosnmp(&["lint", HINTS, "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&run.raw_stdout).expect("valid JSON");
    let hinted = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["hint"].is_string())
        .count();
    assert!(hinted >= 10, "{}", run.stdout);
}

#[test]
fn mib_why() {
    let run = ferosnmp(&[
        "mib",
        "-M",
        HINTS,
        "why",
        "--lint",
        "HINT-WRONG-IMPORT-MIB::HintLevel",
    ]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    insta::assert_snapshot!("mib_why_wrong_import", run.stdout);

    let run = ferosnmp(&["mib", "-M", HINTS, "why", "HINT-DEFS-MIB::hintLevel"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout
            .contains("Resolves to HINT-DEFS-MIB::hintLevel: defined in the module itself."),
        "{}",
        run.stdout
    );

    let run = ferosnmp(&["mib", "-M", HINTS, "why", "NO-SUCH-MIB::x"]);
    assert_eq!(run.code, 2);
    assert_eq!(run.stderr, "ferosnmp: module NO-SUCH-MIB is not loaded\n");
}

/// Nothing listens on the discard port of localhost, so the request is
/// refused (or, where the system drops it silently, times out).
#[test]
fn query_without_agent() {
    let run = ferosnmp(&[
        "get",
        "127.0.0.1:9",
        "1.3.6.1.2.1.1.1.0",
        "-t",
        "0.3",
        "-r",
        "0",
    ]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(
        run.stderr.contains("refused") || run.stderr.contains("no response"),
        "{}",
        run.stderr
    );
}

#[test]
fn query_errors() {
    let run = ferosnmp(&["get", "127.0.0.1", "noSuchObjectName.0"]);
    assert_eq!(run.code, 2);
    insta::assert_snapshot!("query_unknown_name", run.stderr);
    let run = ferosnmp(&["walk", "host:99999"]);
    assert_eq!(run.code, 2);
    insta::assert_snapshot!("query_bad_agent", run.stderr);
}

//! Golden-file tests of `get`, `next`, `bulk` and `walk` against a real
//! net-snmp agent serving fixed values (see
//! `crates/snmp-core/tests/agent/`).
//!
//! They run only when `FEROSNMP_TEST_AGENT` names the agent, as the
//! "Query tests (net-snmp)" CI job does:
//!
//! ```sh
//! crates/snmp-core/tests/agent/run-agent.sh
//! FEROSNMP_TEST_AGENT=127.0.0.1:11161 cargo test -p ferosnmp-cli --test agent
//! ```
//!
//! Outputs are compared with the stored snapshots in `tests/snapshots/`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The agent's address, or `None` (and a note) when the tests are skipped.
fn agent() -> Option<String> {
    let agent = std::env::var("FEROSNMP_TEST_AGENT").ok();
    if agent.is_none() {
        eprintln!("skipped: set FEROSNMP_TEST_AGENT=127.0.0.1:11161 and start the test agent");
    }
    agent
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

/// Run `ferosnmp COMMAND AGENT ARGS...` with the test MIB loaded.
fn query(command: &str, agent: &str, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_ferosnmp"))
        .arg(command)
        .arg(agent)
        .args(args)
        .args([
            "-M",
            "crates/snmp-core/tests/agent/mibs",
            "--no-system-mibs",
        ])
        .current_dir(workspace_root())
        .env_remove("FEROSNMP_MIBDIRS")
        .output()
        .expect("ferosnmp runs");
    Run {
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
        code: output.status.code().expect("exited normally"),
    }
}

/// Run and expect success, returning standard output.
fn ok(command: &str, agent: &str, args: &[&str]) -> String {
    let run = query(command, agent, args);
    assert_eq!(run.code, 0, "{}", run.stderr);
    run.stdout
}

#[test]
fn walk_v2c() {
    let Some(agent) = agent() else { return };
    let output = ok("walk", &agent, &["1.3.6.1", "-v", "2c"]);
    insta::assert_snapshot!("walk_v2c", output);
    // GETNEXT gives exactly the same, and so do other GETBULK sizes.
    assert_eq!(ok("walk", &agent, &["1.3.6.1", "--no-bulk"]), output);
    assert_eq!(
        ok("walk", &agent, &["1.3.6.1", "--max-repetitions", "1"]),
        output
    );
    assert_eq!(
        ok("walk", &agent, &["1.3.6.1", "--max-repetitions", "50"]),
        output
    );
}

#[test]
fn walk_v1() {
    let Some(agent) = agent() else { return };
    // SNMPv1 cannot carry Counter64, so the agent skips fstOctets.
    insta::assert_snapshot!("walk_v1", ok("walk", &agent, &["1.3.6.1", "-v", "1"]));
}

#[test]
fn walk_raw_and_numeric() {
    let Some(agent) = agent() else { return };
    insta::assert_snapshot!("walk_raw", ok("walk", &agent, &["fstScalars", "--raw"]));
    insta::assert_snapshot!("walk_numeric", ok("walk", &agent, &["fstScalars", "-n"]));
}

#[test]
fn walk_of_one_instance_and_default_root() {
    let Some(agent) = agent() else { return };
    assert_eq!(
        ok("walk", &agent, &["fstStatus.0"]),
        "FEROSNMP-TEST-MIB::fstStatus.0 = INTEGER: testing(3)\n"
    );
    // Without a root, mib-2: here only the system group is visible.
    assert_eq!(ok("walk", &agent, &[]).lines().count(), 6);
}

#[test]
fn walk_log_counts_requests() {
    let Some(agent) = agent() else { return };
    // 300 table cells, 10 per GETBULK, and one more request that leaves the
    // table.
    let run = query("walk", &agent, &["fstTable", "--log"]);
    assert_eq!(run.code, 0);
    let requests = run
        .stderr
        .lines()
        .filter(|l| l.contains(" GETBULK "))
        .count();
    assert_eq!(requests, 31, "{}", run.stderr);
    assert!(
        run.stderr.contains("done: 300 varbinds, 31 requests"),
        "{}",
        run.stderr
    );
}

#[test]
fn get_next_and_bulk() {
    let Some(agent) = agent() else { return };
    let get = ok(
        "get",
        &agent,
        &[
            "fstString.0",
            "fstTemperature.0",
            "1.3.6.1.2.1.1.5.0",
            "fstName.42",
        ],
    );
    // Nothing follows the last table cell: endOfMibView, and exit code 1.
    let next = query("next", &agent, &["fstString.0", "fstTable", "fstCount.150"]);
    assert_eq!(next.code, 1);
    let next = next.stdout;
    let bulk = ok(
        "bulk",
        &agent,
        &[
            "fstString",
            "fstName",
            "fstCount",
            "--non-repeaters",
            "1",
            "--max-repetitions",
            "3",
        ],
    );
    insta::assert_snapshot!("get_next_bulk", format!("{get}--\n{next}--\n{bulk}"));
}

#[test]
fn missing_objects() {
    let Some(agent) = agent() else { return };
    // SNMPv2c: an exception in place of the value, and exit code 1.
    let v2c = query("get", &agent, &["fstString.0", "fstString.1"]);
    assert_eq!(v2c.code, 1);
    assert_eq!(
        v2c.stdout,
        "FEROSNMP-TEST-MIB::fstString.0 = STRING: \"FeroSNMP test agent\"\n\
         FEROSNMP-TEST-MIB::fstString.1 = No Such Instance currently exists at this OID\n"
    );
    // SNMPv1: the whole request fails with noSuchName.
    let v1 = query("get", &agent, &["fstString.0", "fstString.1", "-v", "1"]);
    assert_eq!(v1.code, 1);
    assert_eq!(v1.stdout, "");
    assert!(
        v1.stderr
            .ends_with("answered noSuchName(2) for FEROSNMP-TEST-MIB::fstString.1\n"),
        "{}",
        v1.stderr
    );
    // GETBULK does not exist in SNMPv1.
    let bulk = query("bulk", &agent, &["fstString", "-v", "1"]);
    assert_eq!(bulk.code, 2);
}

#[test]
fn wrong_community_times_out() {
    let Some(agent) = agent() else { return };
    // net-snmp does not answer requests with an unknown community.
    let run = query(
        "get",
        &agent,
        &["fstString.0", "-c", "wrong", "-t", "0.3", "-r", "1"],
    );
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("no response from"), "{}", run.stderr);
    assert!(
        run.stderr.contains("after 2 attempts of 0.3 s each"),
        "{}",
        run.stderr
    );
}

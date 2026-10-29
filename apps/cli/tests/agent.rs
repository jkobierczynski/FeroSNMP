//! Golden-file tests of `get`, `next`, `bulk`, `walk`, `table` and `set`
//! against a real
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
    let next = query(
        "next",
        &agent,
        &[
            "fstString.0",
            "fstTable",
            "fstCount.150",
            "fstPolicyHits.2.0.0",
        ],
    );
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

/// The SNMPv3 users in the test agent's snmpd.conf: authentication
/// protocol and privacy protocol, as net-snmp and ferosnmp name them.
const AUTH: [&str; 6] = ["MD5", "SHA", "SHA-224", "SHA-256", "SHA-384", "SHA-512"];
const PRIV: [Option<&str>; 5] = [
    None,
    Some("DES"),
    Some("AES"),
    Some("AES-192"),
    Some("AES-256"),
];

fn v3_user(auth: &str, privacy: Option<&str>) -> String {
    let short = |name: &str| name.to_ascii_lowercase().replace('-', "");
    format!(
        "v3-{}-{}",
        short(auth),
        privacy.map_or("none".into(), short)
    )
}

/// The options for one SNMPv3 user of the test agent.
fn v3_args(auth: &str, privacy: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "-u".to_string(),
        v3_user(auth, privacy),
        "-a".into(),
        auth.into(),
        "-A".into(),
        "ferosnmp-auth".into(),
    ];
    if let Some(privacy) = privacy {
        args.extend([
            "-x".into(),
            privacy.into(),
            "-X".into(),
            "ferosnmp-priv".into(),
        ]);
    }
    args
}

#[test]
fn snmpv3_every_auth_and_privacy_protocol() {
    let Some(agent) = agent() else { return };
    let expected = ok("walk", &agent, &["fstScalars", "-v", "2c"]);
    assert!(!expected.is_empty());
    let mut failed = Vec::new();
    for auth in AUTH {
        for privacy in PRIV {
            let mut args = v3_args(auth, privacy);
            args.push("fstScalars".into());
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let run = query("walk", &agent, &args);
            if run.code != 0 || run.stdout != expected {
                failed.push(format!(
                    "{auth}/{}: exit {}, {}",
                    privacy.unwrap_or("no privacy"),
                    run.code,
                    run.stderr.trim()
                ));
            }
        }
    }
    assert!(failed.is_empty(), "failed:\n{}", failed.join("\n"));
    // And a user without authentication.
    assert_eq!(
        ok("walk", &agent, &["-u", "v3-noauth", "fstScalars"]),
        expected
    );
}

#[test]
fn snmpv3_discover() {
    let Some(agent) = agent() else { return };
    let output = ok("discover", &agent, &[]);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 3, "{output}");
    // net-snmp engine IDs start with 80 00 1f 88 (enterprise 8072, format bit).
    assert!(lines[0].starts_with("engine ID:    80001f88"), "{output}");
    assert!(lines[1].starts_with("engine boots: "), "{output}");
    assert!(lines[2].starts_with("engine time:  "), "{output}");
}

/// Run a GET of sysName.0 with these SNMPv3 options and expect it to fail;
/// returns the error message.
fn v3_failure(agent: &str, args: &[&str]) -> String {
    let mut all = args.to_vec();
    all.extend(["-t", "0.5", "-r", "0", "1.3.6.1.2.1.1.5.0"]);
    let run = query("get", agent, &all);
    assert_ne!(run.code, 0, "{args:?} should fail");
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    run.stderr
}

#[test]
fn snmpv3_failures_are_explained() {
    let Some(agent) = agent() else { return };
    let user = v3_user("SHA-256", Some("AES"));
    let message = v3_failure(&agent, &["-u", "nobody"]);
    assert!(
        message.contains("does not know SNMPv3 user \"nobody\" (usmStatsUnknownUserNames)"),
        "{message}"
    );
    let message = v3_failure(
        &agent,
        &[
            "-u",
            &user,
            "-a",
            "SHA-256",
            "-A",
            "wrong-password",
            "-X",
            "ferosnmp-priv",
        ],
    );
    assert!(message.contains("(usmStatsWrongDigests)"), "{message}");
    // The wrong hash with the right password is a wrong digest too.
    let message = v3_failure(
        &agent,
        &[
            "-u",
            &user,
            "-a",
            "SHA",
            "-A",
            "ferosnmp-auth",
            "-X",
            "ferosnmp-priv",
        ],
    );
    assert!(message.contains("(usmStatsWrongDigests)"), "{message}");
    // net-snmp drops what it cannot decrypt; the probe without privacy
    // finds out why.
    for privacy in [["-X", "wrong-password"], ["-x", "DES"]] {
        let mut args = vec!["-u", &user, "-a", "SHA-256", "-A", "ferosnmp-auth"];
        if privacy[0] == "-x" {
            args.extend(["-X", "ferosnmp-priv"]);
        }
        args.extend(privacy);
        let message = v3_failure(&agent, &args);
        assert!(
            message.contains("the privacy password or protocol differs"),
            "{message}"
        );
    }
    // A user without privacy asked for authPriv.
    let auth_only = v3_user("MD5", None);
    let message = v3_failure(
        &agent,
        &[
            "-u",
            &auth_only,
            "-a",
            "MD5",
            "-A",
            "ferosnmp-auth",
            "-X",
            "ferosnmp-priv",
        ],
    );
    assert!(
        message.contains("(usmStatsUnsupportedSecLevels)"),
        "{message}"
    );
    // A privacy user asked for authNoPriv: the access rules say no.
    let message = v3_failure(
        &agent,
        &["-u", &user, "-a", "SHA-256", "-A", "ferosnmp-auth"],
    );
    assert!(message.contains("authorizationError"), "{message}");
    assert!(message.contains("hint: "), "{message}");
}

#[test]
fn tables() {
    let Some(agent) = agent() else { return };
    let mut output = String::new();
    for table in [
        "fstPortTable",
        "fstPortXTable",
        "fstUserTable",
        "fstPolicyTable",
    ] {
        output.push_str(&ok("table", &agent, &[table]));
        output.push_str("--\n");
    }
    // SNMPv1 with GETNEXT gives the same table, apart from the Counter64
    // column, which SNMPv1 cannot carry.
    assert_eq!(
        ok("table", &agent, &["fstPortTable", "-v", "1"]),
        ok("table", &agent, &["fstPortTable"])
    );
    // A column name finds its table too.
    assert_eq!(
        ok("table", &agent, &["fstUserGroup"]),
        ok("table", &agent, &["fstUserTable"])
    );
    output.push_str(&ok("table", &agent, &["fstPortTable", "--format", "csv"]));
    insta::assert_snapshot!("tables", output);

    // Not a table.
    let run = query("table", &agent, &["fstString"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains("is a scalar, not a table"),
        "{}",
        run.stderr
    );
}

/// `ferosnmp set` with the community that may write fstSettings.
fn set(agent: &str, args: &[&str]) -> Run {
    let mut all = args.to_vec();
    all.extend(["-c", "private"]);
    query("set", agent, &all)
}

#[test]
fn set_values() {
    let Some(agent) = agent() else { return };
    // Every type, in one request; the response shows what was set.
    let values = [
        ("fstContact.0", "FeroSNMP tests"),
        ("fstMode.0", "auto"),
        ("fstLimit.0", "75"),
        ("fstGateway.0", "192.0.2.10"),
        ("fstMacAddress.0", "00:1b:21:0a:ff:0a"),
        ("fstOptions.0", "alpha delta"),
        ("fstTarget.0", "fstTable"),
        ("fstThreshold.0", "999"),
    ];
    let args: Vec<&str> = values.iter().flat_map(|(o, v)| [*o, *v]).collect();
    let run = set(&agent, &args);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let objects: Vec<&str> = values.iter().map(|(o, _)| *o).collect();
    let mut get_args = objects.clone();
    get_args.extend(["-c", "private"]);
    // Reading back gives what was set.
    assert_eq!(ok("get", &agent, &get_args), run.stdout);
    let mut output = run.stdout;

    // SNMPv3, by the user that may write.
    let v3 = [
        "-u",
        "v3-writer",
        "-a",
        "SHA-256",
        "-A",
        "ferosnmp-auth",
        "-X",
        "ferosnmp-priv",
    ];
    let mut args = v3.to_vec();
    args.extend(["fstMode.0", "on", "fstLimit.0", "-0"]);
    output.push_str(&ok("set", &agent, &args));
    output.push_str("--\n");

    // Checked against the MIB before sending: nothing is sent.
    for (object, value) in [
        ("fstMode.0", "maybe"),
        ("fstLimit.0", "101"),
        ("fstContact.0", "a contact name that is longer than 32"),
        ("fstMacAddress.0", "00:1b"),
        ("fstOptions.0", "epsilon"),
        ("fstStatus.0", "up"),
    ] {
        let run = set(&agent, &[object, value]);
        assert_eq!(run.code, 2, "{object} {value}");
        output.push_str(&run.stderr);
    }
    output.push_str("--\n");

    // Sent anyway: the agent says no.
    for (object, value) in [("fstLimit.0", "101"), ("fstStatus.0", "up")] {
        let run = set(&agent, &[object, value, "--force"]);
        assert_eq!(run.code, 1, "{object} {value}");
        output.push_str(&run.stderr);
    }
    // A community that may only read.
    let run = query("set", &agent, &["fstLimit.0", "10"]);
    assert_eq!(run.code, 1);
    output.push_str(&run.stderr);
    insta::assert_snapshot!("set", output.replace(&agent, "AGENT"));
}

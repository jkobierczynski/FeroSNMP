//! Golden-file tests of `traps listen` with notifications sent by net-snmp's
//! `snmptrap` and `snmpinform`: SNMPv1 traps, SNMPv2c traps and INFORMs,
//! and SNMPv3 traps and INFORMs with every authentication and privacy
//! protocol Debian's net-snmp has.
//!
//! They run only when `FEROSNMP_TEST_TRAPS=1` is set and the net-snmp tools
//! are installed (Debian and Ubuntu: `sudo apt install snmp`), as in the
//! "Query tests (net-snmp)" CI job:
//!
//! ```sh
//! FEROSNMP_TEST_TRAPS=1 cargo test -p ferosnmp-cli --test traps
//! ```
//!
//! The test MIBs and users are in `crates/trap-rx/tests/`. Outputs are
//! compared with the stored snapshots in `tests/snapshots/`.

use std::io::{BufRead, BufReader, Read};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn enabled() -> bool {
    let on = std::env::var("FEROSNMP_TEST_TRAPS").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set FEROSNMP_TEST_TRAPS=1 and install net-snmp's snmptrap");
    }
    on
}

/// The receiver's engine ID, fixed so the snapshots do not change.
const RECEIVER: &str = "80000000056665726f736e6d7074657374";
/// The engine ID the SNMPv3 traps are sent with.
const SENDER: &str = "80001f88040102030405";

/// A running `ferosnmp traps listen`.
struct Listen {
    child: Child,
    port: u16,
}

impl Listen {
    /// Start the listener on a free port and wait until it listens.
    fn start(args: &[&str]) -> Listen {
        // A port that was free a moment ago.
        let port = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut child = Command::new(env!("CARGO_BIN_EXE_ferosnmp"))
            .args(["traps", "listen", "--address", "127.0.0.1"])
            .args(["--port", &port.to_string()])
            .args(["--local-engine-id", RECEIVER])
            .args(["-M", "crates/trap-rx/tests/mibs", "--no-system-mibs"])
            .args(["--users", "crates/trap-rx/tests/users.conf"])
            .args(args)
            .current_dir(workspace_root())
            .env_remove("FEROSNMP_MIBDIRS")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("ferosnmp runs");
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        stderr.read_line(&mut line).unwrap();
        assert!(line.contains("listening for traps"), "{line}");
        // Keep reading standard error so the listener never blocks on it.
        std::thread::spawn(move || {
            let mut rest = String::new();
            let _ = stderr.read_to_string(&mut rest);
            eprint!("{rest}");
        });
        Listen { child, port }
    }

    fn target(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Wait for the listener to stop after --count notifications, and
    /// return its output with the times, the senders' ports and the engine
    /// IDs replaced, as they differ from run to run.
    fn output(mut self) -> String {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "the listener failed: {status}");
                break;
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                panic!("the listener did not receive everything in time");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut stdout = String::new();
        self.child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut stdout)
            .unwrap();
        stdout
            .lines()
            .map(|line| {
                // Objects, and JSON (which the tests read themselves).
                if line.starts_with(' ') || line.starts_with('{') {
                    return line.to_string();
                }
                // `TIME 127.0.0.1:PORT SNMPv... engine ID`
                let mut words: Vec<&str> = line.split(' ').collect();
                words[0] = "TIME";
                words[1] = "SOURCE";
                let mut line = words.join(" ");
                if let Some(at) = line.find(", engine ") {
                    let engine = &line[at + 9..];
                    if engine != RECEIVER && engine != SENDER {
                        line = format!("{}, engine SENDER-DEFAULT", &line[..at]);
                    }
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }
}

impl Drop for Listen {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Run a net-snmp tool, without its MIBs or configuration files.
fn net_snmp(tool: &str, args: &[&str]) -> std::process::Output {
    let state = std::env::temp_dir().join("ferosnmp-trap-tests");
    std::fs::create_dir_all(&state).unwrap();
    Command::new(tool)
        .args(args)
        .env("MIBS", "")
        .env("SNMPCONFPATH", &state)
        .env("SNMP_PERSISTENT_DIR", &state)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {tool} (install net-snmp's snmp package): {e}"))
}

fn send(tool: &str, args: &[&str]) {
    let output = net_snmp(tool, args);
    assert!(
        output.status.success(),
        "{tool} {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The objects of the test notifications.
const PORT_CHANGE: [&str; 7] = [
    "1.3.6.1.4.1.8072.9999.2.0.1",
    "1.3.6.1.4.1.8072.9999.2.1.1",
    "s",
    "eth0",
    "1.3.6.1.4.1.8072.9999.2.1.2",
    "i",
    "2",
];
const OVERHEAT: [&str; 4] = [
    "1.3.6.1.4.1.8072.9999.2.0.2",
    "1.3.6.1.4.1.8072.9999.2.1.3",
    "i",
    "85",
];

#[test]
fn v1_and_v2c() {
    if !enabled() {
        return;
    }
    let listen = Listen::start(&["--count", "6"]);
    let target = listen.target();
    let t = target.as_str();
    send(
        "snmptrap",
        &[
            "-v1",
            "-c",
            "public",
            t,
            "1.3.6.1.4.1.8072.9999.3",
            "192.0.2.10",
            "6",
            "3",
            "12345",
            "1.3.6.1.4.1.8072.9999.3.1",
            "s",
            "disk full",
        ],
    );
    send(
        "snmptrap",
        &[
            "-v1",
            "-c",
            "public",
            t,
            "1.3.6.1.4.1.8072.9999.3",
            "192.0.2.10",
            "2",
            "0",
            "500",
            "1.3.6.1.2.1.2.2.1.1.2",
            "i",
            "2",
        ],
    );
    let mut args = vec!["-v2c", "-c", "public", t, "4242"];
    args.extend(PORT_CHANGE);
    send("snmptrap", &args);
    // snmpinform waits for the acknowledgement and fails without one.
    let mut args = vec!["-v2c", "-c", "public", t, "4242"];
    args.extend(OVERHEAT);
    send("snmpinform", &args);
    send(
        "snmptrap",
        &[
            "-v2c",
            "-c",
            "s3cret",
            t,
            "100",
            "1.3.6.1.4.1.9.9.41.2.0.1",
            "1.3.6.1.4.1.9.9.41.1.2.3.1.2.7",
            "s",
            "LINK-3-UPDOWN",
            "1.3.6.1.4.1.8072.9999.2.1.4",
            "C",
            "18446744073709551615",
            "1.3.6.1.4.1.8072.9999.2.1.5",
            "a",
            "192.0.2.77",
        ],
    );
    // An empty uptime: snmptrap fills in its own, which changes; give one.
    let mut args = vec!["-v2c", "-c", "public", t, "1"];
    args.extend(OVERHEAT);
    send("snmptrap", &args);
    insta::assert_snapshot!("traps_v1_v2c", listen.output());
}

/// User names and net-snmp options for every protocol combination.
fn v3_users() -> Vec<(String, Vec<&'static str>)> {
    let mut users = vec![("trap-noauth".to_string(), vec!["-l", "noAuthNoPriv"])];
    for auth in ["MD5", "SHA", "SHA-224", "SHA-256", "SHA-384", "SHA-512"] {
        for privacy in ["none", "DES", "AES", "AES-192", "AES-256"] {
            let name = format!(
                "trap-{}-{}",
                auth.to_lowercase().replace('-', ""),
                privacy.to_lowercase().replace('-', "")
            );
            let mut options = vec!["-a", auth, "-A", "ferosnmp-auth"];
            if privacy == "none" {
                options.extend(["-l", "authNoPriv"]);
            } else {
                options.extend(["-l", "authPriv", "-x", privacy, "-X", "ferosnmp-priv"]);
            }
            users.push((name, options));
        }
    }
    users
}

#[test]
fn v3_traps_with_every_protocol() {
    if !enabled() {
        return;
    }
    let users = v3_users();
    let count = (users.len() + 1).to_string();
    let listen = Listen::start(&["--count", &count, "--format", "json"]);
    let target = listen.target();
    for (user, options) in &users {
        let mut args = vec!["-v3", "-e", SENDER, "-u", user];
        args.extend(options);
        args.extend([target.as_str(), "4242"]);
        args.extend(OVERHEAT);
        send("snmptrap", &args);
    }
    // A user bound to the sender's engine ID.
    let mut args = vec![
        "-v3",
        "-e",
        SENDER,
        "-u",
        "trap-bound",
        "-l",
        "authPriv",
        "-a",
        "SHA",
        "-A",
        "ferosnmp-auth",
        "-x",
        "AES",
        "-X",
        "ferosnmp-priv",
        &target,
        "4242",
    ];
    args.extend(OVERHEAT);
    send("snmptrap", &args);

    let output = listen.output();
    let lines: Vec<serde_json::Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let mut names: Vec<&str> = users.iter().map(|(u, _)| u.as_str()).collect();
    names.push("trap-bound");
    assert_eq!(lines.len(), names.len());
    for (line, user) in lines.iter().zip(names) {
        assert_eq!(line["user"], user, "{line}");
        assert_eq!(line["engine_id"], SENDER);
        assert_eq!(
            line["notification"], "FEROSNMP-TRAP-TEST-MIB::fttOverheat",
            "{line}"
        );
        assert_eq!(line["objects"][0]["value"], "85 degrees Celsius");
    }
    // One of them in full.
    let mut first = lines[1].clone();
    first["time"] = "TIME".into();
    first["source"] = "SOURCE".into();
    insta::assert_snapshot!(
        "traps_v3_json",
        serde_json::to_string_pretty(&first).unwrap()
    );
}

#[test]
fn v3_informs_are_acknowledged() {
    if !enabled() {
        return;
    }
    // snmpinform discovers the receiver's engine ID, then sends the INFORM
    // with keys for it and fails unless it is acknowledged.
    let users: Vec<_> = v3_users()
        .into_iter()
        .filter(|(user, _)| {
            user == "trap-noauth" || user.ends_with("-aes") || user.starts_with("trap-sha-")
        })
        .collect();
    let count = (users.len() + 1).to_string();
    let listen = Listen::start(&["--count", &count]);
    let target = listen.target();
    for (user, options) in &users {
        let mut args = vec!["-v3", "-u", user];
        args.extend(options);
        args.extend([target.as_str(), "4242"]);
        args.extend(PORT_CHANGE);
        send("snmpinform", &args);
    }
    // A user bound to a sender's engine ID also sends INFORMs.
    let mut args = vec![
        "-v3",
        "-u",
        "trap-bound",
        "-l",
        "authPriv",
        "-a",
        "SHA",
        "-A",
        "ferosnmp-auth",
        "-x",
        "AES",
        "-X",
        "ferosnmp-priv",
        &target,
        "4242",
    ];
    args.extend(PORT_CHANGE);
    send("snmpinform", &args);
    insta::assert_snapshot!("traps_v3_informs", listen.output());
}

#[test]
fn wrong_passwords_are_explained() {
    if !enabled() {
        return;
    }
    let listen = Listen::start(&["--count", "1"]);
    let target = listen.target();
    // net-snmp gets a usmStatsWrongDigests Report and gives up.
    let mut args = vec![
        "-v3",
        "-u",
        "trap-sha-aes",
        "-l",
        "authPriv",
        "-a",
        "SHA",
        "-A",
        "not-the-password",
        "-x",
        "AES",
        "-X",
        "ferosnmp-priv",
        "-r",
        "0",
        &target,
        "4242",
    ];
    args.extend(OVERHEAT);
    let output = net_snmp("snmpinform", &args);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Authentication failure"), "{stderr}");
    // The listener is still running; one good trap ends it.
    let mut args = vec!["-v2c", "-c", "public", &target, "1"];
    args.extend(OVERHEAT);
    send("snmptrap", &args);
    assert_eq!(listen.output().lines().count(), 3);
}

#[test]
fn filters_by_notification() {
    if !enabled() {
        return;
    }
    let listen = Listen::start(&["--count", "1", "--notification", "fttOverheat", "-n"]);
    let target = listen.target();
    let mut args = vec!["-v2c", "-c", "public", target.as_str(), "1"];
    args.extend(PORT_CHANGE);
    send("snmptrap", &args);
    let mut args = vec!["-v2c", "-c", "public", target.as_str(), "1"];
    args.extend(OVERHEAT);
    send("snmptrap", &args);
    assert_eq!(
        listen.output(),
        "TIME SOURCE SNMPv2c trap, community \"public\"\n  \
         1.3.6.1.4.1.8072.9999.2.0.2, uptime 0:00:00.01\n  \
         1.3.6.1.4.1.8072.9999.2.1.3 = INTEGER: 85 degrees Celsius\n"
    );
}

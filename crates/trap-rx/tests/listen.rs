//! The listener end to end on localhost: the datagrams net-snmp sent
//! (recorded in `crates/snmp-core/tests/notifications/`) go to a real UDP
//! socket, and what comes out is decoded with the test MIBs in
//! `tests/mibs/` and compared with the snapshots in `tests/snapshots/`.

use std::net::{SocketAddr, UdpSocket};
use std::path::Path;
use std::time::Duration;

use mib_core::{MibLibrary, MibSet};
use snmp_core::{NotificationReceiver, parse_hex};
use trap_rx::{Filter, Incoming, Listener, describe, parse_users};

fn recorded(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../snmp-core/tests/notifications")
        .join(format!("{name}.hex"));
    parse_hex(std::fs::read_to_string(path).unwrap().trim()).unwrap()
}

fn mibs() -> MibSet {
    MibLibrary::new()
        .with_system_folders(false)
        .with_folder(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mibs"))
        .load()
        .unwrap()
}

const USERS: &str = "
createUser trap-sha-aes SHA ferosnmp-auth AES ferosnmp-priv
createUser -e 0x80001f88040102030405 trap-md5-des MD5 ferosnmp-auth DES ferosnmp-priv
createUser trap-noauth
";

/// The time and the sender's port change from run to run.
fn stable(text: &str, source: SocketAddr) -> String {
    text.lines()
        .map(|line| match line.split_once(' ') {
            Some((_time, rest)) if !line.starts_with(' ') => {
                format!("TIME {}", rest.replace(&source.to_string(), "SOURCE"))
            }
            _ => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[tokio::test]
async fn every_kind_of_notification_decodes() {
    let receiver = NotificationReceiver::new(parse_users(USERS).unwrap());
    let listener = Listener::bind("127.0.0.1:0".parse().unwrap(), receiver).unwrap();
    let address = listener.local_addr().unwrap();
    let mut traps = listener.spawn().unwrap();
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    sender
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let source = sender.local_addr().unwrap();
    let mibs = mibs();

    let mut output = String::new();
    for name in [
        "v1-enterprise-specific",
        "v1-link-down",
        "v2c-port-change",
        "v2c-vendor",
        "v2c-inform",
        "v3-sha-aes",
        "v3-md5-des",
        "v3-noauth",
        // No such user here: a problem, not a notification.
        "v3-sha256-none",
    ] {
        sender.send_to(&recorded(name), address).unwrap();
        let incoming = tokio::time::timeout(Duration::from_secs(5), traps.next())
            .await
            .expect("the listener answers")
            .expect("the listener runs");
        output.push_str(&format!("--- {name}\n"));
        match incoming {
            Incoming::Notification(received) => {
                assert_eq!(received.source, source);
                let described = describe(&received, Some(&mibs), false);
                assert!(Filter::default().matches(&described));
                output.push_str(&stable(&described.text(), source));
                let mut json = described.json();
                json["time"] = "TIME".into();
                json["source"] = "SOURCE".into();
                output.push_str(&format!("{json}\n"));
            }
            Incoming::Problem(problem) => {
                output.push_str(&format!("problem: {}\n", problem.error));
                if let Some(hint) = problem.error.hint() {
                    output.push_str(&format!("hint: {hint}\n"));
                }
            }
        }
        if name == "v2c-inform" {
            // The acknowledgement came back to the sender.
            let mut buffer = [0u8; 1500];
            let (length, from) = sender.recv_from(&mut buffer).unwrap();
            assert_eq!(from, address);
            assert_eq!(buffer[..length][0], 0x30);
        }
    }
    insta::assert_snapshot!("every_kind", output);
}

#[tokio::test]
async fn without_mibs_oids_are_numeric_with_a_hint() {
    let listener = Listener::bind(
        "127.0.0.1:0".parse().unwrap(),
        NotificationReceiver::new(vec![]),
    )
    .unwrap();
    let address = listener.local_addr().unwrap();
    let mut traps = listener.spawn().unwrap();
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    let source = sender.local_addr().unwrap();
    let mut output = String::new();
    for name in ["v2c-port-change", "v1-link-down"] {
        sender.send_to(&recorded(name), address).unwrap();
        let Some(Incoming::Notification(received)) = traps.next().await else {
            panic!("{name}: no notification");
        };
        output.push_str(&stable(&describe(&received, None, false).text(), source));
    }
    insta::assert_snapshot!("without_mibs", output);
}

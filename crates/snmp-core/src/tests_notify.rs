//! Tests of the notification receiver, with datagrams recorded from
//! net-snmp's `snmptrap` and `snmpinform` (see
//! `tests/notifications/README.md`).

use super::*;
use crate::ErrorStatus;
use crate::codec;
use crate::session::Request;
use crate::usm::{AuthProtocol, PrivProtocol, parse_hex};
use crate::v3::EngineTime;

fn recorded(name: &str) -> Vec<u8> {
    let text = match name {
        "v1-enterprise-specific" => {
            include_str!("../tests/notifications/v1-enterprise-specific.hex")
        }
        "v1-link-down" => include_str!("../tests/notifications/v1-link-down.hex"),
        "v2c-port-change" => include_str!("../tests/notifications/v2c-port-change.hex"),
        "v2c-vendor" => include_str!("../tests/notifications/v2c-vendor.hex"),
        "v2c-inform" => include_str!("../tests/notifications/v2c-inform.hex"),
        "v3-sha-aes" => include_str!("../tests/notifications/v3-sha-aes.hex"),
        "v3-md5-des" => include_str!("../tests/notifications/v3-md5-des.hex"),
        "v3-sha256-none" => include_str!("../tests/notifications/v3-sha256-none.hex"),
        "v3-noauth" => include_str!("../tests/notifications/v3-noauth.hex"),
        "v3-sha512-aes256" => include_str!("../tests/notifications/v3-sha512-aes256.hex"),
        "v3-inform-discovery" => include_str!("../tests/notifications/v3-inform-discovery.hex"),
        _ => panic!("no recording {name}"),
    };
    parse_hex(text.trim()).unwrap()
}

fn oid(text: &str) -> Oid {
    text.parse().unwrap()
}

/// The engine ID of the SNMPv3 sender in the recordings.
const SENDER: &str = "80001f88040102030405";

fn trap_users() -> Vec<TrapUser> {
    let auth = "ferosnmp-auth";
    let privacy = "ferosnmp-priv";
    vec![
        TrapUser::new(
            Usm::new("trap-sha-aes")
                .with_auth(AuthProtocol::Sha1, auth)
                .with_privacy(PrivProtocol::Aes128, privacy),
        ),
        TrapUser::new(
            Usm::new("trap-md5-des")
                .with_auth(AuthProtocol::Md5, auth)
                .with_privacy(PrivProtocol::Des, privacy),
        )
        .with_engine_id(parse_hex(SENDER).unwrap()),
        TrapUser::new(Usm::new("trap-sha256").with_auth(AuthProtocol::Sha256, auth)),
        TrapUser::new(Usm::new("trap-noauth")),
        TrapUser::new(
            Usm::new("trap-sha512-aes256")
                .with_auth(AuthProtocol::Sha512, auth)
                .with_privacy(PrivProtocol::Aes256, privacy),
        ),
    ]
}

fn receive(receiver: &mut NotificationReceiver, name: &str) -> Notification {
    let handled = receiver.handle(&recorded(name));
    assert!(handled.reply.is_none(), "{name}: a trap is not answered");
    handled.result.unwrap().unwrap()
}

#[test]
fn v1_enterprise_specific_trap_is_translated() {
    let mut receiver = NotificationReceiver::new(vec![]);
    let n = receive(&mut receiver, "v1-enterprise-specific");
    assert_eq!(n.version, Version::V1);
    assert_eq!(n.kind, NotificationKind::Trap);
    assert_eq!(
        n.security,
        NotificationSecurity::Community(b"public".to_vec())
    );
    let v1 = n.v1.clone().unwrap();
    assert_eq!(v1.enterprise, oid("1.3.6.1.4.1.8072.9999.3"));
    assert_eq!(v1.agent_address, [192, 0, 2, 10]);
    assert_eq!((v1.generic_trap, v1.specific_trap), (6, 3));
    assert_eq!(v1.generic_name(), Some("enterpriseSpecific"));
    // RFC 3584, 3.1: enterprise.0.specific.
    assert_eq!(n.trap_oid(), Some(&oid("1.3.6.1.4.1.8072.9999.3.0.3")));
    assert_eq!(n.uptime(), Some(12345));
    let all: Vec<String> = n
        .varbinds
        .iter()
        .map(|v| format!("{} {}", v.oid, v.value))
        .collect();
    assert_eq!(
        all,
        [
            "1.3.6.1.2.1.1.3.0 Timeticks: (12345) 0:02:03.45",
            "1.3.6.1.6.3.1.1.4.1.0 OID: 1.3.6.1.4.1.8072.9999.3.0.3",
            "1.3.6.1.4.1.8072.9999.3.1 STRING: \"disk full\"",
            "1.3.6.1.6.3.18.1.3.0 IpAddress: 192.0.2.10",
            "1.3.6.1.6.3.18.1.4.0 STRING: \"public\"",
            "1.3.6.1.6.3.1.1.4.3.0 OID: 1.3.6.1.4.1.8072.9999.3",
        ]
    );
    // The objects are only what the trap itself carried.
    let objects = n.objects();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].oid, oid("1.3.6.1.4.1.8072.9999.3.1"));
}

#[test]
fn v1_generic_trap_becomes_a_standard_notification() {
    let mut receiver = NotificationReceiver::new(vec![]);
    let n = receive(&mut receiver, "v1-link-down");
    assert_eq!(n.v1.as_ref().unwrap().generic_name(), Some("linkDown"));
    // linkDown is snmpTraps.3.
    assert_eq!(n.trap_oid(), Some(&oid("1.3.6.1.6.3.1.1.5.3")));
    assert_eq!(n.objects().len(), 1);
}

#[test]
fn v2c_trap() {
    let mut receiver = NotificationReceiver::new(vec![]);
    let n = receive(&mut receiver, "v2c-port-change");
    assert_eq!(n.version, Version::V2c);
    assert_eq!(n.kind, NotificationKind::Trap);
    assert!(n.v1.is_none());
    assert_eq!(n.trap_oid(), Some(&oid("1.3.6.1.4.1.8072.9999.2.0.1")));
    assert_eq!(n.uptime(), Some(4242));
    let objects: Vec<String> = n
        .objects()
        .iter()
        .map(|v| format!("{} {}", v.oid, v.value))
        .collect();
    assert_eq!(
        objects,
        [
            "1.3.6.1.4.1.8072.9999.2.1.1 STRING: \"eth0\"",
            "1.3.6.1.4.1.8072.9999.2.1.2 INTEGER: 2"
        ]
    );
}

#[test]
fn v2c_values_are_never_cut_short() {
    let mut receiver = NotificationReceiver::new(vec![]);
    let n = receive(&mut receiver, "v2c-vendor");
    assert_eq!(
        n.security,
        NotificationSecurity::Community(b"s3cret".to_vec())
    );
    let objects = n.objects();
    assert_eq!(objects.len(), 3);
    assert_eq!(objects[1].value, Value::Counter64(u64::MAX));
    assert_eq!(objects[2].value, Value::IpAddress([192, 0, 2, 77]));
}

#[test]
fn v2c_inform_is_acknowledged() {
    let mut receiver = NotificationReceiver::new(vec![]);
    let handled = receiver.handle(&recorded("v2c-inform"));
    let n = handled.result.unwrap().unwrap();
    assert_eq!(n.kind, NotificationKind::Inform);
    let reply = handled.reply.expect("an INFORM is answered");
    // A Response with the same request-id, community and variables.
    let (request_id, response) = codec::decode_response(&reply).unwrap();
    assert_eq!(request_id, n.request_id);
    assert_eq!(response.status, ErrorStatus::NoError);
    assert_eq!(response.varbinds, n.varbinds);
    let parsed = snmp2::Pdu::from_bytes(&reply).unwrap();
    assert_eq!(parsed.community, b"public");
}

#[test]
fn v3_traps_at_every_level() {
    let mut receiver = NotificationReceiver::new(trap_users());
    for (name, user, level) in [
        ("v3-sha-aes", "trap-sha-aes", SecurityLevel::AuthPriv),
        ("v3-md5-des", "trap-md5-des", SecurityLevel::AuthPriv),
        ("v3-sha256-none", "trap-sha256", SecurityLevel::AuthNoPriv),
        ("v3-noauth", "trap-noauth", SecurityLevel::NoAuthNoPriv),
        (
            "v3-sha512-aes256",
            "trap-sha512-aes256",
            SecurityLevel::AuthPriv,
        ),
    ] {
        let n = receive(&mut receiver, name);
        assert_eq!(n.version, Version::V3, "{name}");
        assert_eq!(n.kind, NotificationKind::Trap);
        let NotificationSecurity::Usm(security) = &n.security else {
            panic!("{name}: not SNMPv3");
        };
        assert_eq!(security.user, user);
        assert_eq!(security.level, level, "{name}");
        assert_eq!(hex(&security.engine_id), SENDER);
        // snmptrap -e sets only the authoritative engine; the context
        // engine stays its own default.
        assert!(!security.context_engine_id.is_empty());
        assert_eq!(
            n.trap_oid(),
            Some(&oid("1.3.6.1.4.1.8072.9999.2.0.2")),
            "{name}"
        );
        assert_eq!(n.uptime(), Some(4242));
        assert_eq!(n.objects().len(), 1);
        assert_eq!(n.objects()[0].value, Value::Integer(85));
    }
}

fn security_problem(handled: Handled) -> SecurityProblem {
    match handled.result {
        Err(ReceiveError::Security { problem, .. }) => problem,
        other => panic!("expected a security error, got {other:?}"),
    }
}

#[test]
fn v3_security_failures_are_explained() {
    let packet = recorded("v3-sha-aes");
    // No such user.
    let mut receiver = NotificationReceiver::new(vec![]);
    let handled = receiver.handle(&packet);
    assert!(handled.reply.is_none(), "traps are never answered");
    let error = handled.result.unwrap_err();
    assert!(error.to_string().contains("no trap user"), "{error}");
    assert!(
        error
            .hint()
            .unwrap()
            .contains("createUser -e 80001f88040102030405 trap-sha-aes")
    );

    // Another authentication password.
    let mut receiver = NotificationReceiver::new(vec![TrapUser::new(
        Usm::new("trap-sha-aes")
            .with_auth(AuthProtocol::Sha1, "something-else")
            .with_privacy(PrivProtocol::Aes128, "ferosnmp-priv"),
    )]);
    assert_eq!(
        security_problem(receiver.handle(&packet)),
        SecurityProblem::WrongDigest
    );

    // Another privacy password.
    let mut receiver = NotificationReceiver::new(vec![TrapUser::new(
        Usm::new("trap-sha-aes")
            .with_auth(AuthProtocol::Sha1, "ferosnmp-auth")
            .with_privacy(PrivProtocol::Aes128, "something-else"),
    )]);
    assert_eq!(
        security_problem(receiver.handle(&packet)),
        SecurityProblem::DecryptionError
    );

    // Bound to another sender.
    let mut receiver = NotificationReceiver::new(vec![
        TrapUser::new(
            Usm::new("trap-sha-aes")
                .with_auth(AuthProtocol::Sha1, "ferosnmp-auth")
                .with_privacy(PrivProtocol::Aes128, "ferosnmp-priv"),
        )
        .with_engine_id(vec![0x80, 0, 0, 0, 1, 9]),
    ]);
    assert_eq!(
        security_problem(receiver.handle(&packet)),
        SecurityProblem::UnknownEngineId
    );

    // A user without privacy does not accept encrypted traps, and a user
    // with authentication does not accept unauthenticated ones.
    let mut receiver = NotificationReceiver::new(vec![
        TrapUser::new(Usm::new("trap-sha-aes").with_auth(AuthProtocol::Sha1, "ferosnmp-auth")),
        TrapUser::new(Usm::new("trap-noauth").with_auth(AuthProtocol::Sha1, "ferosnmp-auth")),
    ]);
    assert_eq!(
        security_problem(receiver.handle(&packet)),
        SecurityProblem::UnsupportedSecurityLevel
    );
    assert_eq!(
        security_problem(receiver.handle(&recorded("v3-noauth"))),
        SecurityProblem::UnsupportedSecurityLevel
    );
}

#[test]
fn v3_discovery_gets_our_engine_id() {
    let mut receiver = NotificationReceiver::new(trap_users());
    receiver.set_engine_time(1234);
    let handled = receiver.handle(&recorded("v3-inform-discovery"));
    assert_eq!(handled.result, Ok(None));
    let reply = handled.reply.expect("discovery is answered");
    // What the sender's engine (snmp-core's own session code) makes of it.
    let usm = Usm::new("anyone");
    let message = v3::decode_message(&reply).unwrap();
    let received = v3::decode_received(&reply, &usm, None, &[message.msg_id])
        .unwrap()
        .unwrap();
    assert!(received.is_report());
    assert_eq!(received.engine.engine_id, receiver.engine_id());
    assert_eq!(received.engine.boots, 1);
    assert!((1234..1240).contains(&received.engine.time));
    assert_eq!(
        v3::report_problem(&received.response),
        SecurityProblem::UnknownEngineId
    );
}

/// An SNMPv3 InformRequest from `usm` to `receiver`, built as net-snmp
/// does after discovery.
fn inform(usm: &Usm, receiver: &NotificationReceiver, engine_time: u32) -> (Vec<u8>, Keys) {
    let engine = EngineTime {
        engine_id: receiver.engine_id().to_vec(),
        boots: 1,
        time: engine_time,
    };
    let keys = Keys::derive(usm, &engine.engine_id);
    let varbinds = vec![
        Varbind {
            oid: Oid::new(SYS_UP_TIME_0.to_vec()),
            value: Value::TimeTicks(77),
        },
        Varbind {
            oid: Oid::new(SNMP_TRAP_OID_0.to_vec()),
            value: Value::ObjectIdentifier(oid("1.3.6.1.4.1.8072.9999.2.0.1")),
        },
        Varbind {
            oid: oid("1.3.6.1.4.1.8072.9999.2.1.1"),
            value: Value::OctetString(b"eth1".to_vec()),
        },
    ];
    // An InformRequest has the shape of a SET request with another tag.
    let mut pdu = codec::encode_pdu(321, &Request::Set(varbinds)).unwrap();
    pdu[0] = INFORM_REQUEST;
    let scoped = v3::scoped_pdu(&engine.engine_id, b"", &pdu);
    let bytes = v3::seal(
        usm,
        usm.level,
        &keys,
        99,
        FLAG_REPORTABLE,
        &engine,
        &scoped,
        5,
    );
    (bytes, keys)
}

#[test]
fn v3_inform_is_acknowledged_with_our_engine() {
    // A user bound to a sender's engine ID can still send INFORMs, which
    // use the receiver's engine ID.
    let mut receiver = NotificationReceiver::new(trap_users());
    receiver.set_engine_time(500);
    for user in ["trap-sha-aes", "trap-md5-des", "trap-sha256"] {
        let usm = trap_users()
            .into_iter()
            .find(|u| u.usm.user == user)
            .unwrap()
            .usm;
        let (bytes, keys) = inform(&usm, &receiver, 510);
        let handled = receiver.handle(&bytes);
        let n = handled.result.unwrap().unwrap();
        assert_eq!(n.kind, NotificationKind::Inform);
        assert_eq!(n.request_id, 321);
        assert_eq!(n.trap_oid(), Some(&oid("1.3.6.1.4.1.8072.9999.2.0.1")));
        let NotificationSecurity::Usm(security) = &n.security else {
            panic!()
        };
        assert_eq!(security.engine_id, receiver.engine_id());

        // The answer is a Response the sender can authenticate and decrypt.
        let reply = handled.reply.expect("an INFORM is answered");
        let received = v3::decode_received(&reply, &usm, Some(&keys), &[99])
            .unwrap()
            .unwrap();
        assert!(!received.is_report());
        assert!(received.authenticated);
        assert_eq!(received.request_id, 321);
        assert_eq!(received.response.varbinds, n.varbinds, "{user}");
    }
}

#[test]
fn v3_inform_out_of_the_time_window_gets_an_authenticated_report() {
    let mut receiver = NotificationReceiver::new(trap_users());
    receiver.set_engine_time(5000);
    let usm = trap_users().remove(0).usm;
    let (bytes, keys) = inform(&usm, &receiver, 10);
    let handled = receiver.handle(&bytes);
    assert_eq!(
        handled.result.clone().map(|_| ()),
        Err(ReceiveError::Security {
            user: "trap-sha-aes".to_string(),
            engine_id: receiver.engine_id().to_vec(),
            problem: SecurityProblem::NotInTimeWindow
        })
    );
    let reply = handled.reply.unwrap();
    let received = v3::decode_received(&reply, &usm, Some(&keys), &[99])
        .unwrap()
        .unwrap();
    assert!(received.is_report());
    assert!(received.authenticated);
    assert!((5000..5010).contains(&received.engine.time));
    assert_eq!(
        v3::report_problem(&received.response),
        SecurityProblem::NotInTimeWindow
    );
}

#[test]
fn inform_to_another_engine_gets_unknown_engine_id() {
    let mut receiver = NotificationReceiver::new(trap_users());
    let other = NotificationReceiver::new(vec![]);
    let usm = trap_users().remove(0).usm;
    let (bytes, _) = inform(&usm, &other, 0);
    let handled = receiver.handle(&bytes);
    assert_eq!(
        security_problem(Handled {
            result: handled.result,
            reply: None
        }),
        SecurityProblem::UnknownEngineId
    );
    let reply = handled.reply.unwrap();
    let received = v3::decode_received(&reply, &usm, None, &[99])
        .unwrap()
        .unwrap();
    assert_eq!(
        v3::report_problem(&received.response),
        SecurityProblem::UnknownEngineId
    );
}

#[test]
fn requests_and_junk_are_not_notifications() {
    let mut receiver = NotificationReceiver::new(vec![]);
    let get = codec::encode_request(
        Version::V2c,
        b"public",
        1,
        &Request::Get(vec![oid("1.3.6.1.2.1.1.1.0")]),
    )
    .unwrap();
    let handled = receiver.handle(&get);
    assert_eq!(
        handled.result,
        Err(ReceiveError::NotNotification {
            version: Version::V2c,
            pdu: "GetRequest"
        })
    );
    assert!(handled.reply.is_none());

    for junk in [&b""[..], b"hello", &[0x30, 0x03, 0x02, 0x01, 0x07]] {
        assert!(matches!(
            receiver.handle(junk).result,
            Err(ReceiveError::Malformed(_))
        ));
    }
    // A truncated recording is malformed, never a panic.
    for name in ["v1-enterprise-specific", "v2c-inform", "v3-sha-aes"] {
        let bytes = recorded(name);
        for len in 0..bytes.len() {
            let _ = receiver.handle(&bytes[..len]);
        }
    }
}

#[test]
fn engine_ids_are_new_each_time() {
    let a = new_engine_id();
    let b = new_engine_id();
    assert_ne!(a, b);
    assert_eq!(a.len(), 21);
    assert_eq!(&a[..5], [0x80, 0, 0, 0, 5]);
}

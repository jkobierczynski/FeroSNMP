//! SNMPv3 engine tests against a fake agent: discovery, synchronizing with
//! the agent's clock, a changed engine ID, and errors from Reports. The
//! protocol matrix against real net-snmp is in `apps/cli/tests/agent.rs`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mib_core::Oid;
use tokio::net::UdpSocket;

use crate::codec::{REPORT, RESPONSE};
use crate::usm::Keys;
use crate::v3::{EngineTime, encode_answer, peek_request};
use crate::{
    AuthProtocol, Engine, EngineConfig, Error, Event, Operation, SecurityProblem, Target, Usm,
    Value,
};

fn oid(text: &str) -> Oid {
    text.parse().unwrap()
}

fn user() -> Usm {
    Usm::new("tester").with_auth(AuthProtocol::Sha256, "password1")
}

#[derive(Debug)]
struct State {
    engine: EngineTime,
    /// The time the discovery Report gives, when not the real one (some
    /// agents send 0 there).
    discovery_time: Option<u32>,
    seen: Vec<&'static str>,
}

struct FakeV3 {
    state: Mutex<State>,
}

impl FakeV3 {
    fn set_engine_id(&self, id: &[u8]) {
        self.state.lock().unwrap().engine.engine_id = id.to_vec();
    }

    fn seen(&self) -> Vec<&'static str> {
        self.state.lock().unwrap().seen.clone()
    }

    fn answer(&self, bytes: &[u8]) -> Vec<u8> {
        let mut state = self.state.lock().unwrap();
        let request = peek_request(bytes);
        let usm = user();
        let engine = state.engine.clone();
        let keys = Keys::derive(&usm, &engine.engine_id);
        let report = |counter: u32, keys: Option<&Keys>, engine: &EngineTime| {
            let oid = Oid::new(vec![1, 3, 6, 1, 6, 3, 15, 1, 1, counter, 0]);
            encode_answer(
                &usm,
                keys,
                engine,
                request.msg_id,
                REPORT,
                request.request_id,
                &[(oid, Value::Counter32(1))],
            )
        };
        if request.engine.engine_id.is_empty() {
            state.seen.push("discovery");
            let mut reported = engine.clone();
            if let Some(time) = state.discovery_time {
                reported.time = time;
            }
            return report(4, None, &reported);
        }
        if request.engine.engine_id != engine.engine_id {
            state.seen.push("unknown engine");
            return report(4, None, &engine);
        }
        if request.user != usm.user.as_bytes() {
            state.seen.push("unknown user");
            return report(3, None, &engine);
        }
        if request.engine.boots != engine.boots || request.engine.time.abs_diff(engine.time) > 150 {
            state.seen.push("time window");
            return report(2, Some(&keys), &engine);
        }
        state.seen.push("get");
        encode_answer(
            &usm,
            Some(&keys),
            &engine,
            request.msg_id,
            RESPONSE,
            request.request_id,
            &[(
                oid("1.3.6.1.2.1.1.1.0"),
                Value::OctetString(b"fake".to_vec()),
            )],
        )
    }
}

async fn fake_v3(discovery_time: Option<u32>) -> (SocketAddr, Arc<FakeV3>) {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let address = socket.local_addr().unwrap();
    let agent = Arc::new(FakeV3 {
        state: Mutex::new(State {
            engine: EngineTime {
                engine_id: b"\x80\x00\x1f\x88\x04fake".to_vec(),
                boots: 3,
                time: 5000,
            },
            discovery_time,
            seen: Vec::new(),
        }),
    });
    let serving = agent.clone();
    tokio::spawn(async move {
        let mut buffer = vec![0; 65_535];
        loop {
            let Ok((len, from)) = socket.recv_from(&mut buffer).await else {
                return;
            };
            let answer = serving.answer(&buffer[..len]);
            let _ = socket.send_to(&answer, from).await;
        }
    });
    (address, agent)
}

fn target(address: SocketAddr, usm: Usm) -> Target {
    Target::new(address.ip().to_string(), address.port())
        .with_usm(usm)
        .with_timeout(Duration::from_millis(300))
        .with_retries(1)
}

#[derive(Debug, Default)]
struct Outcome {
    values: Vec<Value>,
    engines: Vec<crate::EngineInfo>,
    error: Option<Error>,
}

async fn run(engine: &Engine, target: Target, operation: Operation) -> Outcome {
    let mut running = engine.submit(target, operation).unwrap();
    let mut outcome = Outcome::default();
    while let Some(event) = running.next_event().await {
        match event {
            Event::Varbinds(varbinds) => {
                outcome.values.extend(varbinds.into_iter().map(|v| v.value));
            }
            Event::Engine(info) => outcome.engines.push(info),
            Event::Failed(error) => outcome.error = Some(error),
            _ => {}
        }
    }
    outcome
}

fn get() -> Operation {
    Operation::Get(vec![oid("1.3.6.1.2.1.1.1.0")])
}

#[tokio::test]
async fn discovers_once_and_remembers_the_engine() {
    let (address, agent) = fake_v3(None).await;
    let engine = Engine::start(EngineConfig::default());
    let first = run(&engine, target(address, user()), get()).await;
    assert_eq!(first.error, None);
    assert_eq!(first.values, [Value::OctetString(b"fake".to_vec())]);
    assert_eq!(first.engines.len(), 1);
    assert!(first.engines[0].fresh);
    assert_eq!(first.engines[0].boots, 3);
    assert_eq!(first.engines[0].engine_id, b"\x80\x00\x1f\x88\x04fake");

    let second = run(&engine, target(address, user()), get()).await;
    assert_eq!(second.error, None);
    assert!(!second.engines[0].fresh);
    assert_eq!(agent.seen(), ["discovery", "get", "get"]);
}

#[tokio::test]
async fn synchronizes_with_the_agents_clock() {
    // The discovery Report says time 0; the agent's real time is 5000.
    let (address, agent) = fake_v3(Some(0)).await;
    let engine = Engine::start(EngineConfig::default());
    let outcome = run(&engine, target(address, user()), get()).await;
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.values.len(), 1);
    assert_eq!(agent.seen(), ["discovery", "time window", "get"]);
}

#[tokio::test]
async fn discovers_again_when_the_engine_id_changes() {
    let (address, agent) = fake_v3(None).await;
    let engine = Engine::start(EngineConfig::default());
    assert_eq!(
        run(&engine, target(address, user()), get()).await.error,
        None
    );
    // The device was replaced.
    agent.set_engine_id(b"\x80\x00\x1f\x88\x04new!");
    let outcome = run(&engine, target(address, user()), get()).await;
    assert_eq!(outcome.error, None);
    assert_eq!(
        agent.seen(),
        ["discovery", "get", "unknown engine", "discovery", "get"]
    );
}

#[tokio::test]
async fn reports_become_clear_errors() {
    let (address, _) = fake_v3(None).await;
    let engine = Engine::start(EngineConfig::default());
    let outcome = run(&engine, target(address, Usm::new("nobody")), get()).await;
    let Some(Error::Security { problem, user, .. }) = &outcome.error else {
        panic!("{:?}", outcome.error);
    };
    assert_eq!(*problem, SecurityProblem::UnknownUser);
    assert_eq!(user, "nobody");
    let text = outcome.error.unwrap().to_string();
    assert!(
        text.contains("does not know SNMPv3 user \"nobody\""),
        "{text}"
    );
}

#[tokio::test]
async fn wrong_digests_in_responses_are_refused() {
    let (address, _) = fake_v3(None).await;
    let engine = Engine::start(EngineConfig::default());
    // Same user, other password: the agent (which does not check, being
    // fake) answers with its own key, and the answer fails our check.
    let usm = Usm::new("tester").with_auth(AuthProtocol::Sha256, "password2");
    let outcome = run(&engine, target(address, usm), get()).await;
    assert!(
        matches!(
            &outcome.error,
            Some(Error::Security {
                problem: SecurityProblem::ResponseWrongDigest,
                ..
            })
        ),
        "{:?}",
        outcome.error
    );
}

#[tokio::test]
async fn discover_needs_snmpv3() {
    let engine = Engine::start(EngineConfig::default());
    let target = Target::new("127.0.0.1", 9);
    let outcome = run(&engine, target, Operation::Discover).await;
    assert_eq!(outcome.error, Some(Error::DiscoveryNeedsV3));
}

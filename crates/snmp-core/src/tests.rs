//! Engine tests against a small fake agent on localhost, so walks, retries
//! and fallbacks are tested on every platform without net-snmp. The golden
//! tests against real net-snmp are in the CLI (`apps/cli/tests/agent.rs`).

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::ops::Bound;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mib_core::Oid;
use tokio::net::UdpSocket;

use crate::codec::{integer, tlv};
use crate::{
    Engine, EngineConfig, Error, ErrorStatus, Event, Operation, Target, Value, Varbind, Version,
};

fn oid(text: &str) -> Oid {
    text.parse().unwrap()
}

/// How the fake agent misbehaves.
#[derive(Debug, Clone, Default)]
struct Quirks {
    /// Ignore this many requests before answering.
    ignore_first: usize,
    /// Answer GETBULK with this error status.
    bulk_error: Option<u32>,
    /// Answer GETBULK with tooBig when max-repetitions is above this.
    too_big_above: Option<u32>,
    /// Answer GETNEXT/GETBULK starting here with the first object again.
    loop_at: Option<Oid>,
    /// Wait this long before answering.
    delay: Duration,
}

#[derive(Debug, Default)]
struct Seen {
    /// Request PDU types, by name.
    requests: Vec<&'static str>,
}

struct FakeAgent {
    objects: BTreeMap<Oid, Value>,
    quirks: Quirks,
    seen: Mutex<Seen>,
    ignored: Mutex<usize>,
}

/// Start a fake agent with `objects` and return its address.
async fn fake_agent(objects: Vec<(Oid, Value)>, quirks: Quirks) -> (SocketAddr, Arc<FakeAgent>) {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let address = socket.local_addr().unwrap();
    let agent = Arc::new(FakeAgent {
        objects: objects.into_iter().collect(),
        quirks,
        seen: Mutex::default(),
        ignored: Mutex::default(),
    });
    let shared = agent.clone();
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 65_535];
        loop {
            let Ok((len, peer)) = socket.recv_from(&mut buffer).await else {
                return;
            };
            let request = buffer[..len].to_vec();
            let (agent, socket) = (shared.clone(), socket.clone());
            tokio::spawn(async move {
                if let Some(reply) = agent.answer(&request) {
                    tokio::time::sleep(agent.quirks.delay).await;
                    let _ = socket.send_to(&reply, peer).await;
                }
            });
        }
    });
    (address, agent)
}

impl FakeAgent {
    fn requests(&self) -> Vec<&'static str> {
        self.seen.lock().unwrap().requests.clone()
    }

    fn next_after(&self, oid: &Oid) -> Option<(Oid, Value)> {
        if self.quirks.loop_at.as_ref() == Some(oid) {
            let (first, value) = self.objects.iter().next()?;
            return Some((first.clone(), value.clone()));
        }
        self.objects
            .range((Bound::Excluded(oid.clone()), Bound::Unbounded))
            .next()
            .map(|(oid, value)| (oid.clone(), value.clone()))
    }

    fn answer(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        let pdu = snmp2::Pdu::from_bytes(bytes).ok()?;
        let v1 = pdu.version().ok()? == snmp2::Version::V1;
        let requested: Vec<Oid> = pdu
            .varbinds
            .clone()
            .map(|(oid, _)| Oid::new(oid.iter().unwrap().map(|a| a as u32).collect::<Vec<_>>()))
            .collect();
        let name = match pdu.message_type {
            snmp2::MessageType::GetRequest => "GET",
            snmp2::MessageType::GetNextRequest => "GETNEXT",
            snmp2::MessageType::GetBulkRequest => "GETBULK",
            _ => return None,
        };
        self.seen.lock().unwrap().requests.push(name);
        {
            let mut ignored = self.ignored.lock().unwrap();
            if *ignored < self.quirks.ignore_first {
                *ignored += 1;
                return None;
            }
        }

        let mut status = 0;
        let mut index = 0;
        let mut varbinds = Vec::new();
        let mut fail = |s: u32, i: usize, varbinds: &mut Vec<(Oid, Value)>| {
            status = s;
            index = i as u32 + 1;
            varbinds.clear();
        };
        match name {
            "GET" => {
                for (i, oid) in requested.iter().enumerate() {
                    match self.objects.get(oid) {
                        Some(value) => varbinds.push((oid.clone(), value.clone())),
                        None if v1 => {
                            fail(2, i, &mut varbinds);
                            break;
                        }
                        None => varbinds.push((oid.clone(), Value::NoSuchObject)),
                    }
                }
            }
            "GETNEXT" => {
                for (i, oid) in requested.iter().enumerate() {
                    match self.next_after(oid) {
                        Some(next) => varbinds.push(next),
                        None if v1 => {
                            fail(2, i, &mut varbinds);
                            break;
                        }
                        None => varbinds.push((oid.clone(), Value::EndOfMibView)),
                    }
                }
            }
            _ => {
                let (non_repeaters, max_repetitions) = (pdu.error_status, pdu.error_index);
                if let Some(error) = self.quirks.bulk_error {
                    fail(error, 0, &mut varbinds);
                } else if self
                    .quirks
                    .too_big_above
                    .is_some_and(|n| max_repetitions > n)
                {
                    fail(1, 0, &mut varbinds);
                    index = 0;
                } else {
                    let split = (non_repeaters as usize).min(requested.len());
                    let (once, repeated) = requested.split_at(split);
                    for oid in once {
                        varbinds.push(
                            self.next_after(oid)
                                .unwrap_or((oid.clone(), Value::EndOfMibView)),
                        );
                    }
                    let mut cursors = repeated.to_vec();
                    for _ in 0..max_repetitions {
                        for cursor in &mut cursors {
                            match self.next_after(cursor) {
                                Some((next, value)) => {
                                    *cursor = next.clone();
                                    varbinds.push((next, value));
                                }
                                None => varbinds.push((cursor.clone(), Value::EndOfMibView)),
                            }
                        }
                    }
                }
            }
        }
        Some(encode_response(
            if v1 { 0 } else { 1 },
            pdu.community,
            pdu.req_id,
            status,
            index,
            &varbinds,
        ))
    }
}

fn encode_response(
    version: i64,
    community: &[u8],
    request_id: i32,
    status: u32,
    index: u32,
    varbinds: &[(Oid, Value)],
) -> Vec<u8> {
    let mut list = Vec::new();
    for (oid, value) in varbinds {
        let mut pair = tlv(0x06, &crate::value::oid_octets(oid.arcs()));
        pair.extend(tlv(value.tag(), &value.content_octets()));
        list.extend(tlv(0x30, &pair));
    }
    let mut pdu = integer(i64::from(request_id));
    pdu.extend(integer(i64::from(status)));
    pdu.extend(integer(i64::from(index)));
    pdu.extend(tlv(0x30, &list));
    let mut message = integer(version);
    message.extend(tlv(0x04, community));
    message.extend(tlv(0xa2, &pdu));
    tlv(0x30, &message)
}

/// 25 objects below 1.3.6.1.4.1.99.1, then one more outside it.
fn objects() -> Vec<(Oid, Value)> {
    let mut objects: Vec<(Oid, Value)> = (1..=25)
        .map(|i| (oid(&format!("1.3.6.1.4.1.99.1.{i}.0")), Value::Integer(i)))
        .collect();
    objects.push((
        oid("1.3.6.1.4.1.99.2.0"),
        Value::OctetString(b"after".to_vec()),
    ));
    objects
}

fn target(address: SocketAddr, version: Version) -> Target {
    Target::new(address.ip().to_string(), address.port())
        .with_version(version)
        .with_timeout(Duration::from_millis(300))
        .with_retries(1)
}

/// Everything an operation produced.
#[derive(Debug, Default)]
struct Outcome {
    varbinds: Vec<Varbind>,
    log: Vec<crate::LogEntry>,
    notices: Vec<String>,
    #[allow(dead_code)]
    engines: Vec<crate::EngineInfo>,
    error: Option<Error>,
}

async fn run(target: Target, operation: Operation) -> Outcome {
    let engine = Engine::start(EngineConfig::default());
    collect(engine.submit(target, operation).unwrap()).await
}

async fn collect(mut running: crate::Running) -> Outcome {
    let mut outcome = Outcome::default();
    while let Some(event) = running.next_event().await {
        match event {
            Event::Varbinds(varbinds) => outcome.varbinds.extend(varbinds),
            Event::Log(entry) => outcome.log.push(entry),
            Event::Notice(text) => outcome.notices.push(text),
            Event::Engine(info) => outcome.engines.push(info),
            Event::Finished(summary) => {
                assert_eq!(summary.varbinds, outcome.varbinds.len());
                assert_eq!(summary.requests as usize, outcome.log.len());
            }
            Event::Failed(error) => outcome.error = Some(error),
        }
    }
    outcome
}

fn walk(root: &str, max_repetitions: u32) -> Operation {
    Operation::Walk {
        root: oid(root),
        max_repetitions,
        bulk: true,
    }
}

#[tokio::test]
async fn walk_with_getbulk() {
    let (address, agent) = fake_agent(objects(), Quirks::default()).await;
    let outcome = run(target(address, Version::V2c), walk("1.3.6.1.4.1.99.1", 10)).await;
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.varbinds.len(), 25);
    assert_eq!(outcome.varbinds[24].value, Value::Integer(25));
    // 10 + 10 + 5 (and the first object after the subtree ends the walk).
    assert_eq!(agent.requests(), ["GETBULK"; 3]);
    assert!(
        outcome.log[0]
            .request
            .starts_with("GETBULK 1.3.6.1.4.1.99.1 ")
    );
}

#[tokio::test]
async fn walk_with_getnext_in_snmpv1() {
    let (address, agent) = fake_agent(objects(), Quirks::default()).await;
    let outcome = run(target(address, Version::V1), walk("1.3.6.1.4.1.99", 10)).await;
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.varbinds.len(), 26);
    // One per object, and one more that ends with noSuchName.
    assert_eq!(agent.requests(), ["GETNEXT"; 27]);
}

#[tokio::test]
async fn walk_ends_at_end_of_mib_view() {
    let (address, _) = fake_agent(objects(), Quirks::default()).await;
    let outcome = run(target(address, Version::V2c), walk("1", 7)).await;
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.varbinds.len(), 26);
}

#[tokio::test]
async fn walk_falls_back_to_getnext() {
    let quirks = Quirks {
        bulk_error: Some(5),
        ..Quirks::default()
    };
    let (address, agent) = fake_agent(objects(), quirks).await;
    let outcome = run(target(address, Version::V2c), walk("1.3.6.1.4.1.99.1", 10)).await;
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.varbinds.len(), 25);
    assert_eq!(agent.requests()[0], "GETBULK");
    assert_eq!(agent.requests()[1..], ["GETNEXT"; 26]);
    assert_eq!(
        outcome.notices,
        ["GETBULK failed with genErr(5); continuing with GETNEXT"]
    );
}

#[tokio::test]
async fn walk_asks_for_less_when_too_big() {
    let quirks = Quirks {
        too_big_above: Some(5),
        ..Quirks::default()
    };
    let (address, agent) = fake_agent(objects(), quirks).await;
    let outcome = run(target(address, Version::V2c), walk("1.3.6.1.4.1.99.1", 20)).await;
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.varbinds.len(), 25);
    // 20 and 10 are too big; then 5 requests of 5, and one more that
    // finds the end of the subtree.
    assert_eq!(agent.requests().len(), 2 + 6);
    assert_eq!(outcome.notices.len(), 2);
}

#[tokio::test]
async fn walk_stops_when_oids_do_not_increase() {
    let quirks = Quirks {
        loop_at: Some(oid("1.3.6.1.4.1.99.1.3.0")),
        ..Quirks::default()
    };
    let (address, _) = fake_agent(objects(), quirks).await;
    let mut operation = walk("1.3.6.1.4.1.99.1", 10);
    if let Operation::Walk { bulk, .. } = &mut operation {
        *bulk = false;
    }
    let outcome = run(target(address, Version::V2c), operation).await;
    assert_eq!(outcome.varbinds.len(), 3);
    assert_eq!(
        outcome.error,
        Some(Error::NotIncreasing {
            previous: oid("1.3.6.1.4.1.99.1.3.0"),
            next: oid("1.3.6.1.4.1.99.1.1.0"),
        })
    );
}

#[tokio::test]
async fn walk_of_a_single_instance_uses_get() {
    let (address, agent) = fake_agent(objects(), Quirks::default()).await;
    let outcome = run(
        target(address, Version::V2c),
        walk("1.3.6.1.4.1.99.2.0", 10),
    )
    .await;
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.varbinds.len(), 1);
    assert_eq!(agent.requests(), ["GETBULK", "GET"]);
}

#[tokio::test]
async fn retries_after_lost_requests() {
    let quirks = Quirks {
        ignore_first: 1,
        ..Quirks::default()
    };
    let (address, _) = fake_agent(objects(), quirks).await;
    let get = Operation::Get(vec![oid("1.3.6.1.4.1.99.1.1.0")]);
    let outcome = run(target(address, Version::V2c), get).await;
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.varbinds[0].value, Value::Integer(1));
    assert_eq!(outcome.log.len(), 2);
    assert_eq!(outcome.log[0].result, "no response");
    assert_eq!(outcome.log[1].attempt, 2);
    assert_eq!(outcome.log[1].result, "1 varbind");
}

#[tokio::test]
async fn accepts_a_late_answer_to_the_first_try() {
    // Answers take 400 ms, the timeout is 300 ms: the answer to the first
    // try arrives during the retry, which uses the same request-id.
    let quirks = Quirks {
        delay: Duration::from_millis(400),
        ..Quirks::default()
    };
    let (address, _) = fake_agent(objects(), quirks).await;
    let get = Operation::Get(vec![oid("1.3.6.1.4.1.99.1.1.0")]);
    let outcome = run(target(address, Version::V2c), get).await;
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.varbinds.len(), 1);
    assert_eq!(outcome.log[0].result, "no response");
    assert_eq!(outcome.log[1].result, "1 varbind");
    assert_eq!(outcome.log[0].request_id, outcome.log[1].request_id);
}

#[tokio::test]
async fn gives_up_after_the_last_retry() {
    let quirks = Quirks {
        ignore_first: usize::MAX,
        ..Quirks::default()
    };
    let (address, agent) = fake_agent(objects(), quirks).await;
    let get = Operation::Get(vec![oid("1.3.6.1.4.1.99.1.1.0")]);
    let outcome = run(target(address, Version::V2c), get).await;
    assert!(
        matches!(outcome.error, Some(Error::Timeout { attempts: 2, .. })),
        "{:?}",
        outcome.error
    );
    assert_eq!(agent.requests().len(), 2);
}

#[tokio::test]
async fn snmpv1_errors_and_snmpv2_exceptions() {
    let (address, _) = fake_agent(objects(), Quirks::default()).await;
    let oids = vec![oid("1.3.6.1.4.1.99.1.1.0"), oid("1.3.6.1.4.1.99.9.0")];

    let v1 = run(target(address, Version::V1), Operation::Get(oids.clone())).await;
    match v1.error {
        Some(Error::Agent {
            status: ErrorStatus::NoSuchName,
            index: 2,
            oid: Some(missing),
            ..
        }) => assert_eq!(missing, oids[1]),
        other => panic!("expected noSuchName, got {other:?}"),
    }

    let v2 = run(target(address, Version::V2c), Operation::Get(oids)).await;
    assert_eq!(v2.error, None);
    assert_eq!(v2.varbinds[1].value, Value::NoSuchObject);
}

#[tokio::test]
async fn getnext_and_getbulk() {
    let (address, _) = fake_agent(objects(), Quirks::default()).await;
    let next = Operation::GetNext(vec![
        oid("1.3.6.1.4.1.99.1.1.0"),
        oid("1.3.6.1.4.1.99.1.25.0"),
    ]);
    let outcome = run(target(address, Version::V2c), next).await;
    let oids: Vec<String> = outcome.varbinds.iter().map(|v| v.oid.to_string()).collect();
    assert_eq!(oids, ["1.3.6.1.4.1.99.1.2.0", "1.3.6.1.4.1.99.2.0"]);

    let bulk = Operation::GetBulk {
        oids: vec![oid("1.3.6.1.4.1.99.1.1.0"), oid("1.3.6.1.4.1.99.1.10.0")],
        non_repeaters: 1,
        max_repetitions: 3,
    };
    let outcome = run(target(address, Version::V2c), bulk.clone()).await;
    let oids: Vec<String> = outcome.varbinds.iter().map(|v| v.oid.to_string()).collect();
    assert_eq!(
        oids,
        [
            "1.3.6.1.4.1.99.1.2.0",
            "1.3.6.1.4.1.99.1.11.0",
            "1.3.6.1.4.1.99.1.12.0",
            "1.3.6.1.4.1.99.1.13.0"
        ]
    );

    let outcome = run(target(address, Version::V1), bulk).await;
    assert_eq!(outcome.error, Some(Error::BulkNeedsV2));
}

#[tokio::test]
async fn limits_parallel_operations_per_agent() {
    let quirks = Quirks {
        delay: Duration::from_millis(100),
        ..Quirks::default()
    };
    let (address, _) = fake_agent(objects(), quirks).await;
    let get = || Operation::Get(vec![oid("1.3.6.1.4.1.99.1.1.0")]);

    for (limit, min, max) in [(1, 300, 10_000), (3, 100, 290)] {
        let engine = Engine::start(EngineConfig {
            per_target_limit: limit,
        });
        let started = Instant::now();
        let running: Vec<_> = (0..3)
            .map(|_| engine.submit(target(address, Version::V2c), get()).unwrap())
            .collect();
        for running in running {
            assert_eq!(collect(running).await.error, None);
        }
        let elapsed = started.elapsed().as_millis();
        assert!(
            (min..max).contains(&elapsed),
            "limit {limit}: 3 requests took {elapsed} ms"
        );
    }
}

#[tokio::test]
async fn unknown_host() {
    let target = Target::new("no-such-host.invalid", 161);
    let outcome = run(target, Operation::Get(vec![oid("1.3.6.1.2.1.1.1.0")])).await;
    assert!(
        matches!(outcome.error, Some(Error::Resolve { .. })),
        "{:?}",
        outcome.error
    );
}

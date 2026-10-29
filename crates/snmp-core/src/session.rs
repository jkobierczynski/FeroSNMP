//! One request and its response, with timeout and retries.
//!
//! Each session has its own UDP socket, connected to the agent so only its
//! answers arrive. A retry sends the same message again with the same
//! request-id, so a late answer to the first try is accepted too; answers to
//! earlier requests (late arrivals after a timeout) are skipped.
//!
//! SNMPv3 adds a step before the first request: discovering the agent's
//! engine ID, boots and time (RFC 3414, section 4). Both are cached per
//! agent for the engine's lifetime, with the keys derived from the
//! passwords. Every SNMPv3 retry is a new message (new msgID, current time,
//! new salt) for the same request-id.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mib_core::Oid;
use sha2::{Digest, Sha256};
use tokio::net::UdpSocket;

use crate::codec;
use crate::error::{Error, ErrorStatus, SecurityProblem};
use crate::target::{Target, Version};
use crate::usm::{Keys, SecurityLevel, hex};
use crate::v3::{self, DecodeError, EngineTime, Outgoing};
use crate::value::Varbind;

/// Largest UDP payload.
const MAX_MESSAGE: usize = 65_535;

/// A request PDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    Get(Vec<Oid>),
    GetNext(Oid),
    GetBulk {
        oids: Vec<Oid>,
        non_repeaters: u32,
        max_repetitions: u32,
    },
    Set(Vec<Varbind>),
}

impl Request {
    pub fn oids(&self) -> Vec<Oid> {
        match self {
            Request::Get(oids) | Request::GetBulk { oids, .. } => oids.clone(),
            Request::GetNext(oid) => vec![oid.clone()],
            Request::Set(varbinds) => varbinds.iter().map(|v| v.oid.clone()).collect(),
        }
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Request::Get(_) => "GET",
            Request::GetNext(_) => "GETNEXT",
            Request::GetBulk { .. } => "GETBULK",
            Request::Set(_) => "SET",
        };
        let oids = self.oids();
        write!(f, "{name}")?;
        for (i, oid) in oids.iter().enumerate() {
            write!(f, "{}{oid}", if i == 0 { " " } else { ", " })?;
        }
        if let Request::GetBulk {
            non_repeaters,
            max_repetitions,
            ..
        } = self
        {
            write!(
                f,
                " (non-repeaters {non_repeaters}, max-repetitions {max_repetitions})"
            )?;
        }
        Ok(())
    }
}

/// A decoded response PDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Response {
    pub status: ErrorStatus,
    pub index: u32,
    pub varbinds: Vec<Varbind>,
}

impl Response {
    /// The error for a response whose status is not `noError`.
    pub fn error(&self, target: &Target, requested: &[Oid]) -> Error {
        let oid = (self.index as usize)
            .checked_sub(1)
            .and_then(|i| requested.get(i))
            .cloned();
        Error::Agent {
            target: target.to_string(),
            status: self.status,
            index: self.index,
            oid,
        }
    }
}

/// One request sent to an agent, for the operation log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// The agent, as `host:port`.
    pub target: String,
    /// What was sent, such as `GETNEXT 1.3.6.1.2.1.1`.
    pub request: String,
    /// The request-id field.
    pub request_id: i32,
    /// 1 for the first try, 2 for the first retry, ...
    pub attempt: u32,
    /// Time from sending to the response (or to giving up).
    pub elapsed: Duration,
    /// What came back, such as `10 varbinds` or `no response`.
    pub result: String,
}

impl fmt::Display for LogEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} to {}, request-id {}",
            self.request, self.target, self.request_id
        )?;
        if self.attempt > 1 {
            write!(f, ", retry {}", self.attempt - 1)?;
        }
        write!(
            f,
            ": {} in {:.1} ms",
            self.result,
            self.elapsed.as_secs_f64() * 1000.0
        )
    }
}

/// An SNMPv3 agent's engine, as discovered: what to show the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineInfo {
    /// The agent, as `host:port`.
    pub target: String,
    /// The authoritative engine ID (`snmpEngineID`).
    pub engine_id: Vec<u8>,
    /// How often the engine has restarted (`snmpEngineBoots`).
    pub boots: u32,
    /// Seconds since the last restart (`snmpEngineTime`), when last heard.
    pub time: u32,
    /// Whether it was discovered just now rather than remembered from an
    /// earlier operation.
    pub fresh: bool,
}

impl fmt::Display for EngineInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: engine ID {}, boots {}, time {} s",
            self.target,
            hex(&self.engine_id),
            self.boots,
            self.time
        )
    }
}

/// Seconds as `3 d 4 h 5 min 6 s`, leaving out leading zero units: for
/// [`EngineInfo::time`].
pub fn uptime_text(seconds: u32) -> String {
    let parts = [
        (seconds / 86_400, "d"),
        (seconds / 3600 % 24, "h"),
        (seconds / 60 % 60, "min"),
        (seconds % 60, "s"),
    ];
    let first = parts.iter().position(|(n, _)| *n > 0).unwrap_or(3);
    parts[first..]
        .iter()
        .map(|(n, unit)| format!("{n} {unit}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// An engine's boots and time, and when we heard them.
#[derive(Debug, Clone)]
struct KnownEngine {
    engine: EngineTime,
    at: Instant,
}

impl KnownEngine {
    /// The agent's engine time now, counting on from when we heard it.
    fn now(&self) -> EngineTime {
        let elapsed = u32::try_from(self.at.elapsed().as_secs()).unwrap_or(u32::MAX);
        EngineTime {
            time: self
                .engine
                .time
                .saturating_add(elapsed)
                .min(i32::MAX as u32),
            ..self.engine.clone()
        }
    }
}

/// What SNMPv3 sessions remember between operations: engines by agent
/// address, and localized keys by a hash of what they are made from.
#[derive(Debug, Clone, Default)]
pub(crate) struct V3Cache {
    engines: Arc<Mutex<HashMap<SocketAddr, KnownEngine>>>,
    keys: Arc<Mutex<HashMap<[u8; 32], Keys>>>,
}

/// SNMPv3 state of a session.
#[derive(Debug)]
struct V3State {
    cache: V3Cache,
    engine: Option<KnownEngine>,
    keys: Option<Keys>,
}

/// A connection to one agent.
pub(crate) struct Session {
    target: Target,
    label: String,
    address: SocketAddr,
    socket: Option<UdpSocket>,
    buffer: Vec<u8>,
    next_request_id: i32,
    log: Vec<LogEntry>,
    v3: V3State,
    engine_info: Option<EngineInfo>,
}

/// What came back.
enum Answer {
    Community(Response),
    V3(v3::Received),
}

impl Session {
    /// Resolve the target's address. Nothing is sent yet. `cache` is the
    /// engine's SNMPv3 cache, shared by its sessions.
    pub async fn open(target: &Target, cache: V3Cache) -> Result<Session, Error> {
        let resolve_error = |message: String| Error::Resolve {
            host: target.host.clone(),
            message,
        };
        let address = tokio::net::lookup_host((target.host.as_str(), target.port))
            .await
            .map_err(|e| resolve_error(e.to_string()))?
            .next()
            .ok_or_else(|| resolve_error("no address found".to_string()))?;
        Ok(Session {
            label: target.to_string(),
            target: target.clone(),
            address,
            socket: None,
            buffer: Vec::new(),
            next_request_id: first_request_id(),
            log: Vec::new(),
            v3: V3State {
                cache,
                engine: None,
                keys: None,
            },
            engine_info: None,
        })
    }

    /// The SNMPv3 engine this session started using since the last call.
    pub fn take_engine_info(&mut self) -> Option<EngineInfo> {
        self.engine_info.take()
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The log entries written since the last call.
    pub fn take_log(&mut self) -> Vec<LogEntry> {
        std::mem::take(&mut self.log)
    }

    /// Send a request and wait for its response, retrying after timeouts.
    pub async fn request(&mut self, request: &Request) -> Result<Response, Error> {
        let request_id = self.take_request_id();
        if self.target.version == Version::V3 {
            return self.request_v3(request, request_id).await;
        }
        let message = codec::encode_request(
            self.target.version,
            &self.target.community,
            request_id,
            request,
        )?;
        match self
            .exchange(
                request,
                &request.to_string(),
                request_id,
                Some(&message),
                false,
            )
            .await?
        {
            Answer::Community(response) => Ok(response),
            Answer::V3(_) => unreachable!("community messages get community answers"),
        }
    }

    fn take_request_id(&mut self) -> i32 {
        let request_id = self.next_request_id;
        self.next_request_id = following_request_id(request_id);
        request_id
    }

    fn security_error(&self, problem: SecurityProblem) -> Error {
        Error::Security {
            target: self.label.clone(),
            user: self.target.usm.user.clone(),
            level: self.target.usm.level,
            problem,
        }
    }

    async fn request_v3(&mut self, request: &Request, request_id: i32) -> Result<Response, Error> {
        if let Err(reason) = self.target.usm.check() {
            return Err(self.security_error(SecurityProblem::InvalidSettings(reason)));
        }
        let mut synchronized = false;
        let mut rediscovered = false;
        loop {
            self.know_engine().await?;
            let answer = self
                .exchange(request, &request.to_string(), request_id, None, false)
                .await;
            let answer = match answer {
                Err(timeout @ Error::Timeout { .. }) if self.target.usm.level.has_priv() => {
                    return Err(self.diagnose_priv_timeout(timeout).await);
                }
                answer => answer?,
            };
            let Answer::V3(received) = answer else {
                unreachable!("SNMPv3 messages get SNMPv3 answers");
            };
            if !received.is_report() {
                if self.target.usm.level.has_auth() && !received.authenticated {
                    return Err(self.security_error(SecurityProblem::ResponseNotAuthenticated));
                }
                if received.authenticated {
                    self.heard(received.engine);
                }
                return Ok(received.response);
            }
            match v3::report_problem(&received.response) {
                // RFC 3414, 4: the Report carries the agent's boots and time;
                // send again with those.
                SecurityProblem::NotInTimeWindow if !synchronized => {
                    synchronized = true;
                    self.heard(received.engine);
                }
                // The agent's engine ID changed (a new device on the same
                // address, or a reconfigured one): discover it again.
                SecurityProblem::UnknownEngineId if !rediscovered => {
                    rediscovered = true;
                    self.forget_engine();
                }
                problem => return Err(self.security_error(problem)),
            }
        }
    }

    /// After an `authPriv` request got no answer: many agents, net-snmp
    /// among them, drop a request they cannot decrypt instead of sending a
    /// `usmStatsDecryptionErrors` Report. Ask once more without privacy;
    /// when that is answered, the privacy settings must be what is wrong.
    async fn diagnose_priv_timeout(&mut self, timeout: Error) -> Error {
        let level = self.target.usm.level;
        let retries = self.target.retries;
        self.target.usm.level = SecurityLevel::AuthNoPriv;
        self.target.retries = 0;
        let request_id = self.take_request_id();
        let probe = Request::Get(vec![snmp_engine_time()]);
        let answer = self
            .exchange(&probe, "check without privacy", request_id, None, false)
            .await;
        self.target.usm.level = level;
        self.target.retries = retries;
        match answer {
            Ok(Answer::V3(received)) if !received.is_report() && received.authenticated => {
                self.security_error(SecurityProblem::PrivacyMismatch)
            }
            Ok(Answer::V3(received)) if received.is_report() => {
                match v3::report_problem(&received.response) {
                    // The user cannot do authNoPriv; that says nothing
                    // about privacy.
                    SecurityProblem::UnsupportedSecurityLevel
                    | SecurityProblem::NotInTimeWindow => timeout,
                    problem => self.security_error(problem),
                }
            }
            Err(error @ Error::Security { .. }) => error,
            _ => timeout,
        }
    }

    /// Discover the agent's engine again, even when it is known.
    pub async fn discover(&mut self) -> Result<(), Error> {
        if self.target.version != Version::V3 {
            return Err(Error::DiscoveryNeedsV3);
        }
        if let Err(reason) = self.target.usm.check() {
            return Err(self.security_error(SecurityProblem::InvalidSettings(reason)));
        }
        self.forget_engine();
        self.know_engine().await
    }

    /// Discover the agent's engine unless it is known, and derive the keys.
    async fn know_engine(&mut self) -> Result<(), Error> {
        if self.v3.engine.is_none() {
            let cached = lock(&self.v3.cache.engines).get(&self.address).cloned();
            let fresh = cached.is_none();
            let known = match cached {
                Some(known) => known,
                None => {
                    let request_id = self.take_request_id();
                    let discovery = Request::Get(Vec::new());
                    let Answer::V3(received) = self
                        .exchange(&discovery, "discovery", request_id, None, true)
                        .await?
                    else {
                        unreachable!("SNMPv3 messages get SNMPv3 answers");
                    };
                    if received.engine.engine_id.is_empty() {
                        return Err(self.security_error(SecurityProblem::NoEngineId));
                    }
                    KnownEngine {
                        engine: received.engine,
                        at: Instant::now(),
                    }
                }
            };
            lock(&self.v3.cache.engines).insert(self.address, known.clone());
            self.engine_info = Some(EngineInfo {
                target: self.label.clone(),
                engine_id: known.engine.engine_id.clone(),
                boots: known.engine.boots,
                time: known.now().time,
                fresh,
            });
            self.v3.engine = Some(known);
            self.v3.keys = None;
        }
        if self.v3.keys.is_none() {
            let engine_id = &self
                .v3
                .engine
                .as_ref()
                .expect("known above")
                .engine
                .engine_id;
            let usm = &self.target.usm;
            let fingerprint: [u8; 32] = Sha256::new()
                .chain_update(engine_id)
                .chain_update([0])
                .chain_update(format!(
                    "{:?}",
                    (usm.level, usm.auth_protocol, usm.priv_protocol)
                ))
                .chain_update([0])
                .chain_update(&usm.auth_password)
                .chain_update([0])
                .chain_update(usm.effective_priv_password())
                .finalize()
                .into();
            let cached = lock(&self.v3.cache.keys).get(&fingerprint).cloned();
            let keys = cached.unwrap_or_else(|| Keys::derive(usm, engine_id));
            lock(&self.v3.cache.keys).insert(fingerprint, keys.clone());
            self.v3.keys = Some(keys);
        }
        Ok(())
    }

    /// Take the boots and time from a message of the agent's own engine.
    fn heard(&mut self, engine: EngineTime) {
        let Some(known) = &self.v3.engine else {
            return;
        };
        if engine.engine_id != known.engine.engine_id {
            return;
        }
        let known = KnownEngine {
            engine,
            at: Instant::now(),
        };
        lock(&self.v3.cache.engines).insert(self.address, known.clone());
        self.v3.engine = Some(known);
    }

    fn forget_engine(&mut self) {
        lock(&self.v3.cache.engines).remove(&self.address);
        self.v3.engine = None;
        self.v3.keys = None;
    }

    /// Send a message and wait for its answer, retrying after timeouts.
    /// `message` is the encoded community message; for SNMPv3 (`None`) a
    /// new message is built for every attempt. `discovery` sends the
    /// SNMPv3 discovery message.
    async fn exchange(
        &mut self,
        request: &Request,
        what: &str,
        request_id: i32,
        message: Option<&[u8]>,
        discovery: bool,
    ) -> Result<Answer, Error> {
        if self.socket.is_none() {
            self.socket = Some(self.connect().await?);
            self.buffer = vec![0; MAX_MESSAGE];
        }
        let socket = self.socket.take().expect("connected above");
        let mut buffer = std::mem::take(&mut self.buffer);
        let result = self
            .exchange_on(
                &socket,
                &mut buffer,
                request,
                what,
                request_id,
                message,
                discovery,
            )
            .await;
        self.socket = Some(socket);
        self.buffer = buffer;
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn exchange_on(
        &mut self,
        socket: &UdpSocket,
        buffer: &mut [u8],
        request: &Request,
        what: &str,
        request_id: i32,
        message: Option<&[u8]>,
        discovery: bool,
    ) -> Result<Answer, Error> {
        let network_error = |label: &str, error: io::Error| Error::Network {
            target: label.to_string(),
            message: describe_io_error(&error),
        };
        let mut msg_ids = Vec::new();
        let attempts = self.target.retries.saturating_add(1);
        for attempt in 1..=attempts {
            let bytes = match message {
                Some(bytes) => bytes.to_vec(),
                None => {
                    let msg_id = next_msg_id();
                    msg_ids.push(msg_id);
                    let engine = (!discovery)
                        .then(|| self.v3.engine.as_ref().map(KnownEngine::now))
                        .flatten();
                    let outgoing = Outgoing {
                        msg_id,
                        request_id,
                        request,
                        engine: engine.as_ref(),
                        salt: next_salt(),
                    };
                    v3::encode_request(&self.target.usm, self.v3.keys.as_ref(), &outgoing)?
                }
            };
            let started = Instant::now();
            let label = self.label.clone();
            let log = |log: &mut Vec<LogEntry>, result: String| {
                log.push(LogEntry {
                    target: label.clone(),
                    request: what.to_string(),
                    request_id,
                    attempt,
                    elapsed: started.elapsed(),
                    result,
                });
            };
            if let Err(error) = socket.send(&bytes).await {
                let error = network_error(&self.label, error);
                log(&mut self.log, error.to_string());
                return Err(error);
            }
            let deadline = tokio::time::Instant::from_std(started + self.target.timeout);
            loop {
                let received = tokio::time::timeout_at(deadline, socket.recv(buffer)).await;
                let len = match received {
                    Err(_elapsed) => {
                        log(&mut self.log, "no response".to_string());
                        break;
                    }
                    Ok(Err(error)) => {
                        let error = network_error(&self.label, error);
                        log(&mut self.log, error.to_string());
                        return Err(error);
                    }
                    Ok(Ok(len)) => len,
                };
                let decoded = match message {
                    Some(_) => match codec::decode_response(&buffer[..len]) {
                        Ok((id, response)) if id == request_id => {
                            Ok(Some(Answer::Community(response)))
                        }
                        // A late answer to an earlier request: keep waiting.
                        Ok(_) => Ok(None),
                        Err(message) => Err(Error::Decode {
                            target: self.label.clone(),
                            message,
                        }),
                    },
                    None => {
                        let keys = (!discovery).then_some(self.v3.keys.as_ref()).flatten();
                        match v3::decode_received(&buffer[..len], &self.target.usm, keys, &msg_ids)
                        {
                            // A Response must answer this request; a Report
                            // may not carry the request-id.
                            Ok(Some(received))
                                if received.is_report() || received.request_id == request_id =>
                            {
                                Ok(Some(Answer::V3(received)))
                            }
                            Ok(_) => Ok(None),
                            Err(DecodeError::Malformed(message)) => Err(Error::Decode {
                                target: self.label.clone(),
                                message,
                            }),
                            Err(DecodeError::Security(problem)) => {
                                Err(self.security_error(problem))
                            }
                        }
                    }
                };
                match decoded {
                    Ok(Some(answer)) => {
                        log(&mut self.log, answer_summary(&answer));
                        return Ok(answer);
                    }
                    Ok(None) => continue,
                    Err(error) => {
                        log(&mut self.log, error.to_string());
                        return Err(error);
                    }
                }
            }
        }
        Err(Error::Timeout {
            target: self.label.clone(),
            attempts,
            timeout: self.target.timeout,
        })
    }

    async fn connect(&self) -> Result<UdpSocket, Error> {
        let local: SocketAddr = match self.address {
            SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
            SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
        };
        let socket = UdpSocket::bind(local).await;
        let socket = match socket {
            Ok(socket) => socket.connect(self.address).await.map(|()| socket),
            Err(error) => Err(error),
        };
        socket.map_err(|error| Error::Network {
            target: self.label.clone(),
            message: format!("cannot open a UDP socket: {error}"),
        })
    }
}

/// Explain a socket error. On a connected UDP socket, an ICMP message from
/// the host ("port unreachable") shows up as an error on the next receive.
fn describe_io_error(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset => format!(
            "the host refused the request; is an SNMP agent listening on that port? ({error})"
        ),
        io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable => {
            format!("cannot reach the host ({error})")
        }
        _ => error.to_string(),
    }
}

fn answer_summary(answer: &Answer) -> String {
    match answer {
        Answer::Community(response) => response_summary(response),
        Answer::V3(received) if received.is_report() => {
            let problem = v3::report_problem(&received.response);
            match (problem.counter(), received.response.varbinds.first()) {
                (Some(counter), _) => format!("Report {counter}"),
                (None, Some(varbind)) => format!("Report {}", varbind.oid),
                (None, None) => "empty Report".to_string(),
            }
        }
        Answer::V3(received) => response_summary(&received.response),
    }
}

/// `snmpEngineTime.0`, which every SNMPv3 agent has.
fn snmp_engine_time() -> Oid {
    Oid::new(vec![1, 3, 6, 1, 6, 3, 10, 2, 1, 3, 0])
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// msgIDs and privacy salts are counted per process, from a start that
/// differs between runs, so no two messages share one.
fn next_msg_id() -> i32 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let start = u64::from(first_request_id().unsigned_abs());
    static START: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let start = *START.get_or_init(|| start);
    ((start + n) % 0x7fff_fff0) as i32 + 1
}

fn next_salt() -> u64 {
    static START: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let start = *START.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        nanos ^ u64::from(std::process::id()).rotate_left(40)
    });
    start.wrapping_add(NEXT.fetch_add(1, Ordering::Relaxed))
}

fn response_summary(response: &Response) -> String {
    let count = response.varbinds.len();
    let varbinds = format!("{count} varbind{}", if count == 1 { "" } else { "s" });
    match response.status {
        ErrorStatus::NoError => varbinds,
        status => format!("{status}, error index {}", response.index),
    }
}

/// A request-id to start with that differs between runs, so a late answer
/// to an earlier run of the program is not taken for an answer.
fn first_request_id() -> i32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
        .unwrap_or(0);
    let mixed = nanos ^ std::process::id().rotate_left(16);
    (mixed & 0x3fff_ffff) as i32 + 1
}

fn following_request_id(id: i32) -> i32 {
    if id >= i32::MAX - 1 { 1 } else { id + 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_text() {
        let bulk = Request::GetBulk {
            oids: vec!["1.3.6.1.2.1.1".parse().unwrap()],
            non_repeaters: 0,
            max_repetitions: 10,
        };
        assert_eq!(
            bulk.to_string(),
            "GETBULK 1.3.6.1.2.1.1 (non-repeaters 0, max-repetitions 10)"
        );
    }

    #[test]
    fn uptime() {
        assert_eq!(uptime_text(0), "0 s");
        assert_eq!(uptime_text(61), "1 min 1 s");
        assert_eq!(uptime_text(90_061), "1 d 1 h 1 min 1 s");
    }

    #[test]
    fn request_ids_stay_positive() {
        assert_eq!(following_request_id(i32::MAX - 1), 1);
        assert!(first_request_id() > 0);
    }
}

//! One request and its response, with timeout and retries.
//!
//! Each session has its own UDP socket, connected to the agent so only its
//! answers arrive. A retry sends the same message again with the same
//! request-id, so a late answer to the first try is accepted too; answers to
//! earlier requests (late arrivals after a timeout) are skipped.

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mib_core::Oid;
use tokio::net::UdpSocket;

use crate::codec;
use crate::error::{Error, ErrorStatus};
use crate::target::Target;
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
}

impl Request {
    pub fn oids(&self) -> &[Oid] {
        match self {
            Request::Get(oids) | Request::GetBulk { oids, .. } => oids,
            Request::GetNext(oid) => std::slice::from_ref(oid),
        }
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, oids) = match self {
            Request::Get(oids) => ("GET", oids.as_slice()),
            Request::GetNext(oid) => ("GETNEXT", std::slice::from_ref(oid)),
            Request::GetBulk { oids, .. } => ("GETBULK", oids.as_slice()),
        };
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

/// A connection to one agent.
pub(crate) struct Session {
    target: Target,
    label: String,
    address: SocketAddr,
    socket: Option<UdpSocket>,
    buffer: Vec<u8>,
    next_request_id: i32,
    log: Vec<LogEntry>,
}

impl Session {
    /// Resolve the target's address. Nothing is sent yet.
    pub async fn open(target: &Target) -> Result<Session, Error> {
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
        })
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
        let request_id = self.next_request_id;
        self.next_request_id = following_request_id(request_id);
        let message = codec::encode_request(
            self.target.version,
            &self.target.community,
            request_id,
            request,
        )?;
        if self.socket.is_none() {
            self.socket = Some(self.connect().await?);
            self.buffer = vec![0; MAX_MESSAGE];
        }
        let socket = self.socket.as_ref().expect("connected above");
        let network_error = |error: io::Error| Error::Network {
            target: self.label.clone(),
            message: describe_io_error(&error),
        };

        let attempts = self.target.retries.saturating_add(1);
        for attempt in 1..=attempts {
            let started = Instant::now();
            let mut log = |result: String| {
                self.log.push(LogEntry {
                    target: self.label.clone(),
                    request: request.to_string(),
                    request_id,
                    attempt,
                    elapsed: started.elapsed(),
                    result,
                });
            };
            if let Err(error) = socket.send(&message).await {
                let error = network_error(error);
                log(error.to_string());
                return Err(error);
            }
            let deadline = tokio::time::Instant::from_std(started + self.target.timeout);
            loop {
                let received =
                    tokio::time::timeout_at(deadline, socket.recv(&mut self.buffer)).await;
                let len = match received {
                    Err(_elapsed) => {
                        log("no response".to_string());
                        break;
                    }
                    Ok(Err(error)) => {
                        let error = network_error(error);
                        log(error.to_string());
                        return Err(error);
                    }
                    Ok(Ok(len)) => len,
                };
                match codec::decode_response(&self.buffer[..len]) {
                    Ok((id, response)) if id == request_id => {
                        log(response_summary(&response));
                        return Ok(response);
                    }
                    // A late answer to an earlier request: keep waiting.
                    Ok(_) => continue,
                    Err(message) => {
                        let error = Error::Decode {
                            target: self.label.clone(),
                            message,
                        };
                        log(error.to_string());
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
    fn request_ids_stay_positive() {
        assert_eq!(following_request_id(i32::MAX - 1), 1);
        assert!(first_request_id() > 0);
    }
}

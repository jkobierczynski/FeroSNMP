//! The UDP listener: a tokio task that reads datagrams, answers INFORMs and
//! SNMPv3 discovery, and hands what it received to a channel.

use std::fmt;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, SystemTime};

use snmp_core::{Notification, NotificationReceiver, ReceiveError};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// The standard port for notifications.
pub const TRAP_PORT: u16 = 162;

/// An unprivileged port to use instead of 162, as net-snmp's tests do.
pub const UNPRIVILEGED_PORT: u16 = 1162;

/// How many received notifications may wait in the channel. When the reader
/// falls behind, the listener waits rather than dropping any.
const CHANNEL_SIZE: usize = 1024;

/// What the listener received from one datagram.
#[derive(Debug, Clone)]
pub enum Incoming {
    /// A trap or INFORM.
    Notification(Received),
    /// A datagram that was not a notification, or failed a security check.
    Problem(Problem),
}

/// A notification and where and when it came from.
#[derive(Debug, Clone)]
pub struct Received {
    /// When the datagram arrived.
    pub at: SystemTime,
    /// The sender's address and port.
    pub source: SocketAddr,
    /// The notification.
    pub notification: Notification,
    /// For an INFORM: whether the Response was sent.
    pub acknowledged: bool,
}

/// A datagram that did not give a notification.
#[derive(Debug, Clone)]
pub struct Problem {
    /// When the datagram arrived.
    pub at: SystemTime,
    /// The sender's address and port.
    pub source: SocketAddr,
    /// What was wrong.
    pub error: ReceiveError,
}

/// Why the listener could not open its port.
#[derive(Debug)]
pub struct BindError {
    /// The address and port asked for.
    pub address: SocketAddr,
    /// What the operating system said.
    pub error: io::Error,
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot listen on UDP {}: {}", self.address, self.error)
    }
}

impl std::error::Error for BindError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl BindError {
    /// What to do about it, when the cause is a known one.
    pub fn hint(&self) -> Option<String> {
        bind_hint(self.address.port(), self.error.kind(), cfg!(windows))
    }
}

/// The hint for a failed bind; `windows` chooses the platform, so both can
/// be tested anywhere.
fn bind_hint(port: u16, kind: io::ErrorKind, windows: bool) -> Option<String> {
    let alternative = format!(
        "or listen on an unprivileged port such as {UNPRIVILEGED_PORT} and send the \
         notifications there"
    );
    match kind {
        io::ErrorKind::AddrInUse if port == TRAP_PORT && windows => Some(format!(
            "another program is already listening on UDP port 162. On Windows this is \
             usually the SNMP Trap service (SNMPTRAP). Stop it in a PowerShell run as \
             administrator with `Stop-Service SNMPTRAP` (and `Set-Service SNMPTRAP \
             -StartupType Disabled` to keep it stopped after a restart), {alternative}"
        )),
        io::ErrorKind::AddrInUse if port == TRAP_PORT => Some(format!(
            "another program is already listening on UDP port 162, usually snmptrapd: stop \
             it with `sudo systemctl stop snmptrapd`, {alternative}"
        )),
        io::ErrorKind::AddrInUse => Some(format!(
            "another program is already listening on UDP port {port}; stop it, or choose \
             another port"
        )),
        io::ErrorKind::PermissionDenied if windows => Some(format!(
            "Windows refused the port. Another program may hold it exclusively (for port \
             162, the SNMP Trap service: `Stop-Service SNMPTRAP` as administrator), or it is \
             in a range Windows reserves (see `netsh int ipv4 show excludedportrange \
             protocol=udp`); {alternative}"
        )),
        io::ErrorKind::PermissionDenied if port < 1024 => Some(format!(
            "ports below 1024 need extra rights on Linux and macOS. On Linux, allow \
             ferosnmp to use them once with `sudo setcap cap_net_bind_service=+ep \
             \"$(command -v ferosnmp)\"` (again after every update of the program), \
             {alternative}"
        )),
        _ => None,
    }
}

/// An open UDP socket for notifications, not yet listening.
#[derive(Debug)]
pub struct Listener {
    socket: UdpSocket,
    receiver: NotificationReceiver,
}

impl Listener {
    /// Open the port. This does not need a tokio runtime; [`spawn`]
    /// does.
    ///
    /// [`spawn`]: Listener::spawn
    pub fn bind(
        address: SocketAddr,
        receiver: NotificationReceiver,
    ) -> Result<Listener, BindError> {
        let socket = UdpSocket::bind(address).map_err(|error| BindError { address, error })?;
        socket
            .set_nonblocking(true)
            .map_err(|error| BindError { address, error })?;
        Ok(Listener { socket, receiver })
    }

    /// The address and port the socket is bound to (with the real port when
    /// port 0 was asked for).
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// The receiver's SNMPv3 engine ID.
    pub fn engine_id(&self) -> &[u8] {
        self.receiver.engine_id()
    }

    /// Start listening, as a task on the current tokio runtime. Must be
    /// called from within a runtime.
    pub fn spawn(self) -> io::Result<Traps> {
        let socket = tokio::net::UdpSocket::from_std(self.socket)?;
        let (sender, incoming) = mpsc::channel(CHANNEL_SIZE);
        let task = tokio::spawn(listen(socket, self.receiver, sender));
        Ok(Traps { incoming, task })
    }
}

/// A running listener. Dropping it stops the listener.
#[derive(Debug)]
pub struct Traps {
    incoming: mpsc::Receiver<Incoming>,
    task: JoinHandle<()>,
}

impl Traps {
    /// The next notification or problem; `None` once the listener stopped.
    pub async fn next(&mut self) -> Option<Incoming> {
        self.incoming.recv().await
    }

    /// The same from a thread outside the runtime, such as a window's.
    pub fn blocking_next(&mut self) -> Option<Incoming> {
        self.incoming.blocking_recv()
    }

    /// What has arrived already, without waiting.
    pub fn try_next(&mut self) -> Option<Incoming> {
        self.incoming.try_recv().ok()
    }
}

impl Drop for Traps {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn listen(
    socket: tokio::net::UdpSocket,
    mut receiver: NotificationReceiver,
    sender: mpsc::Sender<Incoming>,
) {
    // The largest UDP datagram.
    let mut buffer = vec![0u8; 65_535];
    loop {
        let (length, source) = match socket.recv_from(&mut buffer).await {
            Ok(received) => received,
            // Windows reports an ICMP "port unreachable" for an earlier
            // reply (a sender that gave up on an INFORM) as an error of the
            // next receive. It says nothing about this socket.
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => continue,
            Err(_) => {
                // Should not happen; do not spin if it keeps happening.
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let at = SystemTime::now();
        let handled = receiver.handle(&buffer[..length]);
        let acknowledged = match &handled.reply {
            Some(reply) => socket.send_to(reply, source).await.is_ok(),
            None => false,
        };
        let incoming = match handled.result {
            Ok(Some(notification)) => Incoming::Notification(Received {
                at,
                source,
                notification,
                acknowledged,
            }),
            // Discovery, answered.
            Ok(None) => continue,
            Err(error) => Incoming::Problem(Problem { at, source, error }),
        };
        if sender.send(incoming).await.is_err() {
            // Nobody reads any more.
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_name_the_usual_causes() {
        let hint = |port, kind, windows| bind_hint(port, kind, windows).unwrap();
        let windows = hint(162, io::ErrorKind::AddrInUse, true);
        assert!(
            windows.contains("SNMPTRAP") && windows.contains("1162"),
            "{windows}"
        );
        let linux = hint(162, io::ErrorKind::AddrInUse, false);
        assert!(linux.contains("snmptrapd"), "{linux}");
        let setcap = hint(162, io::ErrorKind::PermissionDenied, false);
        assert!(setcap.contains("setcap cap_net_bind_service"), "{setcap}");
        let excluded = hint(162, io::ErrorKind::PermissionDenied, true);
        assert!(excluded.contains("excludedportrange"), "{excluded}");
        assert!(bind_hint(5000, io::ErrorKind::PermissionDenied, false).is_none());
        assert!(bind_hint(162, io::ErrorKind::Other, false).is_none());
    }

    #[test]
    fn a_port_in_use_is_reported() {
        let first = Listener::bind(
            "127.0.0.1:0".parse().unwrap(),
            NotificationReceiver::new(vec![]),
        )
        .unwrap();
        let address = first.local_addr().unwrap();
        let error = Listener::bind(address, NotificationReceiver::new(vec![])).unwrap_err();
        assert_eq!(error.address, address);
        assert!(
            error
                .to_string()
                .starts_with("cannot listen on UDP 127.0.0.1:")
        );
    }
}

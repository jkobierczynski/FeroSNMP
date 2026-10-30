//! Running SNMP queries for the window without ever blocking it.
//!
//! The window has one UI thread, which must keep handling messages or Windows
//! shows "Not responding". So the SNMP engine runs on a thread of its own,
//! inside one tokio runtime: the [`QueryWorker`]. The window submits an
//! operation and gets a [`Query`] back at once.
//!
//! Results come back through an inbox: the worker puts events in it and
//! calls `notify` (in the window: post a message to the window) only when
//! the inbox was empty. The window then takes everything that has arrived
//! in one go. However fast a walk streams, at most one message is waiting
//! in the window's queue, and the window handles results in batches.
//!
//! The trap receiver of the Traps tab runs on the same thread:
//! [`QueryWorker::listen`] starts a [`Listener`] there and hands back a
//! [`TrapFeed`] with the same kind of inbox.

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use snmp_core::{Engine, EngineConfig, Error, Event, Operation, Target};
use tokio::runtime::{Builder, Handle};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use trap_rx::{Incoming, Listener};

/// The thread with the tokio runtime and the SNMP engine. Stops when
/// dropped; running queries are cancelled then.
#[derive(Debug)]
pub struct QueryWorker {
    runtime: Handle,
    engine: Engine,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl QueryWorker {
    /// Start the worker thread.
    pub fn start() -> std::io::Result<QueryWorker> {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let handle = runtime.handle().clone();
        let (stop, stopped) = oneshot::channel::<()>();
        let thread = thread::Builder::new()
            .name("snmp-engine".to_string())
            .spawn(move || {
                // Runs every task until the worker is dropped (or the
                // sender is dropped, which also ends the wait).
                runtime.block_on(async {
                    let _ = stopped.await;
                });
            })?;
        // The engine spawns its service task on the runtime; entering it
        // from this thread queues the task for the worker thread.
        let engine = {
            let _entered = handle.enter();
            Engine::start(EngineConfig::default())
        };
        Ok(QueryWorker {
            runtime: handle,
            engine,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    /// Start an operation. `notify` is called on the worker thread when
    /// events are waiting in the query's inbox that were not there before.
    pub fn submit(
        &self,
        target: Target,
        operation: Operation,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Result<Query, Error> {
        let mut running = self.engine.submit(target, operation)?;
        let inbox = Arc::new(Mutex::new(Inbox::default()));
        let (cancel, mut cancelled) = oneshot::channel::<()>();
        let shared = inbox.clone();
        self.runtime.spawn(async move {
            let deliver = |event: Event| {
                let mut inbox = lock(&shared);
                inbox.events.push(event);
                if !inbox.waiting {
                    inbox.waiting = true;
                    drop(inbox);
                    notify();
                }
            };
            loop {
                tokio::select! {
                    event = running.next_event() => match event {
                        Some(event) => {
                            let last = matches!(event, Event::Finished(_) | Event::Failed(_));
                            deliver(event);
                            if last {
                                break;
                            }
                        }
                        // The engine always sends a last event; be safe.
                        None => break,
                    },
                    // Cancel was pressed, or the query was dropped. Dropping
                    // `running` stops the engine at once, even mid-request.
                    _ = &mut cancelled => {
                        drop(running);
                        deliver(Event::Failed(Error::Cancelled));
                        break;
                    }
                }
            }
        });
        Ok(Query {
            inbox,
            cancel: Some(cancel),
        })
    }
}

impl QueryWorker {
    /// Start receiving notifications on `listener`'s port. `notify` is
    /// called on the worker thread when notifications are waiting in the
    /// feed that were not there before. Dropping the feed closes the port.
    pub fn listen(
        &self,
        listener: Listener,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<TrapFeed> {
        let traps = {
            // The listener's task and socket belong to the worker's runtime.
            let _entered = self.runtime.enter();
            listener.spawn()?
        };
        let inbox = Arc::new(Mutex::new(TrapInbox::default()));
        let shared = inbox.clone();
        let task = self.runtime.spawn(async move {
            let mut traps = traps;
            while let Some(incoming) = traps.next().await {
                let mut inbox = lock(&shared);
                if inbox.incoming.len() == TRAP_INBOX_LIMIT {
                    // The window has not looked for a long time; it keeps
                    // only the newest ones anyway.
                    inbox.incoming.pop_front();
                    inbox.dropped += 1;
                }
                inbox.incoming.push_back(incoming);
                if !inbox.waiting {
                    inbox.waiting = true;
                    drop(inbox);
                    notify();
                }
            }
        });
        Ok(TrapFeed { inbox, task })
    }
}

/// How many notifications wait for the window at most: as many as the
/// Traps tab keeps.
const TRAP_INBOX_LIMIT: usize = crate::traps::LIMIT;

#[derive(Debug, Default)]
struct TrapInbox {
    incoming: std::collections::VecDeque<Incoming>,
    /// Notifications dropped because the inbox was full.
    dropped: u64,
    /// As in [`Inbox`].
    waiting: bool,
}

/// A running trap receiver. Dropping it stops the receiver and closes its
/// port.
#[derive(Debug)]
pub struct TrapFeed {
    inbox: Arc<Mutex<TrapInbox>>,
    task: JoinHandle<()>,
}

impl TrapFeed {
    /// Everything received since the last call, in order, and how many
    /// notifications had to be dropped because the window did not take
    /// them in time.
    pub fn take(&self) -> (Vec<Incoming>, u64) {
        let mut inbox = lock(&self.inbox);
        inbox.waiting = false;
        (
            std::mem::take(&mut inbox.incoming).into(),
            std::mem::take(&mut inbox.dropped),
        )
    }
}

impl Drop for TrapFeed {
    fn drop(&mut self) {
        // Aborting the task drops the listener, which stops its own task
        // and closes the socket.
        self.task.abort();
    }
}

impl Drop for QueryWorker {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Debug, Default)]
struct Inbox {
    events: Vec<Event>,
    /// True from the first event after a [`Query::take`] until the next
    /// take: `notify` was called and the window has not looked yet.
    waiting: bool,
}

fn lock<T>(inbox: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding the lock leaves the events intact.
    inbox
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A submitted operation. Dropping it cancels the operation.
#[derive(Debug)]
pub struct Query {
    inbox: Arc<Mutex<Inbox>>,
    cancel: Option<oneshot::Sender<()>>,
}

impl Query {
    /// Every event that arrived since the last call, in order. The last
    /// event of a query is [`Event::Finished`] or [`Event::Failed`]
    /// (with [`Error::Cancelled`] after [`cancel`](Self::cancel)).
    pub fn take(&self) -> Vec<Event> {
        let mut inbox = lock(&self.inbox);
        inbox.waiting = false;
        std::mem::take(&mut inbox.events)
    }

    /// Stop the operation. Results already received stay valid; a
    /// `Failed(Cancelled)` event follows.
    pub fn cancel(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Collect a query's events until its last one, as the window does.
    fn collect(query: &Query, wake: &mpsc::Receiver<()>) -> Vec<Event> {
        let mut events = Vec::new();
        loop {
            wake.recv_timeout(Duration::from_secs(30))
                .expect("the worker notifies");
            for event in query.take() {
                let last = matches!(event, Event::Finished(_) | Event::Failed(_));
                events.push(event);
                if last {
                    return events;
                }
            }
        }
    }

    fn notifier() -> (impl Fn() + Send + Sync + 'static, mpsc::Receiver<()>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        (move || sender.lock().unwrap().send(()).unwrap(), receiver)
    }

    /// An agent that never answers: a bound socket that reads nothing.
    fn silent_agent() -> (std::net::UdpSocket, Target) {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        let target = Target::new("127.0.0.1", port)
            .with_timeout(Duration::from_millis(100))
            .with_retries(1);
        (socket, target)
    }

    #[test]
    fn timeouts_arrive_as_events() {
        let worker = QueryWorker::start().unwrap();
        let (_socket, target) = silent_agent();
        let (notify, wake) = notifier();
        let query = worker
            .submit(
                target,
                Operation::Get(vec!["1.3.6.1.2.1.1.1.0".parse().unwrap()]),
                notify,
            )
            .unwrap();
        let events = collect(&query, &wake);
        let logs = events.iter().filter(|e| matches!(e, Event::Log(_))).count();
        assert_eq!(logs, 2, "the first try and one retry: {events:?}");
        assert!(
            matches!(
                events.last(),
                Some(Event::Failed(Error::Timeout { attempts: 2, .. }))
            ),
            "{events:?}"
        );
    }

    #[test]
    fn cancel_stops_at_once() {
        let worker = QueryWorker::start().unwrap();
        let (_socket, target) = silent_agent();
        let target = target.with_timeout(Duration::from_secs(60));
        let (notify, wake) = notifier();
        let mut query = worker
            .submit(
                target,
                Operation::walk("1.3.6.1.2.1".parse().unwrap()),
                notify,
            )
            .unwrap();
        query.cancel();
        let events = collect(&query, &wake);
        assert!(
            matches!(events.last(), Some(Event::Failed(Error::Cancelled))),
            "{events:?}"
        );
    }

    #[test]
    fn many_queries_share_the_worker() {
        let worker = QueryWorker::start().unwrap();
        let (_socket, target) = silent_agent();
        let target = target.with_retries(0);
        let queries: Vec<_> = (0..8)
            .map(|_| {
                let (notify, wake) = notifier();
                let query = worker
                    .submit(
                        target.clone(),
                        Operation::Get(vec!["1.3.6.1".parse().unwrap()]),
                        notify,
                    )
                    .unwrap();
                (query, wake)
            })
            .collect();
        for (query, wake) in &queries {
            let events = collect(query, wake);
            assert!(
                matches!(events.last(), Some(Event::Failed(_))),
                "{events:?}"
            );
        }
        drop(queries);
        drop(worker); // stops the thread without hanging
    }

    #[test]
    fn traps_arrive_in_the_feed() {
        use snmp_core::NotificationReceiver;
        let worker = QueryWorker::start().unwrap();
        let listener = Listener::bind(
            "127.0.0.1:0".parse().unwrap(),
            NotificationReceiver::new(vec![]),
        )
        .unwrap();
        let address = listener.local_addr().unwrap();
        let (notify, wake) = notifier();
        let feed = worker.listen(listener, notify).unwrap();
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let datagram = crate::test_support::recorded("v2c-port-change");
        sender.send_to(&datagram, address).unwrap();
        sender.send_to(b"not SNMP", address).unwrap();
        let mut incoming = Vec::new();
        while incoming.len() < 2 {
            wake.recv_timeout(Duration::from_secs(30))
                .expect("the worker notifies");
            incoming.extend(feed.take().0);
        }
        assert!(
            matches!(incoming[0], Incoming::Notification(_)),
            "{incoming:?}"
        );
        assert!(matches!(incoming[1], Incoming::Problem(_)), "{incoming:?}");

        // Dropping the feed closes the port, so it can be opened again.
        drop(feed);
        let again = (0..50).find_map(|_| {
            std::thread::sleep(Duration::from_millis(20));
            Listener::bind(address, NotificationReceiver::new(vec![])).ok()
        });
        assert!(again.is_some(), "the port was not closed");
    }
}

//! The engine: a tokio service that runs SNMP operations and streams their
//! results back over channels.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mib_core::Oid;
use tokio::sync::{Semaphore, mpsc};

use crate::error::{Error, ErrorStatus};
use crate::session::{EngineInfo, LogEntry, Request, Response, Session, V3Cache};
use crate::target::{Target, Version};
use crate::value::{Value, Varbind};

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// GET: the values of these instances, in one request.
    Get(Vec<Oid>),
    /// GETNEXT: for each OID, the first instance after it (one request per
    /// OID).
    GetNext(Vec<Oid>),
    /// GETBULK (SNMPv2c): the first `non_repeaters` OIDs once, like GETNEXT,
    /// and up to `max_repetitions` successors of each of the others.
    GetBulk {
        /// Starting OIDs.
        oids: Vec<Oid>,
        /// How many of `oids` (from the front) get only one successor.
        non_repeaters: u32,
        /// How many successors of each of the other OIDs.
        max_repetitions: u32,
    },
    /// Every instance below `root`, in OID order: GETBULK requests with
    /// SNMPv2c (unless `bulk` is false), GETNEXT requests with SNMPv1 or
    /// when the agent fails on GETBULK.
    ///
    /// The walk stops at the end of the subtree, at `endOfMibView`, or with
    /// [`Error::NotIncreasing`] when the agent sends an OID that is not
    /// greater than the one before. When the subtree is empty, `root`
    /// itself is fetched with GET, so walking a single instance such as
    /// `sysDescr.0` works.
    Walk {
        /// Where to start; only instances below it are returned.
        root: Oid,
        /// Successors per GETBULK request.
        max_repetitions: u32,
        /// Use GETBULK when the version allows it.
        bulk: bool,
    },
    /// SET: write these values, all in one request. The agent sets all of
    /// them or none; its response (the values as set) arrives as
    /// [`Event::Varbinds`]. Build the values with [`ValueInput`](crate::ValueInput)
    /// to have them checked against the MIB first.
    Set(Vec<Varbind>),
    /// SNMPv3 only: discover the agent's engine ID, boots and time, which
    /// arrive as [`Event::Engine`]. Nothing else is asked.
    Discover,
}

impl Operation {
    /// A walk with the default settings: GETBULK with 10 repetitions.
    pub fn walk(root: Oid) -> Operation {
        Operation::Walk {
            root,
            max_repetitions: DEFAULT_MAX_REPETITIONS,
            bulk: true,
        }
    }
}

/// The default max-repetitions for GETBULK.
pub const DEFAULT_MAX_REPETITIONS: u32 = 10;

/// What an operation reports while it runs. The last event is always
/// [`Finished`](Event::Finished) or [`Failed`](Event::Failed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Results, in the order received.
    Varbinds(Vec<Varbind>),
    /// A request was sent and answered (or not): for the operation log.
    Log(LogEntry),
    /// SNMPv3: the agent's engine, when an operation starts using it.
    Engine(EngineInfo),
    /// Something worth knowing that is not an error, such as falling back
    /// from GETBULK to GETNEXT.
    Notice(String),
    /// The operation completed.
    Finished(Summary),
    /// The operation stopped with an error. Results sent before it are
    /// still valid.
    Failed(Error),
}

/// Totals for a completed operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Variable bindings returned.
    pub varbinds: usize,
    /// Requests sent, including retries.
    pub requests: u32,
    /// Time from start to finish, including time waiting for a free slot
    /// (see [`EngineConfig::per_target_limit`]).
    pub elapsed: Duration,
}

/// Engine settings.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// How many operations may run against the same agent at once; more
    /// wait for a free slot. Small agents drop requests when flooded.
    pub per_target_limit: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            per_target_limit: 4,
        }
    }
}

/// Handle to the engine. Cheap to clone; the engine stops when the last
/// handle is dropped and the running operations have finished.
#[derive(Debug, Clone)]
pub struct Engine {
    jobs: mpsc::UnboundedSender<Job>,
}

#[derive(Debug)]
struct Job {
    target: Target,
    operation: Operation,
    events: mpsc::Sender<Event>,
}

/// How many events may wait unread before the operation pauses. Keeps a
/// huge walk from filling memory when the reader is slow.
const EVENT_BUFFER: usize = 64;

impl Engine {
    /// Start the engine on the current tokio runtime.
    ///
    /// # Panics
    ///
    /// Panics when called outside a tokio runtime.
    pub fn start(config: EngineConfig) -> Engine {
        let (jobs, receiver) = mpsc::unbounded_channel();
        tokio::spawn(serve(receiver, config));
        Engine { jobs }
    }

    /// Start an operation. Returns at once; read the results from the
    /// handle. Can be called from any thread, also outside the runtime.
    pub fn submit(&self, target: Target, operation: Operation) -> Result<Running, Error> {
        let (events, receiver) = mpsc::channel(EVENT_BUFFER);
        self.jobs
            .send(Job {
                target,
                operation,
                events,
            })
            .map_err(|_| Error::EngineStopped)?;
        Ok(Running { events: receiver })
    }
}

/// A submitted operation. Dropping it cancels the operation.
#[derive(Debug)]
pub struct Running {
    events: mpsc::Receiver<Event>,
}

impl Running {
    /// The next event, or `None` after the last one.
    pub async fn next_event(&mut self) -> Option<Event> {
        self.events.recv().await
    }

    /// The next event, waiting outside the runtime (for a thread that is
    /// not a tokio worker, such as a GUI thread).
    ///
    /// # Panics
    ///
    /// Panics when called from inside a tokio runtime.
    pub fn blocking_next_event(&mut self) -> Option<Event> {
        self.events.blocking_recv()
    }

    /// The next event if one is waiting, without blocking.
    pub fn try_next_event(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Stop the operation. The same as dropping the handle.
    pub fn cancel(self) {}
}

async fn serve(mut jobs: mpsc::UnboundedReceiver<Job>, config: EngineConfig) {
    // One semaphore per agent; entries are dropped when no job holds them.
    let mut limits: HashMap<String, Arc<Semaphore>> = HashMap::new();
    // SNMPv3 engines and keys, shared by every operation.
    let cache = V3Cache::default();
    while let Some(job) = jobs.recv().await {
        limits.retain(|_, limit| Arc::strong_count(limit) > 1);
        let limit = limits
            .entry(job.target.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(config.per_target_limit.max(1))))
            .clone();
        tokio::spawn(run_job(job, limit, cache.clone()));
    }
}

async fn run_job(job: Job, limit: Arc<Semaphore>, cache: V3Cache) {
    let Job {
        target,
        operation,
        events,
    } = job;
    let started = Instant::now();
    let work = async {
        let _permit = limit.acquire().await.map_err(|_| Error::EngineStopped)?;
        let mut run = Run::open(&target, &events, cache).await?;
        let result = run.operation(&operation).await;
        // Report the requests made even when the operation failed.
        run.flush_log().await?;
        result?;
        Ok(Summary {
            varbinds: run.varbinds,
            requests: run.requests,
            elapsed: started.elapsed(),
        })
    };
    // When the reader drops its handle, stop at once, even mid-request.
    let result = tokio::select! {
        result = work => result,
        () = events.closed() => Err(Error::Cancelled),
    };
    let last = match result {
        Ok(summary) => Event::Finished(summary),
        Err(error) => Event::Failed(error),
    };
    let _ = events.send(last).await;
}

/// The state of one running operation.
struct Run<'a> {
    session: Session,
    events: &'a mpsc::Sender<Event>,
    varbinds: usize,
    requests: u32,
}

impl<'a> Run<'a> {
    async fn open(
        target: &Target,
        events: &'a mpsc::Sender<Event>,
        cache: V3Cache,
    ) -> Result<Run<'a>, Error> {
        Ok(Run {
            session: Session::open(target, cache).await?,
            events,
            varbinds: 0,
            requests: 0,
        })
    }

    async fn send(&self, event: Event) -> Result<(), Error> {
        self.events.send(event).await.map_err(|_| Error::Cancelled)
    }

    async fn flush_log(&mut self) -> Result<(), Error> {
        for entry in self.session.take_log() {
            self.requests += 1;
            self.send(Event::Log(entry)).await?;
        }
        if let Some(info) = self.session.take_engine_info() {
            self.send(Event::Engine(info)).await?;
        }
        Ok(())
    }

    async fn request(&mut self, request: Request) -> Result<Response, Error> {
        let response = self.session.request(&request).await;
        self.flush_log().await?;
        response
    }

    /// Like [`request`](Self::request), but an error status is an error.
    async fn request_ok(&mut self, request: Request) -> Result<Response, Error> {
        let requested = request.oids();
        let response = self.request(request).await?;
        if response.status != ErrorStatus::NoError {
            return Err(response.error(self.session.target(), &requested));
        }
        Ok(response)
    }

    async fn results(&mut self, varbinds: Vec<Varbind>) -> Result<(), Error> {
        if varbinds.is_empty() {
            return Ok(());
        }
        self.varbinds += varbinds.len();
        self.send(Event::Varbinds(varbinds)).await
    }

    async fn operation(&mut self, operation: &Operation) -> Result<(), Error> {
        match operation {
            Operation::Get(oids) => {
                let response = self.request_ok(Request::Get(oids.clone())).await?;
                self.results(response.varbinds).await
            }
            Operation::GetNext(oids) => {
                for oid in oids {
                    let response = self.request_ok(Request::GetNext(oid.clone())).await?;
                    self.results(response.varbinds).await?;
                }
                Ok(())
            }
            Operation::GetBulk {
                oids,
                non_repeaters,
                max_repetitions,
            } => {
                if self.session.target().version == Version::V1 {
                    return Err(Error::BulkNeedsV2);
                }
                let request = Request::GetBulk {
                    oids: oids.clone(),
                    non_repeaters: *non_repeaters,
                    max_repetitions: *max_repetitions,
                };
                let response = self.request_ok(request).await?;
                self.results(response.varbinds).await
            }
            Operation::Walk {
                root,
                max_repetitions,
                bulk,
            } => self.walk(root, *max_repetitions, *bulk).await,
            Operation::Set(varbinds) => {
                let response = self.request_ok(Request::Set(varbinds.clone())).await?;
                self.results(response.varbinds).await
            }
            Operation::Discover => self.session.discover().await,
        }
    }

    async fn walk(&mut self, root: &Oid, max_repetitions: u32, bulk: bool) -> Result<(), Error> {
        let version = self.session.target().version;
        let mut bulk = bulk && version != Version::V1;
        let mut max_repetitions = max_repetitions.max(1);
        // GETNEXT needs an OID of two arcs or more; `1` becomes `1.0`, and
        // everything below `1` still comes after `1.0`.
        let mut last = root.clone();
        while last.len() < 2 {
            last = last.child(0);
        }
        let start = last.clone();
        let mut found = 0usize;

        loop {
            let request = if bulk {
                Request::GetBulk {
                    oids: vec![last.clone()],
                    non_repeaters: 0,
                    max_repetitions,
                }
            } else {
                Request::GetNext(last.clone())
            };
            let response = self.request(request).await?;

            match response.status {
                ErrorStatus::NoError => {}
                // SNMPv1 ends a walk with noSuchName.
                ErrorStatus::NoSuchName if version == Version::V1 => break,
                ErrorStatus::TooBig if bulk && max_repetitions > 1 => {
                    max_repetitions = (max_repetitions / 2).max(1);
                    self.send(Event::Notice(format!(
                        "the response was too big; asking for {max_repetitions} at a time"
                    )))
                    .await?;
                    continue;
                }
                status if bulk => {
                    bulk = false;
                    self.send(Event::Notice(format!(
                        "GETBULK failed with {status}; continuing with GETNEXT"
                    )))
                    .await?;
                    continue;
                }
                _ => {
                    return Err(response.error(self.session.target(), &[last]));
                }
            }

            let mut batch = Vec::with_capacity(response.varbinds.len());
            let mut done = response.varbinds.is_empty();
            for varbind in response.varbinds {
                if varbind.value == Value::EndOfMibView || !varbind.oid.starts_with(root) {
                    done = true;
                    break;
                }
                if varbind.oid <= last {
                    self.results(batch).await?;
                    return Err(Error::NotIncreasing {
                        previous: last,
                        next: varbind.oid,
                    });
                }
                last = varbind.oid.clone();
                batch.push(varbind);
            }
            found += batch.len();
            self.results(batch).await?;
            if done {
                break;
            }
        }

        // Walking a single instance, such as sysDescr.0: nothing lies below
        // it, so fetch the instance itself (as net-snmp's snmpwalk does).
        if found == 0 && start == *root {
            let response = self.request(Request::Get(vec![root.clone()])).await?;
            if response.status == ErrorStatus::NoError {
                let varbinds = response
                    .varbinds
                    .into_iter()
                    .filter(|v| !v.value.is_exception())
                    .collect();
                self.results(varbinds).await?;
            }
        }
        Ok(())
    }
}

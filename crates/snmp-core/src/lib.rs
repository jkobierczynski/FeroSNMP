//! The SNMP engine of FeroSNMP: GET, GETNEXT, GETBULK and WALK over SNMPv1,
//! SNMPv2c and SNMPv3 (USM, with pure-Rust crypto), on tokio.
//!
//! What a MIB browser needs from an SNMP engine: timeouts and retries, walks
//! that stop safely, a limit on parallel requests per agent, an operation
//! log, and value formatting through the MIBs of [`mib_core`]. The BER
//! tag-length-value parsing comes from the [snmp2] crate; why the rest of
//! snmp2 is not used (yet) is in this crate's README.md.
//!
//! # Overview
//!
//! - A [`Target`] is an agent plus version, community or SNMPv3 user
//!   ([`Usm`]), timeout and retries.
//! - An [`Operation`] is what to do: [`Get`](Operation::Get),
//!   [`GetNext`](Operation::GetNext), [`GetBulk`](Operation::GetBulk) or
//!   [`Walk`](Operation::Walk), or for SNMPv3 [`Discover`](Operation::Discover).
//! - The [`Engine`] runs operations as tokio tasks. [`Engine::submit`]
//!   returns a [`Running`] handle that streams [`Event`]s: results, log
//!   entries, and finally [`Finished`](Event::Finished) or
//!   [`Failed`](Event::Failed).
//! - A [`Formatter`] turns results into text such as
//!   `IF-MIB::ifOperStatus.1 = INTEGER: up(1)`.
//! - A [`NotificationReceiver`] decodes received traps and INFORMs
//!   (SNMPv1, SNMPv2c and SNMPv3) and builds the replies they need; the
//!   `trap-rx` crate puts it behind a UDP socket.
//!
//! # Example
//!
//! ```no_run
//! use snmp_core::{Engine, EngineConfig, Event, Formatter, Operation, Target};
//!
//! # async fn example() -> Result<(), snmp_core::Error> {
//! let engine = Engine::start(EngineConfig::default());
//! let target = Target::parse("192.0.2.1").unwrap().with_community("public");
//! let mut running = engine.submit(target, Operation::walk("1.3.6.1.2.1.1".parse().unwrap()))?;
//! let mut formatter = Formatter::new(None);
//! while let Some(event) = running.next_event().await {
//!     match event {
//!         Event::Varbinds(varbinds) => {
//!             for varbind in &varbinds {
//!                 println!("{}", formatter.line(varbind));
//!             }
//!         }
//!         Event::Failed(error) => return Err(error),
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [snmp2]: https://docs.rs/snmp2/0.5.2

#![forbid(unsafe_code)]

mod codec;
mod display_hint;
mod engine;
mod error;
mod format;
mod input;
mod notify;
mod object;
mod session;
mod table;
mod target;
mod usm;
mod v3;
mod value;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_v3;

pub use display_hint::{format_integer, format_octets, parse_integer, parse_octets};
pub use engine::{
    DEFAULT_MAX_REPETITIONS, Engine, EngineConfig, Event, Operation, Running, Summary,
};
pub use error::{Error, ErrorStatus, SecurityProblem};
pub use format::{Formatter, ValueText, raw_line};
pub use input::{InputError, ValueInput, ValueType};
pub use notify::{
    Handled, Notification, NotificationKind, NotificationReceiver, NotificationSecurity,
    ReceiveError, SNMP_TRAP_ADDRESS_0, SNMP_TRAP_COMMUNITY_0, SNMP_TRAP_ENTERPRISE_0,
    SNMP_TRAP_OID_0, SNMP_TRAPS, SYS_UP_TIME_0, TrapUser, UsmSecurity, V1Trap, new_engine_id,
};
pub use object::{ObjectError, resolve_object};
pub use session::{EngineInfo, LogEntry, uptime_text};
pub use table::{TableRow, TableView, index_text};
pub use target::{DEFAULT_PORT, ParseTargetError, Target, Version};
pub use usm::{AuthProtocol, PrivProtocol, SecurityLevel, Usm, hex, parse_hex};
pub use value::{Value, Varbind, tag};

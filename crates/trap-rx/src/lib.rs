//! Trap and INFORM receiver for FeroSNMP, decoded against the loaded MIBs.
//!
//! - [`Listener`] opens a UDP port ([`TRAP_PORT`], 162, by default) and
//!   [`Listener::spawn`] runs it as a tokio task. Each datagram goes through
//!   snmp-core's [`NotificationReceiver`](snmp_core::NotificationReceiver),
//!   which decodes SNMPv1 traps (translated to SNMPv2 form, RFC 3584),
//!   SNMPv2c traps and INFORMs, and SNMPv3 traps and INFORMs; INFORMs are
//!   acknowledged and SNMPv3 discovery is answered right there. What
//!   arrived comes out of [`Traps`] as [`Incoming`] values.
//! - [`describe`] turns a received notification into text with the MIBs:
//!   the NOTIFICATION-TYPE's name, the formatted objects, and a hint to load
//!   the vendor's MIB for any OID no loaded MIB defines.
//! - [`Filter`] picks notifications by sender or name.
//! - [`read_users`] reads the SNMPv3 trap users from `createUser` lines, as
//!   in net-snmp's `snmptrapd.conf`.
//!
//! When the port cannot be opened, [`BindError::hint`] explains the usual
//! causes: the Windows SNMP Trap service holding port 162, or Linux needing
//! `setcap` for ports below 1024.
//!
//! # Example
//!
//! ```no_run
//! use snmp_core::NotificationReceiver;
//! use trap_rx::{Incoming, Listener, describe};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let receiver = NotificationReceiver::new(vec![]);
//! let listener = Listener::bind("0.0.0.0:1162".parse()?, receiver)?;
//! let mut traps = listener.spawn()?;
//! while let Some(incoming) = traps.next().await {
//!     match incoming {
//!         Incoming::Notification(received) => print!("{}", describe(&received, None, false).text()),
//!         Incoming::Problem(problem) => eprintln!("{}: {}", problem.source, problem.error),
//!     }
//! }
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod describe;
mod filter;
mod listener;
mod users;

pub use describe::{Described, DescribedObject, DescribedV1, describe, rfc3339};
pub use filter::Filter;
pub use listener::{
    BindError, Incoming, Listener, Problem, Received, TRAP_PORT, Traps, UNPRIVILEGED_PORT,
};
pub use users::{UsersError, parse_users, read_users};

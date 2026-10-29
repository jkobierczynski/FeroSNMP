//! The MIB layer of FeroSNMP: load a library of MIB modules and answer
//! "what is this OID or name?".
//!
//! The parsing and resolution work is done by the [mib-rs] crate. mib-rs is
//! not stable before 1.0, so this crate wraps it completely: none of its types
//! appear in the public API below, and an mib-rs upgrade only touches the
//! private conversion code in this crate.
//!
//! # Overview
//!
//! - [`MibLibrary`] says where MIB files come from (your folders, the system
//!   folders of net-snmp and libsmi, and built-in foundation modules) and how
//!   strictly to read them.
//! - [`MibLibrary::load`] returns a [`MibSet`]: an immutable, cheaply
//!   cloneable view of every loaded module that can be shared between threads.
//! - A [`Node`] is one point in the OID tree. [`Node::details`] returns
//!   everything a details pane shows: OID, module, kind, type chain, access,
//!   status, units, enums, ranges and INDEX.
//! - [`MibSet::table`] describes a table: its columns and its INDEX, with
//!   the codec ([`decode_index`]) that splits an instance suffix into index
//!   values, including multi-part, `AUGMENTS` and `IMPLIED` indexes.
//! - [`MibSet::search`] finds nodes by name with `*` and `?` wildcards.
//! - Problems in the MIB files are collected as [`Diagnostic`]s with file,
//!   line and column instead of stopping the load, most with a concrete fix
//!   hint.
//! - [`MibSet::trace`] explains how a name resolves, or why it does not.
//! - [`SourceView`] is the text of a MIB file split into classified tokens,
//!   for showing the place a diagnostic points at.
//!
//! # Example
//!
//! ```
//! use mib_core::{MibLibrary, Strictness};
//!
//! // No user folders, no system folders: only the built-in foundation
//! // modules (SNMPv2-SMI, SNMPv2-TC, ...) are loaded.
//! let set = MibLibrary::new()
//!     .with_system_folders(false)
//!     .with_strictness(Strictness::Permissive)
//!     .load()
//!     .expect("the foundation modules always load");
//!
//! let enterprises = set.resolve("enterprises").expect("defined in SNMPv2-SMI");
//! assert_eq!(enterprises.oid().to_string(), "1.3.6.1.4.1");
//! assert_eq!(enterprises.module_name(), Some("SNMPv2-SMI"));
//! ```
//!
//! [mib-rs]: https://docs.rs/mib-rs/0.10.0

#![forbid(unsafe_code)]

mod convert;
mod details;
mod diagnostic;
mod error;
mod hints;
mod library;
mod oid;
mod pattern;
mod set;
mod source;
mod table;
mod trace;

pub use details::{
    Access, BaseType, IndexPart, NamedNumber, NodeDetails, NodeKind, Range, RangeBound, Status,
    Syntax, text_lines,
};
pub use diagnostic::{Diagnostic, Location, Severity};
pub use error::LoadError;
pub use library::{MibLibrary, ModuleFile, SourceKind, Strictness, load_module_text};
pub use oid::{Oid, ParseOidError};
pub use pattern::Pattern;
pub use set::{InstanceLookup, MibSet, ModuleInfo, Node, NodeId, SmiVersion};
pub use source::{SourceView, Token, TokenClass};
pub use table::{
    IndexColumn, IndexEncoding, IndexError, IndexValue, Table, TableColumn, decode_index,
    encode_index,
};
pub use trace::{
    LookupKind, ResolvedBy, SymbolTrace, TraceDefinition, TraceError, TraceImport, TraceOutcome,
    UnresolvedUse,
};

/// Version of this crate, for "about" boxes and bug reports.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

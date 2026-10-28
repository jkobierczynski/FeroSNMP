//! Plain data describing one node, as shown in a details pane.
//!
//! Everything here is owned data with public fields, so it can be kept after
//! the [`MibSet`](crate::MibSet) is dropped, sent to another thread, or printed.

use std::fmt;

use crate::oid::Oid;

/// What kind of definition a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// A node that only exists because something below it was defined.
    Internal,
    /// An `OBJECT IDENTIFIER` value assignment.
    Node,
    /// An `OBJECT-TYPE` that is not part of a table.
    Scalar,
    /// An `OBJECT-TYPE` with `SYNTAX SEQUENCE OF ...`.
    Table,
    /// The entry (row) of a table.
    Row,
    /// A column of a table.
    Column,
    /// A `NOTIFICATION-TYPE` or SMIv1 `TRAP-TYPE`.
    Notification,
    /// An `OBJECT-GROUP` or `NOTIFICATION-GROUP`.
    Group,
    /// A `MODULE-COMPLIANCE`.
    Compliance,
    /// An `AGENT-CAPABILITIES`.
    Capabilities,
    /// A `MODULE-IDENTITY`.
    ModuleIdentity,
    /// An `OBJECT-IDENTITY`.
    ObjectIdentity,
    /// Could not be determined, usually because of errors in the MIB.
    Unknown,
}

impl NodeKind {
    /// Lower-case name, such as `"scalar"` or `"module-identity"`.
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Internal => "internal",
            NodeKind::Node => "node",
            NodeKind::Scalar => "scalar",
            NodeKind::Table => "table",
            NodeKind::Row => "row",
            NodeKind::Column => "column",
            NodeKind::Notification => "notification",
            NodeKind::Group => "group",
            NodeKind::Compliance => "compliance",
            NodeKind::Capabilities => "capabilities",
            NodeKind::ModuleIdentity => "module-identity",
            NodeKind::ObjectIdentity => "object-identity",
            NodeKind::Unknown => "unknown",
        }
    }
}

impl fmt::Display for NodeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `MAX-ACCESS` (SMIv2) or `ACCESS` (SMIv1) of an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    /// `not-accessible`
    NotAccessible,
    /// `accessible-for-notify`
    AccessibleForNotify,
    /// `read-only`
    ReadOnly,
    /// `read-write`
    ReadWrite,
    /// `read-create`
    ReadCreate,
    /// `write-only` (SMIv1 only)
    WriteOnly,
    /// `not-implemented` (in AGENT-CAPABILITIES)
    NotImplemented,
}

impl Access {
    /// The keyword as written in a MIB, such as `"read-only"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Access::NotAccessible => "not-accessible",
            Access::AccessibleForNotify => "accessible-for-notify",
            Access::ReadOnly => "read-only",
            Access::ReadWrite => "read-write",
            Access::ReadCreate => "read-create",
            Access::WriteOnly => "write-only",
            Access::NotImplemented => "not-implemented",
        }
    }
}

impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `STATUS` of a definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// `current` (SMIv2)
    Current,
    /// `deprecated`
    Deprecated,
    /// `obsolete`
    Obsolete,
    /// `mandatory` (SMIv1 only)
    Mandatory,
    /// `optional` (SMIv1 only)
    Optional,
}

impl Status {
    /// The keyword as written in a MIB, such as `"current"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Current => "current",
            Status::Deprecated => "deprecated",
            Status::Obsolete => "obsolete",
            Status::Mandatory => "mandatory",
            Status::Optional => "optional",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The SMI base type that a type ultimately resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BaseType {
    /// `INTEGER` or `Integer32`
    Integer32,
    /// `Unsigned32`
    Unsigned32,
    /// `Counter32` (`Counter` in SMIv1)
    Counter32,
    /// `Counter64`
    Counter64,
    /// `Gauge32` (`Gauge` in SMIv1)
    Gauge32,
    /// `TimeTicks`
    TimeTicks,
    /// `IpAddress`
    IpAddress,
    /// `OCTET STRING`
    OctetString,
    /// `OBJECT IDENTIFIER`
    ObjectIdentifier,
    /// `BITS`
    Bits,
    /// `Opaque`
    Opaque,
    /// `SEQUENCE` (the type of a table row)
    Sequence,
    /// `Integer64` (SPPI)
    Integer64,
    /// `Unsigned64` (SPPI)
    Unsigned64,
    /// Could not be resolved, usually because of a missing import.
    Unknown,
}

impl BaseType {
    /// The name as written in a MIB, such as `"OCTET STRING"`.
    pub fn as_str(self) -> &'static str {
        match self {
            BaseType::Integer32 => "Integer32",
            BaseType::Unsigned32 => "Unsigned32",
            BaseType::Counter32 => "Counter32",
            BaseType::Counter64 => "Counter64",
            BaseType::Gauge32 => "Gauge32",
            BaseType::TimeTicks => "TimeTicks",
            BaseType::IpAddress => "IpAddress",
            BaseType::OctetString => "OCTET STRING",
            BaseType::ObjectIdentifier => "OBJECT IDENTIFIER",
            BaseType::Bits => "BITS",
            BaseType::Opaque => "Opaque",
            BaseType::Sequence => "SEQUENCE",
            BaseType::Integer64 => "Integer64",
            BaseType::Unsigned64 => "Unsigned64",
            BaseType::Unknown => "unknown",
        }
    }
}

impl fmt::Display for BaseType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One end of a [`Range`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RangeBound {
    /// A number that fits in an `i64`.
    Signed(i64),
    /// A number above `i64::MAX`, such as the top of a `Counter64` range.
    Unsigned(u64),
    /// The `MIN` keyword.
    Min,
    /// The `MAX` keyword.
    Max,
    /// A value that could not be read as a number, kept as written.
    Text(String),
}

impl fmt::Display for RangeBound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RangeBound::Signed(value) => write!(f, "{value}"),
            RangeBound::Unsigned(value) => write!(f, "{value}"),
            RangeBound::Min => f.write_str("MIN"),
            RangeBound::Max => f.write_str("MAX"),
            RangeBound::Text(text) => f.write_str(text),
        }
    }
}

/// An inclusive `min..max` constraint, used for both value ranges and SIZE.
///
/// A single value such as `SIZE (6)` has `min == max` and displays as `6`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Range {
    /// Lowest allowed value.
    pub min: RangeBound,
    /// Highest allowed value.
    pub max: RangeBound,
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.min == self.max {
            write!(f, "{}", self.min)
        } else {
            write!(f, "{}..{}", self.min, self.max)
        }
    }
}

/// A labelled number from an enumeration (`up(1)`) or a BITS definition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamedNumber {
    /// The label, such as `"up"`.
    pub label: String,
    /// The number, such as `1`. For BITS this is the bit position.
    pub value: i64,
}

impl fmt::Display for NamedNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.label, self.value)
    }
}

/// The type of an object, after following textual conventions to the base type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Syntax {
    /// Named types from the object's own type down to the ASN.1 type, for
    /// example `["DisplayString", "OCTET STRING"]` for `sysDescr`. Unnamed
    /// inline types such as `INTEGER { up(1), down(2) }` are skipped. The base
    /// type's name is always included.
    pub type_chain: Vec<String>,
    /// The SMI base type at the end of the chain.
    pub base_type: BaseType,
    /// The `DISPLAY-HINT` in effect, if any (inherited from the type chain).
    pub display_hint: Option<String>,
    /// Allowed values of an enumerated INTEGER, in definition order.
    pub enums: Vec<NamedNumber>,
    /// Named bits of a BITS type, in definition order.
    pub bits: Vec<NamedNumber>,
    /// Allowed value ranges, such as `0..65535`.
    pub ranges: Vec<Range>,
    /// Allowed sizes of an OCTET STRING, such as `0..255`.
    pub sizes: Vec<Range>,
}

/// One component of a table's `INDEX`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexPart {
    /// Name of the index object, such as `"ifIndex"`.
    pub name: String,
    /// True if the component is marked `IMPLIED` (its length is not encoded).
    pub implied: bool,
}

/// Everything known about one node, for a details pane or `mib show`.
///
/// Fields that do not apply to the node's kind are `None` or empty: a plain
/// `OBJECT IDENTIFIER` has no syntax, a scalar has no index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeDetails {
    /// Name, such as `"ifDescr"`. Empty for unnamed internal nodes.
    pub name: String,
    /// Numeric OID.
    pub oid: Oid,
    /// Module that defines the node.
    pub module: Option<String>,
    /// What kind of definition this is.
    pub kind: NodeKind,
    /// `STATUS` clause.
    pub status: Option<Status>,
    /// `MAX-ACCESS` / `ACCESS` clause (objects only).
    pub access: Option<Access>,
    /// Type information (objects only).
    pub syntax: Option<Syntax>,
    /// `UNITS` clause.
    pub units: Option<String>,
    /// INDEX components of the row this node belongs to (tables, rows and
    /// columns). For a row that uses `AUGMENTS`, the index of the augmented row.
    pub index: Vec<IndexPart>,
    /// The row named in this row's `AUGMENTS` clause.
    pub augments: Option<String>,
    /// `DESCRIPTION` text, with the MIB's own line breaks.
    pub description: Option<String>,
    /// `REFERENCE` text.
    pub reference: Option<String>,
}

impl NodeDetails {
    /// The INDEX as written in a MIB, such as `testPortSlot, IMPLIED testPortName`.
    /// Empty when the node has no index.
    pub fn index_text(&self) -> String {
        self.index
            .iter()
            .map(|part| {
                if part.implied {
                    format!("IMPLIED {}", part.name)
                } else {
                    part.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Split a DESCRIPTION or REFERENCE text into lines for display.
///
/// MIB files indent continuation lines to line up with the opening quote; that
/// common indentation is removed, as is trailing white space. Blank lines are
/// kept (as empty strings), so paragraphs stay apart.
///
/// ```
/// let text = "The textual description.\n            Second line.\n\n              Indented more.";
/// assert_eq!(
///     mib_core::text_lines(text),
///     ["The textual description.", "Second line.", "", "  Indented more."],
/// );
/// ```
pub fn text_lines(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let indent = lines
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                line.trim()
            } else {
                line.get(indent..).unwrap_or(line.trim_start()).trim_end()
            }
        })
        .collect()
}

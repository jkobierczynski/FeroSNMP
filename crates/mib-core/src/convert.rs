//! Conversions from mib-rs types to our own.
//!
//! mib-rs may change its API in any minor release. Keeping every conversion
//! in this one file means an upgrade is mostly a matter of fixing this file.

use std::path::PathBuf;

use mib_rs::mib as m;

use crate::details::{Access, BaseType, NamedNumber, NodeKind, Range, RangeBound, Status};
use crate::diagnostic::{Diagnostic, Location, Severity};
use crate::oid::Oid;

pub(crate) fn oid(oid: &m::Oid) -> Oid {
    Oid::new(oid.to_vec())
}

pub(crate) fn to_mib_oid(oid: &Oid) -> m::Oid {
    m::Oid::from(oid.arcs())
}

pub(crate) fn node_kind(kind: mib_rs::Kind) -> NodeKind {
    use mib_rs::Kind as K;
    match kind {
        K::Internal => NodeKind::Internal,
        K::Node => NodeKind::Node,
        K::Scalar => NodeKind::Scalar,
        K::Table => NodeKind::Table,
        K::Row => NodeKind::Row,
        K::Column => NodeKind::Column,
        K::Notification => NodeKind::Notification,
        K::Group => NodeKind::Group,
        K::Compliance => NodeKind::Compliance,
        K::Capability => NodeKind::Capabilities,
        K::ModuleIdentity => NodeKind::ModuleIdentity,
        K::ObjectIdentity => NodeKind::ObjectIdentity,
        K::Unknown => NodeKind::Unknown,
    }
}

pub(crate) fn access(access: mib_rs::Access) -> Access {
    use mib_rs::Access as A;
    match access {
        A::NotAccessible => Access::NotAccessible,
        A::AccessibleForNotify => Access::AccessibleForNotify,
        A::ReadOnly => Access::ReadOnly,
        A::ReadWrite => Access::ReadWrite,
        A::ReadCreate => Access::ReadCreate,
        A::WriteOnly => Access::WriteOnly,
        A::NotImplemented => Access::NotImplemented,
    }
}

pub(crate) fn status(status: mib_rs::Status) -> Status {
    use mib_rs::Status as S;
    match status {
        S::Current => Status::Current,
        S::Deprecated => Status::Deprecated,
        S::Obsolete => Status::Obsolete,
        S::Mandatory => Status::Mandatory,
        S::Optional => Status::Optional,
    }
}

pub(crate) fn base_type(base: mib_rs::BaseType) -> BaseType {
    use mib_rs::BaseType as B;
    match base {
        B::Integer32 => BaseType::Integer32,
        B::Unsigned32 => BaseType::Unsigned32,
        B::Counter32 => BaseType::Counter32,
        B::Counter64 => BaseType::Counter64,
        B::Gauge32 => BaseType::Gauge32,
        B::TimeTicks => BaseType::TimeTicks,
        B::IpAddress => BaseType::IpAddress,
        B::OctetString => BaseType::OctetString,
        B::ObjectIdentifier => BaseType::ObjectIdentifier,
        B::Bits => BaseType::Bits,
        B::Opaque => BaseType::Opaque,
        B::Sequence => BaseType::Sequence,
        B::Integer64 => BaseType::Integer64,
        B::Unsigned64 => BaseType::Unsigned64,
        B::Unknown => BaseType::Unknown,
    }
}

pub(crate) fn ranges(ranges: &[m::Range]) -> Vec<Range> {
    ranges
        .iter()
        .map(|range| Range {
            min: range_bound(&range.min),
            max: range_bound(&range.max),
        })
        .collect()
}

fn range_bound(bound: &m::RangeBound) -> RangeBound {
    match bound {
        m::RangeBound::Signed(value) => RangeBound::Signed(*value),
        // mib-rs stores every non-negative literal as unsigned; use Signed
        // wherever the value fits, so `0..65535` compares as two Signed bounds.
        m::RangeBound::Unsigned(value) => match i64::try_from(*value) {
            Ok(value) => RangeBound::Signed(value),
            Err(_) => RangeBound::Unsigned(*value),
        },
        m::RangeBound::Min => RangeBound::Min,
        m::RangeBound::Max => RangeBound::Max,
        m::RangeBound::Raw(text) => RangeBound::Text(text.clone()),
    }
}

pub(crate) fn named_numbers(values: &[m::NamedValue]) -> Vec<NamedNumber> {
    values
        .iter()
        .map(|value| NamedNumber {
            label: value.label.clone(),
            value: value.value,
        })
        .collect()
}

fn severity(severity: mib_rs::Severity) -> Severity {
    use mib_rs::Severity as S;
    match severity {
        S::Fatal => Severity::Fatal,
        S::Severe => Severity::Severe,
        S::Error => Severity::Error,
        S::Minor => Severity::Minor,
        S::Style => Severity::Style,
        S::Warning => Severity::Warning,
        S::Info => Severity::Info,
    }
}

/// Convert every diagnostic in a report, resolving byte ranges to lines and
/// columns while the source text is still at hand.
pub(crate) fn diagnostics(report: &mib_rs::DiagnosticReport) -> Vec<Diagnostic> {
    report
        .iter()
        .map(|entry| {
            let diagnostic = entry.diagnostic();
            Diagnostic {
                severity: severity(diagnostic.severity),
                code: diagnostic.code.as_code().to_string(),
                module: diagnostic.module.clone(),
                location: location(&entry),
                message: diagnostic.message.clone(),
                hint: None,
                symbol: None,
            }
        })
        .collect()
}

fn location(entry: &mib_rs::DiagnosticEntry<'_>) -> Option<Location> {
    let (source, _) = entry.range().ok()??;
    // Count columns in characters where the text allows it, so a column
    // matches what an editor shows. Files that are not valid UTF-8 (some
    // vendor MIBs are Latin-1) fall back to byte columns.
    let (start, end) = match entry.positions(mib_rs::PositionEncoding::Utf32) {
        Ok(Some((start, end))) => (
            (start.line(), start.character()),
            (end.line(), end.character()),
        ),
        _ => {
            let (start, end) = entry.byte_positions().ok()??;
            ((start.line(), start.column()), (end.line(), end.column()))
        }
    };
    Some(Location {
        file: source_file(source.origin(), source.label()),
        line: start.0 + 1,
        column: start.1 + 1,
        end_line: end.0 + 1,
        end_column: end.1 + 1,
    })
}

/// The file a source came from, or `embedded:<MODULE>` for built-in modules.
pub(crate) fn source_file(origin: &mib_rs::SourceOrigin, label: &str) -> PathBuf {
    match origin {
        mib_rs::SourceOrigin::File { path } => path.clone(),
        _ => PathBuf::from(label),
    }
}

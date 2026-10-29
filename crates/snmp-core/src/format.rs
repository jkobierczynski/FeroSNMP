//! Turning variable bindings into text, using the MIBs for names, enum
//! labels, BITS names, display hints and units.

use std::collections::HashMap;
use std::fmt::Write;
use std::rc::Rc;

use mib_core::{BaseType, MibSet, NamedNumber, NodeId, NodeKind, Oid};

use crate::display_hint;
use crate::value::{Value, Varbind};

/// Formats variable bindings for people, the way net-snmp's tools do:
/// `IF-MIB::ifOperStatus.1 = INTEGER: up(1)`.
///
/// Without MIBs (or for objects no loaded MIB defines), OIDs are numeric and
/// values are shown by their wire type only.
#[derive(Debug)]
pub struct Formatter<'a> {
    mibs: Option<&'a MibSet>,
    numeric: bool,
    /// What the MIB says about each object seen so far; a walk asks about
    /// the same columns over and over.
    syntax: HashMap<NodeId, Option<Rc<ObjectSyntax>>>,
}

/// The parts of an object's definition that affect how its value is shown.
#[derive(Debug)]
struct ObjectSyntax {
    base_type: BaseType,
    enums: Vec<NamedNumber>,
    bits: Vec<NamedNumber>,
    display_hint: Option<String>,
    units: Option<String>,
}

impl<'a> Formatter<'a> {
    /// A formatter that uses `mibs` when given.
    pub fn new(mibs: Option<&'a MibSet>) -> Formatter<'a> {
        Formatter {
            mibs,
            numeric: false,
            syntax: HashMap::new(),
        }
    }

    /// Show OIDs (names and OID values) as numbers, like `snmpwalk -On`.
    /// Values still use the MIBs.
    pub fn with_numeric_oids(mut self, numeric: bool) -> Formatter<'a> {
        self.numeric = numeric;
        self
    }

    /// An OID as `MODULE::name.suffix`, or numeric.
    pub fn oid(&self, oid: &Oid) -> String {
        match self.mibs {
            Some(mibs) if !self.numeric => mibs.format_oid(oid),
            _ => oid.to_string(),
        }
    }

    /// `name = value`, as one line.
    pub fn line(&mut self, varbind: &Varbind) -> String {
        format!("{} = {}", self.oid(&varbind.oid), self.value(varbind))
    }

    /// The value with its type, such as `INTEGER: up(1)` or
    /// `STRING: "Linux"`.
    pub fn value(&mut self, varbind: &Varbind) -> String {
        let syntax = self.syntax_of(&varbind.oid);
        let syntax = syntax.as_deref();
        let units = |text: String| match syntax.and_then(|s| s.units.as_deref()) {
            Some(units) => format!("{text} {units}"),
            None => text,
        };
        match &varbind.value {
            Value::Integer(n) => {
                if let Some(label) = syntax.and_then(|s| s.enums.iter().find(|e| e.value == *n)) {
                    return format!("INTEGER: {}({n})", label.label);
                }
                let hinted = syntax
                    .and_then(|s| s.display_hint.as_deref())
                    .and_then(|hint| display_hint::format_integer(hint, *n));
                units(format!(
                    "INTEGER: {}",
                    hinted.unwrap_or_else(|| n.to_string())
                ))
            }
            Value::OctetString(data) => match syntax {
                Some(s) if s.base_type == BaseType::Bits => format_bits(data, &s.bits),
                Some(ObjectSyntax {
                    display_hint: Some(hint),
                    ..
                }) => match display_hint::format_octets(hint, data) {
                    Some(text) => format!("STRING: {}", quote(&text)),
                    None => plain_value(&varbind.value),
                },
                _ => plain_value(&varbind.value),
            },
            Value::ObjectIdentifier(oid) => format!("OID: {}", self.oid(oid)),
            Value::Counter32(_) | Value::Gauge32(_) | Value::Counter64(_) => {
                units(plain_value(&varbind.value))
            }
            other => plain_value(other),
        }
    }

    /// The value split in two for a table column each: its type, such as
    /// `INTEGER`, and its text, such as `up(1)`. For the exceptions
    /// (`noSuchObject` ...) the type is the exception's name.
    pub fn value_text(&mut self, varbind: &Varbind) -> ValueText {
        let line = self.value(varbind);
        let exception = |text: String| ValueText {
            type_name: varbind.value.type_name().to_string(),
            text,
        };
        match &varbind.value {
            Value::Null => exception(String::new()),
            value if value.is_exception() => exception(line),
            _ => match line.split_once(": ") {
                Some((type_name, text)) => ValueText {
                    type_name: type_name.to_string(),
                    text: text.to_string(),
                },
                None => exception(line),
            },
        }
    }

    fn syntax_of(&mut self, oid: &Oid) -> Option<Rc<ObjectSyntax>> {
        let mibs = self.mibs?;
        let lookup = mibs.lookup(oid);
        // Only instances of objects have a syntax; a value that arrives
        // for some other node (a broken agent) is shown plainly.
        if !matches!(lookup.node.kind(), NodeKind::Scalar | NodeKind::Column) {
            return None;
        }
        self.syntax
            .entry(lookup.node.id())
            .or_insert_with(|| {
                let details = lookup.node.details();
                let syntax = details.syntax?;
                Some(Rc::new(ObjectSyntax {
                    base_type: syntax.base_type,
                    enums: syntax.enums,
                    bits: syntax.bits,
                    display_hint: syntax.display_hint,
                    units: details.units,
                }))
            })
            .clone()
    }
}

/// A value as two table cells; see [`Formatter::value_text`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueText {
    /// The type as net-snmp names it: `INTEGER`, `STRING`, `Hex-STRING`,
    /// `Timeticks`, ...
    pub type_name: String,
    /// The value without its type.
    pub text: String,
}

/// The value by its wire type alone, without MIB information.
pub(crate) fn plain_value(value: &Value) -> String {
    match value {
        Value::Integer(n) => format!("INTEGER: {n}"),
        Value::OctetString(data) => match printable(data) {
            Some(text) => format!("STRING: {}", quote(text)),
            None if data.is_empty() => "STRING: \"\"".to_string(),
            None => format!("Hex-STRING: {}", hex(data)),
        },
        Value::Null => "NULL".to_string(),
        Value::ObjectIdentifier(oid) => format!("OID: {oid}"),
        Value::IpAddress([a, b, c, d]) => format!("IpAddress: {a}.{b}.{c}.{d}"),
        Value::Counter32(n) => format!("Counter32: {n}"),
        Value::Gauge32(n) => format!("Gauge32: {n}"),
        Value::TimeTicks(n) => format!("Timeticks: ({n}) {}", ticks_text(*n)),
        Value::Opaque(data) => format!("Opaque: {}", hex(data)),
        Value::Counter64(n) => format!("Counter64: {n}"),
        Value::NoSuchObject => "No Such Object available on this agent at this OID".to_string(),
        Value::NoSuchInstance => "No Such Instance currently exists at this OID".to_string(),
        Value::EndOfMibView => {
            "No more variables left in this MIB View (It is past the end of the MIB tree)"
                .to_string()
        }
        Value::Other { tag, data } => format!("Unknown type 0x{tag:02x}: {}", hex(data)),
    }
}

/// The raw view of a variable binding: numeric OID, ASN.1 tag and type,
/// and the value's bytes in hex, such as
/// `1.3.6.1.2.1.1.1.0 = [04 OCTET STRING] 4c 69 6e 75 78`.
pub fn raw_line(varbind: &Varbind) -> String {
    let octets = varbind.value.content_octets();
    let mut line = format!(
        "{} = [{:02x} {}]",
        varbind.oid,
        varbind.value.tag(),
        varbind.value.type_name()
    );
    if octets.is_empty() {
        line.push_str(" (empty)");
    } else {
        line.push(' ');
        line.push_str(&hex(&octets).to_lowercase());
    }
    line
}

/// Bytes as `00 1B 21`.
fn hex(data: &[u8]) -> String {
    let mut text = String::with_capacity(data.len() * 3);
    for (i, byte) in data.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        let _ = write!(text, "{byte:02X}");
    }
    text
}

/// The data as text, if it is UTF-8 without control characters other than
/// tab, carriage return and line feed.
fn printable(data: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(data).ok()?;
    let ok = !text.is_empty()
        && text
            .chars()
            .all(|c| !c.is_control() || matches!(c, '\t' | '\r' | '\n'));
    ok.then_some(text)
}

/// Text in double quotes, with `"` and `\` escaped.
fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        if matches!(c, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// Hundredths of a second as `14 days, 6:56:07.89`.
fn ticks_text(ticks: u32) -> String {
    let hundredths = ticks % 100;
    let seconds = ticks / 100;
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (hours, minutes, secs) = (rest / 3600, rest % 3600 / 60, rest % 60);
    let time = format!("{hours}:{minutes:02}:{secs:02}.{hundredths:02}");
    match days {
        0 => time,
        1 => format!("1 day, {time}"),
        _ => format!("{days} days, {time}"),
    }
}

/// BITS as `BITS: A0 40 alpha(0) gamma(2) delta(9)`. Bit 0 is the most
/// significant bit of the first byte.
fn format_bits(data: &[u8], names: &[NamedNumber]) -> String {
    let mut text = format!("BITS: {}", hex(data));
    for (byte_index, byte) in data.iter().enumerate() {
        for bit in 0..8 {
            if byte & (0x80 >> bit) == 0 {
                continue;
            }
            let position = (byte_index * 8 + bit) as i64;
            match names.iter().find(|n| n.value == position) {
                Some(name) => {
                    let _ = write!(text, " {}({position})", name.label);
                }
                None => {
                    let _ = write!(text, " {position}");
                }
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varbind(value: Value) -> Varbind {
        Varbind {
            oid: "1.3.6.1.2.1.1.1.0".parse().unwrap(),
            value,
        }
    }

    #[test]
    fn plain_values() {
        let mut f = Formatter::new(None);
        let cases = [
            (Value::Integer(-3), "INTEGER: -3"),
            (Value::OctetString(b"Linux".to_vec()), "STRING: \"Linux\""),
            (
                Value::OctetString(b"say \"hi\"".to_vec()),
                "STRING: \"say \\\"hi\\\"\"",
            ),
            (Value::OctetString(vec![0, 1, 0xff]), "Hex-STRING: 00 01 FF"),
            (Value::OctetString(vec![]), "STRING: \"\""),
            (
                Value::TimeTicks(123_456_789),
                "Timeticks: (123456789) 14 days, 6:56:07.89",
            ),
            (
                Value::TimeTicks(8_640_000),
                "Timeticks: (8640000) 1 day, 0:00:00.00",
            ),
            (Value::TimeTicks(4_523), "Timeticks: (4523) 0:00:45.23"),
            (Value::IpAddress([192, 0, 2, 1]), "IpAddress: 192.0.2.1"),
            (Value::Gauge32(7), "Gauge32: 7"),
            (
                Value::Other {
                    tag: 0x01,
                    data: vec![0xff],
                },
                "Unknown type 0x01: FF",
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(f.value(&varbind(value)), expected);
        }
        assert_eq!(f.line(&varbind(Value::Null)), "1.3.6.1.2.1.1.1.0 = NULL");
    }

    #[test]
    fn value_columns() {
        let mut f = Formatter::new(None);
        let mut text = |value| {
            let ValueText { type_name, text } = f.value_text(&varbind(value));
            (type_name, text)
        };
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            text(Value::OctetString(b"a: b".to_vec())),
            pair("STRING", "\"a: b\"")
        );
        assert_eq!(
            text(Value::TimeTicks(4_523)),
            pair("Timeticks", "(4523) 0:00:45.23")
        );
        assert_eq!(text(Value::Null), pair("NULL", ""));
        assert_eq!(
            text(Value::NoSuchInstance),
            pair(
                "noSuchInstance",
                "No Such Instance currently exists at this OID"
            )
        );
    }

    #[test]
    fn raw_lines() {
        assert_eq!(
            raw_line(&varbind(Value::OctetString(b"Li".to_vec()))),
            "1.3.6.1.2.1.1.1.0 = [04 OCTET STRING] 4c 69"
        );
        assert_eq!(
            raw_line(&varbind(Value::Gauge32(0xffff_ffff))),
            "1.3.6.1.2.1.1.1.0 = [42 Gauge32] 00 ff ff ff ff"
        );
        assert_eq!(
            raw_line(&varbind(Value::NoSuchObject)),
            "1.3.6.1.2.1.1.1.0 = [80 noSuchObject] (empty)"
        );
    }

    #[test]
    fn bits() {
        let names = [
            NamedNumber {
                label: "alpha".into(),
                value: 0,
            },
            NamedNumber {
                label: "gamma".into(),
                value: 2,
            },
        ];
        assert_eq!(
            format_bits(&[0xa0, 0x40], &names),
            "BITS: A0 40 alpha(0) gamma(2) 9"
        );
    }
}

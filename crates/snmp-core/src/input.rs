//! The value for a SET, built from what the user typed and the object's MIB
//! type, and checked against the MIB (enumerations, ranges, sizes, access)
//! before it is sent.

use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

use mib_core::{Access, BaseType, MibSet, NodeKind, Oid, Range, RangeBound, Syntax};

use crate::display_hint;
use crate::object::resolve_object;
use crate::usm::parse_hex;
use crate::value::{Value, Varbind};

/// How to read the text of a value: the SMI type to send, and for strings
/// whether the text is the bytes themselves or hex. Usually it comes from
/// the MIB; give one to [`ValueInput::parse`] for objects no loaded MIB
/// defines, or to type bytes in hex.
///
/// Parses from net-snmp's `snmpset` letters (`i`, `u`, `c`, `C`, `t`, `a`,
/// `o`, `s`, `x`, `b`) or from names such as `INTEGER` or `hex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueType {
    /// `INTEGER` / `Integer32`, or an enumeration label.
    Integer,
    /// `Unsigned32` / `Gauge32`.
    Unsigned32,
    /// `Counter32`.
    Counter32,
    /// `Counter64`.
    Counter64,
    /// `TimeTicks`, in hundredths of a second.
    TimeTicks,
    /// `IpAddress`, such as `192.0.2.1`.
    IpAddress,
    /// `OBJECT IDENTIFIER`: a name such as `sysDescr` or a numeric OID.
    ObjectIdentifier,
    /// `OCTET STRING`, typed as text (or as the display hint shows it).
    Text,
    /// `OCTET STRING`, typed as hex bytes such as `00 1b 21` or `001b21`.
    Hex,
    /// `BITS`: bit names or numbers, such as `alpha gamma` or `0 2`.
    Bits,
}

impl ValueType {
    /// Every type, in the order a list shows them.
    pub const ALL: [ValueType; 10] = [
        ValueType::Integer,
        ValueType::Unsigned32,
        ValueType::Counter32,
        ValueType::Counter64,
        ValueType::TimeTicks,
        ValueType::IpAddress,
        ValueType::ObjectIdentifier,
        ValueType::Text,
        ValueType::Hex,
        ValueType::Bits,
    ];

    /// net-snmp's `snmpset` letter.
    pub fn letter(self) -> char {
        match self {
            ValueType::Integer => 'i',
            ValueType::Unsigned32 => 'u',
            ValueType::Counter32 => 'c',
            ValueType::Counter64 => 'C',
            ValueType::TimeTicks => 't',
            ValueType::IpAddress => 'a',
            ValueType::ObjectIdentifier => 'o',
            ValueType::Text => 's',
            ValueType::Hex => 'x',
            ValueType::Bits => 'b',
        }
    }

    /// A name for lists, such as `"INTEGER"` or `"OCTET STRING (hex)"`.
    pub fn name(self) -> &'static str {
        match self {
            ValueType::Integer => "INTEGER",
            ValueType::Unsigned32 => "Unsigned32",
            ValueType::Counter32 => "Counter32",
            ValueType::Counter64 => "Counter64",
            ValueType::TimeTicks => "TimeTicks",
            ValueType::IpAddress => "IpAddress",
            ValueType::ObjectIdentifier => "OBJECT IDENTIFIER",
            ValueType::Text => "OCTET STRING (text)",
            ValueType::Hex => "OCTET STRING (hex)",
            ValueType::Bits => "BITS",
        }
    }

    /// The type for an SMI base type; `None` for types that cannot be set.
    pub fn for_base(base: BaseType) -> Option<ValueType> {
        Some(match base {
            BaseType::Integer32 => ValueType::Integer,
            BaseType::Unsigned32 | BaseType::Gauge32 => ValueType::Unsigned32,
            BaseType::Counter32 => ValueType::Counter32,
            BaseType::Counter64 => ValueType::Counter64,
            BaseType::TimeTicks => ValueType::TimeTicks,
            BaseType::IpAddress => ValueType::IpAddress,
            BaseType::ObjectIdentifier => ValueType::ObjectIdentifier,
            BaseType::OctetString => ValueType::Text,
            BaseType::Opaque => ValueType::Hex,
            BaseType::Bits => ValueType::Bits,
            BaseType::Sequence | BaseType::Integer64 | BaseType::Unsigned64 | BaseType::Unknown => {
                return None;
            }
        })
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for ValueType {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        // The two letters that differ only in case first.
        match text.trim() {
            "c" => return Ok(ValueType::Counter32),
            "C" => return Ok(ValueType::Counter64),
            _ => {}
        }
        let name = text
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-', '_'], "");
        Ok(match name.as_str() {
            "i" | "integer" | "integer32" | "int" => ValueType::Integer,
            "u" | "unsigned32" | "unsigned" | "gauge32" | "gauge" => ValueType::Unsigned32,
            "counter32" | "counter" => ValueType::Counter32,
            "counter64" => ValueType::Counter64,
            "t" | "timeticks" => ValueType::TimeTicks,
            "a" | "ipaddress" | "ip" => ValueType::IpAddress,
            "o" | "objectidentifier" | "oid" => ValueType::ObjectIdentifier,
            "s" | "string" | "octetstring" | "text" => ValueType::Text,
            "x" | "hex" | "hexstring" => ValueType::Hex,
            "b" | "bits" => ValueType::Bits,
            _ => {
                return Err(format!(
                    "unknown type {text:?}: use i (INTEGER), u (Unsigned32), c (Counter32), \
                     C (Counter64), t (TimeTicks), a (IpAddress), o (OBJECT IDENTIFIER), \
                     s (text), x (hex) or b (BITS)"
                ));
            }
        })
    }
}

/// Why a value cannot be built or should not be sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    /// The OID is not an instance of an object, such as a table or a group.
    #[error("{name} is a {kind}, not an object that holds a value")]
    NotAnObject {
        /// The OID as shown to the user.
        name: String,
        /// What it is instead.
        kind: NodeKind,
    },
    /// The OID names the object itself instead of an instance of it.
    #[error("{name} is the object itself; SET needs an instance, such as {example}")]
    NoInstance {
        /// The OID as shown to the user.
        name: String,
        /// What an instance looks like: `sysContact.0`, `ifAdminStatus.<index>`.
        example: String,
    },
    /// The MIB says the object cannot be written.
    #[error("{name} is {access} in the MIB, so the agent will refuse to change it")]
    NotWritable {
        /// The OID as shown to the user.
        name: String,
        /// The object's MAX-ACCESS.
        access: Access,
    },
    /// No loaded MIB says what type the object has, and none was given.
    #[error(
        "no loaded MIB says what type {name} has; give the type, such as i (INTEGER), \
         u (Unsigned32), s (text), x (hex), a (IpAddress) or o (OBJECT IDENTIFIER)"
    )]
    UnknownType {
        /// The OID as shown to the user.
        name: String,
    },
    /// The text cannot be read as a value of the type.
    #[error("{name}: {message}")]
    Invalid {
        /// The OID as shown to the user.
        name: String,
        /// What is wrong.
        message: String,
    },
    /// The value is of the right type, but the MIB does not allow it.
    #[error("{name}: {message}")]
    NotAllowed {
        /// The OID as shown to the user.
        name: String,
        /// What is not allowed, and what is.
        message: String,
    },
}

impl InputError {
    /// True for the problems that sending anyway can get past (access and
    /// MIB constraints, which an agent may not enforce); false when there
    /// is no value to send at all.
    pub fn can_force(&self) -> bool {
        matches!(
            self,
            InputError::NotWritable { .. }
                | InputError::NotAllowed { .. }
                | InputError::NoInstance { .. }
                | InputError::NotAnObject { .. }
        )
    }
}

/// What the MIB says about one instance to SET, and how to turn text into
/// its value.
///
/// ```
/// # let mibs = mib_core::MibLibrary::new().with_system_folders(false).load().unwrap();
/// use snmp_core::{Value, ValueInput, ValueType};
///
/// // An OID no loaded MIB defines: the type must be given.
/// let input = ValueInput::new(Some(&mibs), "1.3.6.1.4.1.99999.1.0".parse().unwrap());
/// assert!(input.parse("x", None).is_err());
/// assert_eq!(
///     input.parse("00 1b 21", Some(ValueType::Hex)).unwrap(),
///     Value::OctetString(vec![0x00, 0x1b, 0x21])
/// );
/// ```
#[derive(Debug, Clone)]
pub struct ValueInput {
    /// The instance to set.
    pub oid: Oid,
    /// The instance as shown to the user, such as `SNMPv2-MIB::sysContact.0`.
    pub name: String,
    /// What kind of node the OID is an instance of, when the MIBs know it.
    pub kind: Option<NodeKind>,
    /// True if the OID has arcs after the object's own OID.
    pub instance: bool,
    /// The object's MAX-ACCESS.
    pub access: Option<Access>,
    /// The object's type.
    pub syntax: Option<Syntax>,
    mibs: Option<MibSet>,
}

impl ValueInput {
    /// Look up what the MIBs say about `oid`.
    pub fn new(mibs: Option<&MibSet>, oid: Oid) -> ValueInput {
        let mut input = ValueInput {
            name: oid.to_string(),
            oid,
            kind: None,
            instance: true,
            access: None,
            syntax: None,
            mibs: mibs.cloned(),
        };
        if let Some(mibs) = mibs {
            let lookup = mibs.lookup(&input.oid);
            let object = matches!(lookup.node.kind(), NodeKind::Scalar | NodeKind::Column);
            // Below a node that is not an object (enterprises.99999.1.0),
            // the MIBs know nothing about the instance.
            if !lookup.node.oid().is_empty() && (object || lookup.suffix.is_empty()) {
                let details = lookup.node.details();
                input.name = mibs.format_oid(&input.oid);
                input.kind = Some(details.kind);
                input.instance = !lookup.suffix.is_empty();
                input.access = details.access;
                input.syntax = details.syntax;
            }
        }
        input
    }

    /// The type the MIB gives the object.
    pub fn value_type(&self) -> Option<ValueType> {
        ValueType::for_base(self.syntax.as_ref()?.base_type)
    }

    /// The object's type as a MIB writes it, such as `DisplayString (SIZE
    /// (0..255))` or `INTEGER { up(1), down(2) }`; empty when unknown.
    pub fn type_text(&self) -> String {
        let Some(syntax) = &self.syntax else {
            return String::new();
        };
        let mut text = syntax.type_chain.first().cloned().unwrap_or_default();
        if let Some(allowed) = self.allowed() {
            text.push_str(&format!(" ({allowed})"));
        }
        text
    }

    /// What the MIB allows, such as `off(1), on(2), auto(3)`, `0..100` or
    /// `SIZE 0..32`; `None` when it allows anything of the type.
    pub fn allowed(&self) -> Option<String> {
        let syntax = self.syntax.as_ref()?;
        let list = |ranges: &[Range]| {
            ranges
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | ")
        };
        if !syntax.enums.is_empty() {
            Some(join(&syntax.enums))
        } else if !syntax.bits.is_empty() {
            Some(format!("bits {}", join(&syntax.bits)))
        } else if !syntax.ranges.is_empty() {
            Some(list(&syntax.ranges))
        } else if !syntax.sizes.is_empty()
            && matches!(syntax.base_type, BaseType::OctetString | BaseType::Opaque)
        {
            Some(format!("SIZE {}", list(&syntax.sizes)))
        } else {
            None
        }
    }

    /// Check that the OID is an instance of an object that can be written.
    pub fn check_writable(&self) -> Result<(), InputError> {
        if let Some(kind) = self.kind
            && !matches!(kind, NodeKind::Scalar | NodeKind::Column)
        {
            return Err(InputError::NotAnObject {
                name: self.name.clone(),
                kind,
            });
        }
        if !self.instance {
            return Err(InputError::NoInstance {
                name: self.name.clone(),
                example: match self.kind {
                    Some(NodeKind::Column) => format!("{}.<index>", self.name),
                    _ => format!("{}.0", self.name),
                },
            });
        }
        match self.access {
            Some(Access::ReadWrite | Access::ReadCreate | Access::WriteOnly) | None => Ok(()),
            Some(access) => Err(InputError::NotWritable {
                name: self.name.clone(),
                access,
            }),
        }
    }

    /// Read `text` as a value of `value_type`, or of the MIB's type when
    /// `None`. Only checks that the text is a value of the type; see
    /// [`check`](Self::check) for what the MIB allows.
    pub fn parse(&self, text: &str, value_type: Option<ValueType>) -> Result<Value, InputError> {
        let value_type =
            value_type
                .or_else(|| self.value_type())
                .ok_or_else(|| InputError::UnknownType {
                    name: self.name.clone(),
                })?;
        let invalid = |message: String| InputError::Invalid {
            name: self.name.clone(),
            message,
        };
        let syntax = self.syntax.as_ref();
        let hint = syntax.and_then(|s| s.display_hint.as_deref());
        let trimmed = text.trim();
        Ok(match value_type {
            ValueType::Integer => Value::Integer(self.parse_integer(trimmed).map_err(invalid)?),
            ValueType::Unsigned32 => {
                Value::Gauge32(number(trimmed, "Unsigned32").map_err(invalid)?)
            }
            ValueType::Counter32 => {
                Value::Counter32(number(trimmed, "Counter32").map_err(invalid)?)
            }
            ValueType::Counter64 => {
                Value::Counter64(number(trimmed, "Counter64").map_err(invalid)?)
            }
            ValueType::TimeTicks => Value::TimeTicks(
                number(trimmed, "TimeTicks (hundredths of a second)").map_err(invalid)?,
            ),
            ValueType::IpAddress => {
                let address: Ipv4Addr = trimmed.parse().map_err(|_| {
                    invalid(format!(
                        "{trimmed:?} is not an IPv4 address such as 192.0.2.1"
                    ))
                })?;
                Value::IpAddress(address.octets())
            }
            ValueType::ObjectIdentifier => {
                let oid = match &self.mibs {
                    Some(mibs) => resolve_object(mibs, trimmed).map_err(|e| invalid(e.to_string())),
                    None => trimmed
                        .parse::<Oid>()
                        .map_err(|_| invalid(format!("{trimmed:?} is not a numeric OID"))),
                }?;
                Value::ObjectIdentifier(oid)
            }
            ValueType::Hex => {
                let bytes = if trimmed.is_empty() {
                    Vec::new()
                } else {
                    parse_hex(trimmed).map_err(invalid)?
                };
                match syntax.map(|s| s.base_type) {
                    Some(BaseType::Opaque) => Value::Opaque(bytes),
                    _ => Value::OctetString(bytes),
                }
            }
            // Text is taken as it is (spaces included); a display hint
            // that is not plain text says how to read it.
            ValueType::Text => match hint.filter(|hint| !is_text_hint(hint)) {
                Some(hint) => {
                    Value::OctetString(display_hint::parse_octets(hint, trimmed).map_err(
                        |message| invalid(format!("{message}; or give the bytes in hex")),
                    )?)
                }
                None => Value::OctetString(text.as_bytes().to_vec()),
            },
            ValueType::Bits => Value::OctetString(self.parse_bits(trimmed).map_err(invalid)?),
        })
    }

    fn parse_integer(&self, text: &str) -> Result<i64, String> {
        let syntax = self.syntax.as_ref();
        let enums = syntax.map(|s| s.enums.as_slice()).unwrap_or_default();
        if !enums.is_empty() {
            // "up", "up(1)" or "1".
            let label = text.split_once('(').map_or(text, |(label, _)| label).trim();
            if let Some(found) = enums.iter().find(|e| e.label.eq_ignore_ascii_case(label)) {
                return Ok(found.value);
            }
            return text
                .parse::<i64>()
                .map_err(|_| format!("{text:?} is not one of {} (or a number)", join(enums)));
        }
        let value = match syntax.and_then(|s| s.display_hint.as_deref()) {
            Some(hint) => display_hint::parse_integer(hint, text)
                .or_else(|| text.parse().ok().filter(|_| !hint.contains('-')))
                .ok_or_else(|| {
                    format!("{text:?} is not a number as the display hint {hint:?} writes it")
                })?,
            None => text
                .parse::<i64>()
                .map_err(|_| format!("{text:?} is not a whole number"))?,
        };
        if i32::try_from(value).is_err() {
            return Err(format!(
                "{value} does not fit in an INTEGER (-2147483648..2147483647)"
            ));
        }
        Ok(value)
    }

    fn parse_bits(&self, text: &str) -> Result<Vec<u8>, String> {
        let names = self
            .syntax
            .as_ref()
            .map(|s| s.bits.as_slice())
            .unwrap_or_default();
        let mut positions = Vec::new();
        for word in text.split([' ', ',', '|']).filter(|w| !w.is_empty()) {
            let label = word.split_once('(').map_or(word, |(label, _)| label);
            let position = match names.iter().find(|b| b.label.eq_ignore_ascii_case(label)) {
                Some(bit) => bit.value,
                None => word.parse::<i64>().map_err(|_| {
                    if names.is_empty() {
                        format!("{word:?} is not a bit number")
                    } else {
                        format!("{word:?} is not one of the bits {}", join(names))
                    }
                })?,
            };
            if !(0..65536).contains(&position) {
                return Err(format!("bit {position} is out of range"));
            }
            positions.push(position as usize);
        }
        // BITS are sent as an OCTET STRING, bit 0 the top bit of the first
        // byte, with as many bytes as the highest bit needs.
        let mut bytes = vec![0u8; positions.iter().max().map_or(0, |&p| p / 8 + 1)];
        for position in positions {
            bytes[position / 8] |= 0x80 >> (position % 8);
        }
        Ok(bytes)
    }

    /// Check a value against the MIB: enumeration, ranges and sizes.
    pub fn check(&self, value: &Value) -> Result<(), InputError> {
        let Some(syntax) = &self.syntax else {
            return Ok(());
        };
        let not_allowed = |message: String| {
            Err(InputError::NotAllowed {
                name: self.name.clone(),
                message,
            })
        };
        let wrong_type = |sent: &str| {
            not_allowed(format!(
                "the MIB type is {}, but the value is {sent}",
                syntax.base_type
            ))
        };
        let expected = ValueType::for_base(syntax.base_type);
        match value {
            Value::Integer(n) => {
                if expected != Some(ValueType::Integer) {
                    return wrong_type("an INTEGER");
                }
                if !syntax.enums.is_empty() && !syntax.enums.iter().any(|e| e.value == *n) {
                    return not_allowed(format!("{n} is not one of {}", join(&syntax.enums)));
                }
                self.check_ranges(i128::from(*n))
            }
            Value::Gauge32(n) | Value::Counter32(n) | Value::TimeTicks(n) => {
                let sent = match value {
                    Value::Gauge32(_) => ValueType::Unsigned32,
                    Value::Counter32(_) => ValueType::Counter32,
                    _ => ValueType::TimeTicks,
                };
                if expected != Some(sent) {
                    return wrong_type(sent.name());
                }
                self.check_ranges(i128::from(*n))
            }
            Value::Counter64(n) => {
                if expected != Some(ValueType::Counter64) {
                    return wrong_type("a Counter64");
                }
                self.check_ranges(i128::from(*n))
            }
            Value::IpAddress(_) if expected != Some(ValueType::IpAddress) => {
                wrong_type("an IpAddress")
            }
            Value::ObjectIdentifier(_) if expected != Some(ValueType::ObjectIdentifier) => {
                wrong_type("an OBJECT IDENTIFIER")
            }
            Value::OctetString(bytes) | Value::Opaque(bytes) => {
                if !matches!(
                    expected,
                    Some(ValueType::Text | ValueType::Hex | ValueType::Bits)
                ) {
                    return wrong_type("an OCTET STRING");
                }
                if syntax.base_type == BaseType::Bits {
                    return self.check_bits(bytes);
                }
                let length = bytes.len();
                if !in_ranges(&syntax.sizes, length as i128) {
                    let sizes = syntax
                        .sizes
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" | ");
                    return not_allowed(format!(
                        "{length} byte{} is not an allowed size (SIZE {sizes})",
                        if length == 1 { "" } else { "s" }
                    ));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn check_ranges(&self, n: i128) -> Result<(), InputError> {
        let syntax = self.syntax.as_ref().expect("checked by the caller");
        if in_ranges(&syntax.ranges, n) {
            return Ok(());
        }
        let ranges = syntax
            .ranges
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" | ");
        Err(InputError::NotAllowed {
            name: self.name.clone(),
            message: format!("{n} is outside the allowed range {ranges}"),
        })
    }

    fn check_bits(&self, bytes: &[u8]) -> Result<(), InputError> {
        let syntax = self.syntax.as_ref().expect("checked by the caller");
        for (i, byte) in bytes.iter().enumerate() {
            for bit in 0..8 {
                let position = (i * 8 + bit) as i64;
                if byte & (0x80 >> bit) != 0 && !syntax.bits.iter().any(|b| b.value == position) {
                    return Err(InputError::NotAllowed {
                        name: self.name.clone(),
                        message: format!(
                            "bit {position} is not one of the bits {}",
                            join(&syntax.bits)
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Everything [`Operation::Set`](crate::Operation::Set) needs: check
    /// that the instance can be written, read the text, check the value.
    /// With `checked` false only the reading is done, so a value the MIB
    /// does not allow can still be sent (to test an agent).
    pub fn build(
        &self,
        text: &str,
        value_type: Option<ValueType>,
        checked: bool,
    ) -> Result<Varbind, InputError> {
        if checked {
            self.check_writable()?;
        }
        let value = self.parse(text, value_type)?;
        if checked {
            self.check(&value)?;
        }
        Ok(Varbind {
            oid: self.oid.clone(),
            value,
        })
    }

    /// A value as text that [`parse`](Self::parse) reads back to the same
    /// value, with the type to read it as: to fill in a SET dialog with the
    /// current value. Binary strings come back as hex.
    pub fn edit_text(&self, value: &Value) -> (ValueType, String) {
        let syntax = self.syntax.as_ref();
        let hint = syntax.and_then(|s| s.display_hint.as_deref());
        let hex = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        match value {
            Value::Integer(n) => {
                let text = syntax
                    .and_then(|s| s.enums.iter().find(|e| e.value == *n))
                    .map(|e| e.label.clone())
                    .or_else(|| hint.and_then(|hint| display_hint::format_integer(hint, *n)))
                    .unwrap_or_else(|| n.to_string());
                (ValueType::Integer, text)
            }
            Value::Gauge32(n) => (ValueType::Unsigned32, n.to_string()),
            Value::Counter32(n) => (ValueType::Counter32, n.to_string()),
            Value::TimeTicks(n) => (ValueType::TimeTicks, n.to_string()),
            Value::Counter64(n) => (ValueType::Counter64, n.to_string()),
            Value::IpAddress(address) => {
                (ValueType::IpAddress, Ipv4Addr::from(*address).to_string())
            }
            Value::ObjectIdentifier(oid) => {
                let text = match &self.mibs {
                    Some(mibs) => {
                        let name = mibs.format_oid(oid);
                        // Only a name that reads back to the same OID.
                        match resolve_object(mibs, &name) {
                            Ok(back) if back == *oid => name,
                            _ => oid.to_string(),
                        }
                    }
                    None => oid.to_string(),
                };
                (ValueType::ObjectIdentifier, text)
            }
            Value::OctetString(bytes) if syntax.map(|s| s.base_type) == Some(BaseType::Bits) => {
                let names = syntax.map(|s| s.bits.as_slice()).unwrap_or_default();
                let mut words = Vec::new();
                for (i, byte) in bytes.iter().enumerate() {
                    for bit in 0..8 {
                        if byte & (0x80 >> bit) != 0 {
                            let position = (i * 8 + bit) as i64;
                            words.push(match names.iter().find(|b| b.value == position) {
                                Some(name) => name.label.clone(),
                                None => position.to_string(),
                            });
                        }
                    }
                }
                (ValueType::Bits, words.join(" "))
            }
            Value::OctetString(bytes) => {
                if let Some(hint) = hint.filter(|hint| !is_text_hint(hint))
                    && let Some(text) = display_hint::format_octets(hint, bytes)
                    && display_hint::parse_octets(hint, &text).ok().as_deref() == Some(bytes)
                {
                    return (ValueType::Text, text);
                }
                match std::str::from_utf8(bytes) {
                    Ok(text) if !text.chars().any(char::is_control) => {
                        (ValueType::Text, text.to_string())
                    }
                    _ => (ValueType::Hex, hex(bytes)),
                }
            }
            Value::Opaque(bytes) => (ValueType::Hex, hex(bytes)),
            _ => (self.value_type().unwrap_or(ValueType::Text), String::new()),
        }
    }
}

/// A display hint that shows the bytes as text, such as `255a`.
fn is_text_hint(hint: &str) -> bool {
    let hint = hint.trim();
    let digits = hint.trim_end_matches(['a', 't']);
    digits.len() + 1 == hint.len()
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
}

fn number<T: FromStr>(text: &str, type_name: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("{text:?} is not a number that fits in a {type_name}"))
}

fn join(values: &[mib_core::NamedNumber]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// True when `n` lies in one of the ranges, or there are none.
fn in_ranges(ranges: &[Range], n: i128) -> bool {
    let bound = |bound: &RangeBound| -> Option<i128> {
        match bound {
            RangeBound::Signed(v) => Some(i128::from(*v)),
            RangeBound::Unsigned(v) => Some(i128::from(*v)),
            RangeBound::Min | RangeBound::Max | RangeBound::Text(_) => None,
        }
    };
    ranges.is_empty()
        || ranges.iter().any(|range| {
            bound(&range.min).is_none_or(|min| n >= min)
                && bound(&range.max).is_none_or(|max| n <= max)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_parse_from_letters_and_names() {
        for value_type in ValueType::ALL {
            assert_eq!(
                value_type.letter().to_string().parse::<ValueType>(),
                Ok(value_type)
            );
        }
        assert_eq!("Gauge32".parse(), Ok(ValueType::Unsigned32));
        assert_eq!("OCTET STRING".parse(), Ok(ValueType::Text));
        assert_eq!("hex".parse(), Ok(ValueType::Hex));
        assert!("q".parse::<ValueType>().is_err());
    }

    #[test]
    fn text_hints() {
        assert!(is_text_hint("255a"));
        assert!(is_text_hint("255t"));
        assert!(!is_text_hint("1x:"));
        assert!(!is_text_hint("a"));
    }
}

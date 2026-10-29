//! Values as they arrive from an agent.

use std::fmt;

use mib_core::Oid;

/// One variable binding: an instance OID and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Varbind {
    /// The instance, such as `1.3.6.1.2.1.1.1.0`.
    pub oid: Oid,
    /// Its value.
    pub value: Value,
}

/// A value on the wire, by its ASN.1 type. What the value *means* (an enum
/// label, a display hint) comes from the MIB; see [`Formatter`](crate::Formatter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// `INTEGER` / `Integer32` (tag `0x02`).
    Integer(i64),
    /// `OCTET STRING` (tag `0x04`): text, binary data or BITS.
    OctetString(Vec<u8>),
    /// `NULL` (tag `0x05`).
    Null,
    /// `OBJECT IDENTIFIER` (tag `0x06`).
    ObjectIdentifier(Oid),
    /// `IpAddress` (tag `0x40`).
    IpAddress([u8; 4]),
    /// `Counter32` (tag `0x41`).
    Counter32(u32),
    /// `Gauge32` or `Unsigned32` (tag `0x42`; the two share it).
    Gauge32(u32),
    /// `TimeTicks` in hundredths of a second (tag `0x43`).
    TimeTicks(u32),
    /// `Opaque` (tag `0x44`): BER-encoded data in an octet string.
    Opaque(Vec<u8>),
    /// `Counter64` (tag `0x46`).
    Counter64(u64),
    /// The agent has no object at this OID (tag `0x80`, SNMPv2).
    NoSuchObject,
    /// The object exists but not this instance (tag `0x81`, SNMPv2).
    NoSuchInstance,
    /// Nothing comes after this OID (tag `0x82`, SNMPv2).
    EndOfMibView,
    /// Any other type, which SNMP does not use in variable bindings.
    Other {
        /// The ASN.1 tag.
        tag: u8,
        /// The content octets, when known.
        data: Vec<u8>,
    },
}

/// ASN.1 tags of the SNMP value types.
pub mod tag {
    /// `INTEGER`
    pub const INTEGER: u8 = 0x02;
    /// `OCTET STRING`
    pub const OCTET_STRING: u8 = 0x04;
    /// `NULL`
    pub const NULL: u8 = 0x05;
    /// `OBJECT IDENTIFIER`
    pub const OBJECT_IDENTIFIER: u8 = 0x06;
    /// `IpAddress`
    pub const IP_ADDRESS: u8 = 0x40;
    /// `Counter32`
    pub const COUNTER32: u8 = 0x41;
    /// `Gauge32` and `Unsigned32`
    pub const GAUGE32: u8 = 0x42;
    /// `TimeTicks`
    pub const TIME_TICKS: u8 = 0x43;
    /// `Opaque`
    pub const OPAQUE: u8 = 0x44;
    /// `Counter64`
    pub const COUNTER64: u8 = 0x46;
    /// `noSuchObject`
    pub const NO_SUCH_OBJECT: u8 = 0x80;
    /// `noSuchInstance`
    pub const NO_SUCH_INSTANCE: u8 = 0x81;
    /// `endOfMibView`
    pub const END_OF_MIB_VIEW: u8 = 0x82;
}

impl Value {
    /// The ASN.1 tag the value is sent with.
    pub fn tag(&self) -> u8 {
        match self {
            Value::Integer(_) => tag::INTEGER,
            Value::OctetString(_) => tag::OCTET_STRING,
            Value::Null => tag::NULL,
            Value::ObjectIdentifier(_) => tag::OBJECT_IDENTIFIER,
            Value::IpAddress(_) => tag::IP_ADDRESS,
            Value::Counter32(_) => tag::COUNTER32,
            Value::Gauge32(_) => tag::GAUGE32,
            Value::TimeTicks(_) => tag::TIME_TICKS,
            Value::Opaque(_) => tag::OPAQUE,
            Value::Counter64(_) => tag::COUNTER64,
            Value::NoSuchObject => tag::NO_SUCH_OBJECT,
            Value::NoSuchInstance => tag::NO_SUCH_INSTANCE,
            Value::EndOfMibView => tag::END_OF_MIB_VIEW,
            Value::Other { tag, .. } => *tag,
        }
    }

    /// The type name for the tag, such as `"OCTET STRING"`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Integer(_) => "INTEGER",
            Value::OctetString(_) => "OCTET STRING",
            Value::Null => "NULL",
            Value::ObjectIdentifier(_) => "OBJECT IDENTIFIER",
            Value::IpAddress(_) => "IpAddress",
            Value::Counter32(_) => "Counter32",
            Value::Gauge32(_) => "Gauge32",
            Value::TimeTicks(_) => "TimeTicks",
            Value::Opaque(_) => "Opaque",
            Value::Counter64(_) => "Counter64",
            Value::NoSuchObject => "noSuchObject",
            Value::NoSuchInstance => "noSuchInstance",
            Value::EndOfMibView => "endOfMibView",
            Value::Other { .. } => "unknown type",
        }
    }

    /// True for the three SNMPv2 exceptions (`noSuchObject`,
    /// `noSuchInstance`, `endOfMibView`), which stand in for a value.
    pub fn is_exception(&self) -> bool {
        matches!(
            self,
            Value::NoSuchObject | Value::NoSuchInstance | Value::EndOfMibView
        )
    }

    /// The BER content octets (without tag and length), for the raw view.
    ///
    /// This is the decoded value encoded again in the shortest form. It
    /// differs from the bytes received only if the agent used a longer
    /// encoding than needed, which BER allows but agents rarely do.
    pub fn content_octets(&self) -> Vec<u8> {
        match self {
            Value::Integer(n) => signed_octets(*n),
            Value::OctetString(data) | Value::Opaque(data) => data.clone(),
            Value::Null | Value::NoSuchObject | Value::NoSuchInstance | Value::EndOfMibView => {
                Vec::new()
            }
            Value::ObjectIdentifier(oid) => oid_octets(oid.arcs()),
            Value::IpAddress(address) => address.to_vec(),
            Value::Counter32(n) | Value::Gauge32(n) | Value::TimeTicks(n) => {
                unsigned_octets(u64::from(*n))
            }
            Value::Counter64(n) => unsigned_octets(*n),
            Value::Other { data, .. } => data.clone(),
        }
    }
}

impl fmt::Display for Value {
    /// The value without MIB information, such as `Counter32: 42`. Use a
    /// [`Formatter`](crate::Formatter) for enum labels and display hints.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::format::plain_value(self))
    }
}

/// Shortest two's complement encoding of an INTEGER.
pub(crate) fn signed_octets(n: i64) -> Vec<u8> {
    let bytes = n.to_be_bytes();
    let mut start = 0;
    // Drop a leading byte while the next one carries the same sign.
    while start < 7 {
        let (byte, next) = (bytes[start], bytes[start + 1]);
        if (byte == 0x00 && next & 0x80 == 0) || (byte == 0xff && next & 0x80 != 0) {
            start += 1;
        } else {
            break;
        }
    }
    bytes[start..].to_vec()
}

/// Shortest encoding of an unsigned type: a leading zero byte is added when
/// the top bit is set, so the value does not read as negative.
fn unsigned_octets(n: u64) -> Vec<u8> {
    let bytes = n.to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    let mut octets = Vec::with_capacity(9);
    if bytes[start] & 0x80 != 0 {
        octets.push(0);
    }
    octets.extend_from_slice(&bytes[start..]);
    octets
}

/// BER content octets of an OBJECT IDENTIFIER.
pub(crate) fn oid_octets(arcs: &[u32]) -> Vec<u8> {
    let mut octets = Vec::new();
    let mut push = |mut n: u64| {
        let mut chunk = vec![(n & 0x7f) as u8];
        n >>= 7;
        while n > 0 {
            chunk.push((n & 0x7f) as u8 | 0x80);
            n >>= 7;
        }
        octets.extend(chunk.iter().rev());
    };
    match arcs {
        [] => {}
        [first] => push(u64::from(*first) * 40),
        [first, second, rest @ ..] => {
            push(u64::from(*first) * 40 + u64::from(*second));
            for &arc in rest {
                push(u64::from(arc));
            }
        }
    }
    octets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_use_the_shortest_encoding() {
        assert_eq!(signed_octets(0), [0x00]);
        assert_eq!(signed_octets(127), [0x7f]);
        assert_eq!(signed_octets(128), [0x00, 0x80]);
        assert_eq!(signed_octets(-1), [0xff]);
        assert_eq!(signed_octets(-128), [0x80]);
        assert_eq!(signed_octets(-129), [0xff, 0x7f]);
        assert_eq!(signed_octets(-125), [0x83]);
    }

    #[test]
    fn unsigned_values_get_a_leading_zero_when_needed() {
        assert_eq!(unsigned_octets(0), [0x00]);
        assert_eq!(unsigned_octets(42), [0x2a]);
        assert_eq!(unsigned_octets(0xffff_ffff), [0x00, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(
            unsigned_octets(u64::MAX),
            [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
        );
    }

    #[test]
    fn object_identifiers() {
        // 1.3.6.1.4.1.8072 -> 2b 06 01 04 01 bf 08
        assert_eq!(
            oid_octets(&[1, 3, 6, 1, 4, 1, 8072]),
            [0x2b, 0x06, 0x01, 0x04, 0x01, 0xbf, 0x08]
        );
    }
}

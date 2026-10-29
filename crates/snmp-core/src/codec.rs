//! SNMPv1 and SNMPv2c messages: encoding requests and decoding responses
//! (RFC 1157, RFC 3416). snmp2's `AsnReader` reads the tag-length-value
//! structure; the values are decoded here, so that a value of an unknown or
//! unusual type is kept as [`Value::Other`] instead of cutting the response
//! short (see README.md, "What snmp2 0.5 does and does not do").

use mib_core::Oid;
use snmp2::AsnReader;

use crate::error::{Error, ErrorStatus};
use crate::session::{Request, Response};
use crate::target::Version;
use crate::value::{Value, Varbind, oid_octets, signed_octets, tag};

const SEQUENCE: u8 = 0x30;
const GET_REQUEST: u8 = 0xa0;
const GET_NEXT_REQUEST: u8 = 0xa1;
const RESPONSE: u8 = 0xa2;
const GET_BULK_REQUEST: u8 = 0xa5;

/// Tag, length and content.
pub(crate) fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 6);
    out.push(tag);
    let len = content.len();
    if len < 128 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let start = bytes
            .iter()
            .position(|&b| b != 0)
            .unwrap_or(bytes.len() - 1);
        out.push(0x80 | (bytes.len() - start) as u8);
        out.extend_from_slice(&bytes[start..]);
    }
    out.extend_from_slice(content);
    out
}

pub(crate) fn integer(n: i64) -> Vec<u8> {
    tlv(tag::INTEGER, &signed_octets(n))
}

/// Check that an OID can be encoded: at least two arcs, the first 0, 1 or
/// 2, and the second below 40 unless the first is 2 (X.690, 8.19.4).
pub(crate) fn check_oid(oid: &Oid) -> Result<(), Error> {
    let invalid = |reason: &str| {
        Err(Error::InvalidOid {
            oid: oid.to_string(),
            reason: reason.to_string(),
        })
    };
    match oid.arcs() {
        [] | [_] => invalid("an OID needs at least two arcs"),
        [first, ..] if *first > 2 => invalid("the first arc must be 0, 1 or 2"),
        [first, second, ..] if *first < 2 && *second >= 40 => {
            invalid("the second arc must be below 40 when the first is 0 or 1")
        }
        _ => Ok(()),
    }
}

/// The whole message for a request.
pub(crate) fn encode_request(
    version: Version,
    community: &[u8],
    request_id: i32,
    request: &Request,
) -> Result<Vec<u8>, Error> {
    let (pdu_tag, oids, a, b) = match request {
        Request::Get(oids) => (GET_REQUEST, oids.as_slice(), 0, 0),
        Request::GetNext(oid) => (GET_NEXT_REQUEST, std::slice::from_ref(oid), 0, 0),
        Request::GetBulk {
            oids,
            non_repeaters,
            max_repetitions,
        } => (
            GET_BULK_REQUEST,
            oids.as_slice(),
            *non_repeaters,
            *max_repetitions,
        ),
    };
    let mut list = Vec::new();
    for oid in oids {
        check_oid(oid)?;
        let mut varbind = tlv(tag::OBJECT_IDENTIFIER, &oid_octets(oid.arcs()));
        varbind.extend_from_slice(&[tag::NULL, 0]);
        list.extend(tlv(SEQUENCE, &varbind));
    }
    // GETBULK puts non-repeaters and max-repetitions where the other PDUs
    // have error-status and error-index.
    let mut pdu = integer(i64::from(request_id));
    pdu.extend(integer(i64::from(a)));
    pdu.extend(integer(i64::from(b)));
    pdu.extend(tlv(SEQUENCE, &list));

    let mut message = integer(match version {
        Version::V1 => 0,
        Version::V2c => 1,
    });
    message.extend(tlv(tag::OCTET_STRING, community));
    message.extend(tlv(pdu_tag, &pdu));
    Ok(tlv(SEQUENCE, &message))
}

/// Decode a Response message into its request-id and contents.
pub(crate) fn decode_response(bytes: &[u8]) -> Result<(i32, Response), String> {
    let asn = |e: snmp2::Error| e.to_string();
    let message = AsnReader::from_bytes(bytes)
        .read_raw(SEQUENCE)
        .map_err(asn)?;
    let mut message = AsnReader::from_bytes(message);
    let version = read_integer(&mut message)?;
    if !(0..=1).contains(&version) {
        return Err(format!(
            "SNMP version field {version}; expected SNMPv1 or SNMPv2c"
        ));
    }
    message.read_raw(tag::OCTET_STRING).map_err(asn)?;
    let pdu_tag = message.peek_byte().map_err(asn)?;
    if pdu_tag != RESPONSE {
        return Err(format!("expected a Response PDU, got tag 0x{pdu_tag:02x}"));
    }
    let pdu = message.read_raw(RESPONSE).map_err(asn)?;
    let mut pdu = AsnReader::from_bytes(pdu);
    let request_id = read_integer(&mut pdu)?;
    let status = read_integer(&mut pdu)?;
    let index = read_integer(&mut pdu)?;
    let request_id = i32::try_from(request_id).map_err(|_| "request-id out of range")?;
    let status = u32::try_from(status).map_err(|_| "error-status out of range")?;
    let index = u32::try_from(index).map_err(|_| "error-index out of range")?;

    let list = pdu.read_raw(SEQUENCE).map_err(asn)?;
    let mut list = AsnReader::from_bytes(list);
    let mut varbinds = Vec::new();
    while list.bytes_left() > 0 {
        let varbind = list.read_raw(SEQUENCE).map_err(asn)?;
        let mut varbind = AsnReader::from_bytes(varbind);
        let oid = decode_oid(varbind.read_raw(tag::OBJECT_IDENTIFIER).map_err(asn)?)?;
        let value_tag = varbind.peek_byte().map_err(asn)?;
        let content = varbind.read_raw(value_tag).map_err(asn)?;
        varbinds.push(Varbind {
            oid,
            value: decode_value(value_tag, content),
        });
    }
    Ok((
        request_id,
        Response {
            status: ErrorStatus::from_code(status),
            index,
            varbinds,
        },
    ))
}

fn read_integer(reader: &mut AsnReader<'_>) -> Result<i64, String> {
    let content = reader.read_raw(tag::INTEGER).map_err(|e| e.to_string())?;
    decode_signed(content).ok_or_else(|| "INTEGER of 0 or more than 8 bytes".to_string())
}

fn decode_signed(content: &[u8]) -> Option<i64> {
    if content.is_empty() || content.len() > 8 {
        return None;
    }
    let fill = if content[0] & 0x80 != 0 { 0xff } else { 0x00 };
    let mut bytes = [fill; 8];
    bytes[8 - content.len()..].copy_from_slice(content);
    Some(i64::from_be_bytes(bytes))
}

/// An unsigned value of at most `max_bytes` significant bytes. Leading zero
/// bytes are skipped, and a value sent without the leading zero its top bit
/// needs (`ff ff ff ff` for 4294967295, as some agents do) is read as the
/// unsigned number it was meant to be.
fn decode_unsigned(content: &[u8], max_bytes: usize) -> Option<u64> {
    let start = content
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(content.len());
    let significant = &content[start..];
    if content.is_empty() || significant.len() > max_bytes {
        return None;
    }
    Some(
        significant
            .iter()
            .fold(0u64, |n, &b| (n << 8) | u64::from(b)),
    )
}

fn decode_oid(content: &[u8]) -> Result<Oid, String> {
    let mut arcs = Vec::new();
    let mut value: u64 = 0;
    for (i, &byte) in content.iter().enumerate() {
        value = (value << 7) | u64::from(byte & 0x7f);
        if value > u64::from(u32::MAX) * 128 {
            return Err("OID arc too large".to_string());
        }
        if byte & 0x80 != 0 {
            if i + 1 == content.len() {
                return Err("OID ends in the middle of an arc".to_string());
            }
            continue;
        }
        if arcs.is_empty() {
            let (first, second) = match value {
                0..40 => (0, value),
                40..80 => (1, value - 40),
                _ => (2, value - 80),
            };
            arcs.push(first);
            arcs.push(u32::try_from(second).map_err(|_| "OID arc too large")?);
        } else {
            arcs.push(u32::try_from(value).map_err(|_| "OID arc too large")?);
        }
        value = 0;
    }
    if arcs.is_empty() {
        return Err("empty OID".to_string());
    }
    Ok(Oid::new(arcs))
}

/// A value from its tag and content. Never fails: anything that does not
/// fit its type is kept as [`Value::Other`] with its bytes.
fn decode_value(value_tag: u8, content: &[u8]) -> Value {
    let other = || Value::Other {
        tag: value_tag,
        data: content.to_vec(),
    };
    let unsigned32 =
        |make: fn(u32) -> Value| decode_unsigned(content, 4).map_or_else(other, |n| make(n as u32));
    match value_tag {
        tag::INTEGER => decode_signed(content).map_or_else(other, Value::Integer),
        tag::OCTET_STRING => Value::OctetString(content.to_vec()),
        tag::NULL => Value::Null,
        tag::OBJECT_IDENTIFIER => {
            decode_oid(content).map_or_else(|_| other(), Value::ObjectIdentifier)
        }
        tag::IP_ADDRESS => match <[u8; 4]>::try_from(content) {
            Ok(address) => Value::IpAddress(address),
            Err(_) => other(),
        },
        tag::COUNTER32 => unsigned32(Value::Counter32),
        tag::GAUGE32 => unsigned32(Value::Gauge32),
        tag::TIME_TICKS => unsigned32(Value::TimeTicks),
        tag::OPAQUE => Value::Opaque(content.to_vec()),
        tag::COUNTER64 => decode_unsigned(content, 8).map_or_else(other, Value::Counter64),
        tag::NO_SUCH_OBJECT => Value::NoSuchObject,
        tag::NO_SUCH_INSTANCE => Value::NoSuchInstance,
        tag::END_OF_MIB_VIEW => Value::EndOfMibView,
        _ => other(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(text: &str) -> Oid {
        text.parse().unwrap()
    }

    #[test]
    fn encodes_a_get_request() {
        let request = Request::Get(vec![oid("1.3.6.1.2.1.1.1.0")]);
        let bytes = encode_request(Version::V2c, b"public", 1, &request).unwrap();
        // What `snmpget -v2c -c public HOST 1.3.6.1.2.1.1.1.0` sends, with
        // request-id 1.
        let expected = [
            0x30, 0x26, 0x02, 0x01, 0x01, 0x04, 0x06, b'p', b'u', b'b', b'l', b'i', b'c', 0xa0,
            0x19, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x0e, 0x30, 0x0c,
            0x06, 0x08, 0x2b, 0x06, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x05, 0x00,
        ];
        assert_eq!(bytes, expected);
        let parsed = snmp2::Pdu::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.message_type, snmp2::MessageType::GetRequest);
        assert_eq!(parsed.req_id, 1);
        assert_eq!(parsed.community, b"public");
        let oids: Vec<String> = parsed.varbinds.map(|(o, _)| o.to_id_string()).collect();
        assert_eq!(oids, ["1.3.6.1.2.1.1.1.0"]);
    }

    #[test]
    fn encodes_getbulk_parameters() {
        let request = Request::GetBulk {
            oids: vec![oid("1.3.6.1.2.1.2"), oid("1.3.6.1.2.1.31")],
            non_repeaters: 1,
            max_repetitions: 25,
        };
        let bytes = encode_request(Version::V2c, b"x", 7, &request).unwrap();
        let parsed = snmp2::Pdu::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.message_type, snmp2::MessageType::GetBulkRequest);
        // snmp2 reads the two fields under their GET names.
        assert_eq!((parsed.error_status, parsed.error_index), (1, 25));
        assert_eq!(parsed.varbinds.count(), 2);
    }

    #[test]
    fn refuses_oids_that_cannot_be_encoded() {
        for text in ["1", "3.1", "1.40"] {
            assert!(check_oid(&oid(text)).is_err(), "{text}");
        }
        assert!(check_oid(&oid("2.999.1")).is_ok());
    }

    #[test]
    fn oids_round_trip() {
        for text in ["0.0", "1.3.6.1.4.1.8072.4294967295", "2.999.1", "1.39"] {
            assert_eq!(
                decode_oid(&oid_octets(oid(text).arcs())).unwrap(),
                oid(text)
            );
        }
        assert!(decode_oid(&[0x2b, 0x86]).is_err());
        assert!(decode_oid(&[0x2b, 0x90, 0x80, 0x80, 0x80, 0x80, 0x00]).is_err());
    }

    #[test]
    fn values_never_cut_a_response_short() {
        // Counter64 above 2^63 needs 9 bytes; snmp2's own decoder stops at it.
        assert_eq!(
            decode_value(
                tag::COUNTER64,
                &[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
            ),
            Value::Counter64(u64::MAX)
        );
        // Counter32 4294967295 without its leading zero byte.
        assert_eq!(
            decode_value(tag::COUNTER32, &[0xff, 0xff, 0xff, 0xff]),
            Value::Counter32(u32::MAX)
        );
        assert_eq!(decode_value(tag::INTEGER, &[0x83]), Value::Integer(-125));
        // Unknown types and malformed contents are kept as they are.
        assert_eq!(
            decode_value(0x45, &[1, 2]),
            Value::Other {
                tag: 0x45,
                data: vec![1, 2]
            }
        );
        assert_eq!(
            decode_value(tag::IP_ADDRESS, &[1, 2, 3]),
            Value::Other {
                tag: tag::IP_ADDRESS,
                data: vec![1, 2, 3]
            }
        );
        assert_eq!(
            decode_value(tag::GAUGE32, &[1, 0, 0, 0, 0]),
            Value::Other {
                tag: tag::GAUGE32,
                data: vec![1, 0, 0, 0, 0]
            }
        );
    }

    #[test]
    fn decodes_a_response() {
        let mut list = Vec::new();
        for (oid_text, value) in [
            ("1.3.6.1.2.1.1.5.0", Value::OctetString(b"host".to_vec())),
            ("1.3.6.1.2.1.31.1.1.1.6.1", Value::Counter64(u64::MAX)),
            (
                "1.3.6.1.2.1.1.6.0",
                Value::Other {
                    tag: 0x45,
                    data: vec![9],
                },
            ),
            ("1.3.6.1.2.1.1.7.0", Value::Integer(72)),
        ] {
            let mut varbind = tlv(tag::OBJECT_IDENTIFIER, &oid_octets(oid(oid_text).arcs()));
            varbind.extend(tlv(value.tag(), &value.content_octets()));
            list.extend(tlv(SEQUENCE, &varbind));
        }
        let mut pdu = integer(1234);
        pdu.extend(integer(0));
        pdu.extend(integer(0));
        pdu.extend(tlv(SEQUENCE, &list));
        let mut message = integer(1);
        message.extend(tlv(tag::OCTET_STRING, b"public"));
        message.extend(tlv(RESPONSE, &pdu));
        let bytes = tlv(SEQUENCE, &message);

        let (request_id, response) = decode_response(&bytes).unwrap();
        assert_eq!(request_id, 1234);
        assert_eq!(response.status, ErrorStatus::NoError);
        assert_eq!(response.varbinds.len(), 4);
        assert_eq!(response.varbinds[1].value, Value::Counter64(u64::MAX));
        assert_eq!(response.varbinds[3].value, Value::Integer(72));

        assert!(decode_response(&bytes[..bytes.len() - 1]).is_err());
        assert!(decode_response(b"junk").is_err());
    }
}

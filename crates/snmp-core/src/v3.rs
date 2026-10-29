//! SNMPv3 messages (RFC 3412) with User-based Security Model parameters
//! (RFC 3414): building authenticated and encrypted requests, and checking,
//! decrypting and reading what comes back.

use std::ops::Range;

use mib_core::Oid;
use snmp2::AsnReader;

use crate::codec::{self, SEQUENCE, integer, read_integer, tlv};
use crate::error::SecurityProblem;
use crate::session::{Request, Response};
use crate::usm::{Keys, Usm};
use crate::value::tag;

const FLAG_AUTH: u8 = 0x01;
const FLAG_PRIV: u8 = 0x02;
const FLAG_REPORTABLE: u8 = 0x04;
/// `msgSecurityModel` of USM.
const USM_MODEL: i64 = 3;
/// `msgMaxSize`: the largest message we accept (a UDP datagram over IPv4).
const MAX_SIZE: i64 = 65_507;

/// Where an agent's engine is, as far as we know: the authoritative engine
/// ID, boots and time from its last message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EngineTime {
    pub engine_id: Vec<u8>,
    pub boots: u32,
    pub time: u32,
}

/// The parts of a request that change for every message sent.
#[derive(Debug)]
pub(crate) struct Outgoing<'a> {
    pub msg_id: i32,
    pub request_id: i32,
    pub request: &'a Request,
    /// `None` for discovery: empty engine ID and user, no security.
    pub engine: Option<&'a EngineTime>,
    /// The privacy salt; different for every message.
    pub salt: u64,
}

/// Build a request message.
pub(crate) fn encode_request(
    usm: &Usm,
    keys: Option<&Keys>,
    outgoing: &Outgoing<'_>,
) -> Result<Vec<u8>, crate::Error> {
    let pdu = codec::encode_pdu(outgoing.request_id, outgoing.request)?;
    let Some(engine) = outgoing.engine else {
        // Discovery (RFC 3414, 4): noAuthNoPriv, no user, no engine ID.
        let scoped = scoped_pdu(&[], b"", &pdu);
        return Ok(message(
            outgoing.msg_id,
            FLAG_REPORTABLE,
            &security_parameters(&EngineTime::empty(), b"", &[], &[]),
            &scoped,
        ));
    };
    let keys = keys.expect("keys are derived once the engine is known");
    let context_engine = usm
        .context_engine_id
        .as_deref()
        .unwrap_or(&engine.engine_id);
    let scoped = scoped_pdu(context_engine, usm.context_name.as_bytes(), &pdu);

    let mut flags = FLAG_REPORTABLE;
    let (data, privacy) = if usm.level.has_priv() {
        flags |= FLAG_PRIV;
        let (ciphertext, salt) = usm.priv_protocol.encrypt(
            &keys.privacy,
            engine.boots,
            engine.time,
            outgoing.salt,
            &scoped,
        );
        (tlv(tag::OCTET_STRING, &ciphertext), salt)
    } else {
        (scoped, Vec::new())
    };
    let auth_placeholder = if usm.level.has_auth() {
        flags |= FLAG_AUTH;
        vec![0; usm.auth_protocol.mac_len()]
    } else {
        Vec::new()
    };
    let mut bytes = message(
        outgoing.msg_id,
        flags,
        &security_parameters(engine, usm.user.as_bytes(), &auth_placeholder, &privacy),
        &data,
    );
    if usm.level.has_auth() {
        // RFC 3414, 6.3.1: the code covers the whole message, with zeros
        // where the code goes.
        let range = decode_message(&bytes)
            .expect("a message we built can be read back")
            .auth_range;
        let code = usm.auth_protocol.mac(&keys.auth, &bytes);
        bytes[range].copy_from_slice(&code);
    }
    Ok(bytes)
}

impl EngineTime {
    fn empty() -> EngineTime {
        EngineTime {
            engine_id: Vec::new(),
            boots: 0,
            time: 0,
        }
    }
}

fn scoped_pdu(context_engine_id: &[u8], context_name: &[u8], pdu: &[u8]) -> Vec<u8> {
    let mut content = tlv(tag::OCTET_STRING, context_engine_id);
    content.extend(tlv(tag::OCTET_STRING, context_name));
    content.extend_from_slice(pdu);
    tlv(SEQUENCE, &content)
}

fn security_parameters(engine: &EngineTime, user: &[u8], auth: &[u8], privacy: &[u8]) -> Vec<u8> {
    let mut content = tlv(tag::OCTET_STRING, &engine.engine_id);
    content.extend(integer(i64::from(engine.boots)));
    content.extend(integer(i64::from(engine.time)));
    content.extend(tlv(tag::OCTET_STRING, user));
    content.extend(tlv(tag::OCTET_STRING, auth));
    content.extend(tlv(tag::OCTET_STRING, privacy));
    tlv(SEQUENCE, &content)
}

fn message(msg_id: i32, flags: u8, security: &[u8], data: &[u8]) -> Vec<u8> {
    let mut global = integer(i64::from(msg_id));
    global.extend(integer(MAX_SIZE));
    global.extend(tlv(tag::OCTET_STRING, &[flags]));
    global.extend(integer(USM_MODEL));

    let mut content = integer(3);
    content.extend(tlv(SEQUENCE, &global));
    content.extend(tlv(tag::OCTET_STRING, security));
    content.extend_from_slice(data);
    tlv(SEQUENCE, &content)
}

/// A received message, before its security is checked.
#[derive(Debug)]
struct Message<'a> {
    msg_id: i32,
    flags: u8,
    engine: EngineTime,
    user: &'a [u8],
    auth: &'a [u8],
    /// Where `auth` is in the message.
    auth_range: Range<usize>,
    privacy: &'a [u8],
    /// The ScopedPDU (whole TLV) or, when encrypted, the ciphertext.
    data: &'a [u8],
}

fn decode_message(bytes: &[u8]) -> Result<Message<'_>, String> {
    let asn = |e: snmp2::Error| e.to_string();
    let offset = |part: &[u8]| part.as_ptr() as usize - bytes.as_ptr() as usize;

    let content = AsnReader::from_bytes(bytes)
        .read_raw(SEQUENCE)
        .map_err(asn)?;
    let mut reader = AsnReader::from_bytes(content);
    let version = read_integer(&mut reader)?;
    if version != 3 {
        return Err(format!("SNMP version field {version}; expected 3 (SNMPv3)"));
    }
    let global = reader.read_raw(SEQUENCE).map_err(asn)?;
    let mut global = AsnReader::from_bytes(global);
    let msg_id = read_integer(&mut global)?;
    let _max_size = read_integer(&mut global)?;
    let flags = global.read_raw(tag::OCTET_STRING).map_err(asn)?;
    let model = read_integer(&mut global)?;
    if model != USM_MODEL {
        return Err(format!(
            "security model {model}; only the User-based Security Model (3) is supported"
        ));
    }
    let [flags] = flags else {
        return Err("msgFlags is not one byte".to_string());
    };

    let security = reader.read_raw(tag::OCTET_STRING).map_err(asn)?;
    let security = AsnReader::from_bytes(security)
        .read_raw(SEQUENCE)
        .map_err(asn)?;
    let mut security = AsnReader::from_bytes(security);
    let engine_id = security.read_raw(tag::OCTET_STRING).map_err(asn)?;
    let boots = read_integer(&mut security)?;
    let time = read_integer(&mut security)?;
    let user = security.read_raw(tag::OCTET_STRING).map_err(asn)?;
    let auth = security.read_raw(tag::OCTET_STRING).map_err(asn)?;
    let privacy = security.read_raw(tag::OCTET_STRING).map_err(asn)?;
    let range = |n: i64| u32::try_from(n).map_err(|_| "engine boots or time out of range");

    let data = if flags & FLAG_PRIV != 0 {
        reader.read_raw(tag::OCTET_STRING).map_err(asn)?
    } else {
        // The rest of the message is the ScopedPDU.
        &content[content.len() - reader.bytes_left()..]
    };

    Ok(Message {
        msg_id: i32::try_from(msg_id).map_err(|_| "msgID out of range")?,
        flags: *flags,
        engine: EngineTime {
            engine_id: engine_id.to_vec(),
            boots: range(boots)?,
            time: range(time)?,
        },
        user,
        auth,
        auth_range: offset(auth)..offset(auth) + auth.len(),
        privacy,
        data,
    })
}

/// A received message, checked and decrypted.
#[derive(Debug)]
pub(crate) struct Received {
    /// The agent's engine ID, boots and time from the message.
    pub engine: EngineTime,
    /// Whether the message's authentication code was checked and correct.
    pub authenticated: bool,
    /// The PDU type: Response or Report.
    pub pdu_tag: u8,
    pub request_id: i32,
    pub response: Response,
}

impl Received {
    pub fn is_report(&self) -> bool {
        self.pdu_tag == codec::REPORT
    }
}

/// Why a received message could not be read.
#[derive(Debug)]
pub(crate) enum DecodeError {
    /// Not a valid SNMPv3 message.
    Malformed(String),
    /// A security check failed.
    Security(SecurityProblem),
}

/// Read a message, checking its authentication code and decrypting it when
/// `keys` are known. `msg_ids` are the ones we sent; a message with another
/// one is not checked further and returns `Ok(None)`.
pub(crate) fn decode_received(
    bytes: &[u8],
    usm: &Usm,
    keys: Option<&Keys>,
    msg_ids: &[i32],
) -> Result<Option<Received>, DecodeError> {
    let message = decode_message(bytes).map_err(DecodeError::Malformed)?;
    if !msg_ids.contains(&message.msg_id) {
        return Ok(None);
    }
    let mut authenticated = false;
    if message.flags & FLAG_AUTH != 0
        && let Some(keys) = keys.filter(|_| usm.level.has_auth())
    {
        if message.user != usm.user.as_bytes() || message.auth.len() != usm.auth_protocol.mac_len()
        {
            return Err(DecodeError::Security(SecurityProblem::ResponseWrongDigest));
        }
        let mut zeroed = bytes.to_vec();
        zeroed[message.auth_range.clone()].fill(0);
        let expected = usm.auth_protocol.mac(&keys.auth, &zeroed);
        if !same(&expected, message.auth) {
            return Err(DecodeError::Security(SecurityProblem::ResponseWrongDigest));
        }
        authenticated = true;
    }

    let plaintext;
    let scoped = if message.flags & FLAG_PRIV != 0 {
        let Some(keys) = keys.filter(|_| authenticated && usm.level.has_priv()) else {
            return Err(DecodeError::Security(SecurityProblem::ResponseDecryption(
                "the response is encrypted, but privacy was not asked for".to_string(),
            )));
        };
        plaintext = usm
            .priv_protocol
            .decrypt(
                &keys.privacy,
                message.engine.boots,
                message.engine.time,
                message.privacy,
                message.data,
            )
            .map_err(|e| DecodeError::Security(SecurityProblem::ResponseDecryption(e)))?;
        plaintext.as_slice()
    } else {
        message.data
    };

    let (pdu_tag, request_id, response) = decode_scoped_pdu(scoped).map_err(|error| {
        if message.flags & FLAG_PRIV != 0 {
            // Garbage after decryption: the keys differ.
            DecodeError::Security(SecurityProblem::ResponseDecryption(format!(
                "the decrypted data is not a valid PDU ({error})"
            )))
        } else {
            DecodeError::Malformed(error)
        }
    })?;
    Ok(Some(Received {
        engine: message.engine,
        authenticated,
        pdu_tag,
        request_id,
        response,
    }))
}

fn decode_scoped_pdu(bytes: &[u8]) -> Result<(u8, i32, Response), String> {
    let asn = |e: snmp2::Error| e.to_string();
    // Anything after the ScopedPDU is DES padding.
    let content = AsnReader::from_bytes(bytes)
        .read_raw(SEQUENCE)
        .map_err(asn)?;
    let mut reader = AsnReader::from_bytes(content);
    reader.read_raw(tag::OCTET_STRING).map_err(asn)?;
    reader.read_raw(tag::OCTET_STRING).map_err(asn)?;
    codec::decode_pdu(&mut reader)
}

/// Compare authentication codes without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// What a Report PDU says went wrong, from the counter it carries.
pub(crate) fn report_problem(response: &Response) -> SecurityProblem {
    let Some(varbind) = response.varbinds.first() else {
        return SecurityProblem::OtherReport(None);
    };
    let usm_stats: Oid = "1.3.6.1.6.3.15.1.1".parse().expect("valid OID");
    let mpd_stats: Oid = "1.3.6.1.6.3.11.2.1".parse().expect("valid OID");
    let target_mib: Oid = "1.3.6.1.6.3.12.1".parse().expect("valid OID");
    let arcs = varbind.oid.arcs();
    let leaf = |prefix: &Oid| {
        varbind
            .oid
            .starts_with(prefix)
            .then(|| arcs.get(prefix.len()).copied())
            .flatten()
    };
    match (leaf(&usm_stats), leaf(&mpd_stats), leaf(&target_mib)) {
        (Some(1), _, _) => SecurityProblem::UnsupportedSecurityLevel,
        (Some(2), _, _) => SecurityProblem::NotInTimeWindow,
        (Some(3), _, _) => SecurityProblem::UnknownUser,
        (Some(4), _, _) => SecurityProblem::UnknownEngineId,
        (Some(5), _, _) => SecurityProblem::WrongDigest,
        (Some(6), _, _) => SecurityProblem::DecryptionError,
        (_, Some(1), _) => SecurityProblem::UnknownSecurityModel,
        (_, Some(2), _) => SecurityProblem::InvalidMessage,
        (_, Some(3), _) => SecurityProblem::UnknownPduHandler,
        (_, _, Some(4)) => SecurityProblem::UnavailableContext,
        (_, _, Some(5)) => SecurityProblem::UnknownContext,
        _ => SecurityProblem::OtherReport(Some(varbind.oid.clone())),
    }
}

/// What a fake agent in the tests needs to know about a request without
/// privacy.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Peeked {
    pub msg_id: i32,
    pub engine: EngineTime,
    pub user: Vec<u8>,
    pub request_id: i32,
}

#[cfg(test)]
pub(crate) fn peek_request(bytes: &[u8]) -> Peeked {
    let message = decode_message(bytes).unwrap();
    let content = AsnReader::from_bytes(message.data)
        .read_raw(SEQUENCE)
        .unwrap();
    let mut reader = AsnReader::from_bytes(content);
    reader.read_raw(tag::OCTET_STRING).unwrap();
    reader.read_raw(tag::OCTET_STRING).unwrap();
    let pdu_tag = reader.peek_byte().unwrap();
    let pdu = reader.read_raw(pdu_tag).unwrap();
    let request_id = read_integer(&mut AsnReader::from_bytes(pdu)).unwrap();
    Peeked {
        msg_id: message.msg_id,
        engine: message.engine,
        user: message.user.to_vec(),
        request_id: request_id as i32,
    }
}

/// An answer from a fake agent: authenticated with `keys` when given,
/// never encrypted.
#[cfg(test)]
pub(crate) fn encode_answer(
    usm: &Usm,
    keys: Option<&Keys>,
    engine: &EngineTime,
    msg_id: i32,
    pdu_tag: u8,
    request_id: i32,
    varbinds: &[(Oid, crate::Value)],
) -> Vec<u8> {
    let mut list = Vec::new();
    for (oid, value) in varbinds {
        let mut varbind = tlv(
            tag::OBJECT_IDENTIFIER,
            &crate::value::oid_octets(oid.arcs()),
        );
        varbind.extend(tlv(value.tag(), &value.content_octets()));
        list.extend(tlv(SEQUENCE, &varbind));
    }
    let mut pdu = integer(i64::from(request_id));
    pdu.extend(integer(0));
    pdu.extend(integer(0));
    pdu.extend(tlv(SEQUENCE, &list));
    let scoped = scoped_pdu(&engine.engine_id, b"", &tlv(pdu_tag, &pdu));
    let auth = keys.map_or(Vec::new(), |_| vec![0; usm.auth_protocol.mac_len()]);
    let flags = if keys.is_some() { FLAG_AUTH } else { 0 };
    let mut bytes = message(
        msg_id,
        flags,
        &security_parameters(engine, usm.user.as_bytes(), &auth, &[]),
        &scoped,
    );
    if let Some(keys) = keys {
        let range = decode_message(&bytes).unwrap().auth_range;
        let code = usm.auth_protocol.mac(&keys.auth, &bytes);
        bytes[range].copy_from_slice(&code);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usm::{AuthProtocol, PrivProtocol};
    use crate::value::{Value, Varbind};

    fn engine() -> EngineTime {
        EngineTime {
            engine_id: vec![0x80, 0, 0x1f, 0x88, 4, 1, 2, 3],
            boots: 5,
            time: 1000,
        }
    }

    /// Turn a request we built into the response an agent would send, with
    /// the same security.
    fn as_response(usm: &Usm, keys: &Keys, request: &[u8], value: Value) -> Vec<u8> {
        let message = decode_message(request).unwrap();
        let engine = message.engine.clone();
        let mut pdu = integer(77);
        pdu.extend(integer(0));
        pdu.extend(integer(0));
        let mut varbind = tlv(tag::OBJECT_IDENTIFIER, &[0x2b, 6, 1, 2, 1, 1, 1, 0]);
        varbind.extend(tlv(value.tag(), &value.content_octets()));
        pdu.extend(tlv(SEQUENCE, &tlv(SEQUENCE, &varbind)));
        let scoped = scoped_pdu(&engine.engine_id, b"", &tlv(codec::RESPONSE, &pdu));
        let (data, privacy) = if usm.level.has_priv() {
            let (c, s) =
                usm.priv_protocol
                    .encrypt(&keys.privacy, engine.boots, engine.time, 4242, &scoped);
            (tlv(tag::OCTET_STRING, &c), s)
        } else {
            (scoped, vec![])
        };
        let mut bytes = super::message(
            message.msg_id,
            message.flags & !FLAG_REPORTABLE,
            &security_parameters(
                &engine,
                usm.user.as_bytes(),
                &vec![0; message.auth.len()],
                &privacy,
            ),
            &data,
        );
        if usm.level.has_auth() {
            let range = decode_message(&bytes).unwrap().auth_range;
            let code = usm.auth_protocol.mac(&keys.auth, &bytes);
            bytes[range].copy_from_slice(&code);
        }
        bytes
    }

    #[test]
    fn requests_and_responses_round_trip_at_every_level() {
        let request = Request::Get(vec!["1.3.6.1.2.1.1.1.0".parse().unwrap()]);
        let engine = engine();
        let users = [
            Usm::new("plain"),
            Usm::new("authonly").with_auth(AuthProtocol::Sha384, "password1"),
            Usm::new("des")
                .with_auth(AuthProtocol::Md5, "password1")
                .with_privacy(PrivProtocol::Des, "password2"),
            Usm::new("aes")
                .with_auth(AuthProtocol::Sha1, "password1")
                .with_privacy(PrivProtocol::Aes256Cisco, "password2"),
        ];
        for usm in users {
            let keys = Keys::derive(&usm, &engine.engine_id);
            let outgoing = Outgoing {
                msg_id: 9,
                request_id: 77,
                request: &request,
                engine: Some(&engine),
                salt: 1,
            };
            let bytes = encode_request(&usm, Some(&keys), &outgoing).unwrap();
            let parsed = decode_message(&bytes).unwrap();
            assert_eq!(parsed.user, usm.user.as_bytes());
            assert_eq!(parsed.flags & FLAG_AUTH != 0, usm.level.has_auth());

            let answer = as_response(&usm, &keys, &bytes, Value::OctetString(b"ok".to_vec()));
            let received = decode_received(&answer, &usm, Some(&keys), &[9])
                .unwrap()
                .unwrap();
            assert_eq!(received.request_id, 77, "{}", usm.summary());
            assert_eq!(received.authenticated, usm.level.has_auth());
            assert_eq!(
                received.response.varbinds,
                [Varbind {
                    oid: "1.3.6.1.2.1.1.1.0".parse().unwrap(),
                    value: Value::OctetString(b"ok".to_vec())
                }]
            );
            // Other message IDs are not ours.
            assert!(
                decode_received(&answer, &usm, Some(&keys), &[10])
                    .unwrap()
                    .is_none()
            );

            if usm.level.has_auth() {
                // One flipped bit anywhere fails the check.
                let mut tampered = answer.clone();
                let last = tampered.len() - 1;
                tampered[last] ^= 1;
                assert!(matches!(
                    decode_received(&tampered, &usm, Some(&keys), &[9]),
                    Err(DecodeError::Security(SecurityProblem::ResponseWrongDigest))
                ));
                // So does another password.
                let other = Usm {
                    auth_password: "different".into(),
                    ..usm.clone()
                };
                let other_keys = Keys::derive(&other, &engine.engine_id);
                assert!(decode_received(&answer, &other, Some(&other_keys), &[9]).is_err());
            }
        }
    }

    #[test]
    fn discovery_message() {
        let request = Request::Get(vec![]);
        let outgoing = Outgoing {
            msg_id: 1,
            request_id: 2,
            request: &request,
            engine: None,
            salt: 0,
        };
        let bytes = encode_request(&Usm::new("x"), None, &outgoing).unwrap();
        let message = decode_message(&bytes).unwrap();
        assert_eq!(message.flags, FLAG_REPORTABLE);
        assert!(message.engine.engine_id.is_empty());
        assert!(message.user.is_empty());
    }

    #[test]
    fn reports_are_classified() {
        let report = |oid: &str| Response {
            status: crate::ErrorStatus::NoError,
            index: 0,
            varbinds: vec![Varbind {
                oid: oid.parse().unwrap(),
                value: Value::Counter32(1),
            }],
        };
        assert_eq!(
            report_problem(&report("1.3.6.1.6.3.15.1.1.3.0")),
            SecurityProblem::UnknownUser
        );
        assert_eq!(
            report_problem(&report("1.3.6.1.6.3.15.1.1.5.0")),
            SecurityProblem::WrongDigest
        );
        assert_eq!(
            report_problem(&report("1.3.6.1.6.3.12.1.5.0")),
            SecurityProblem::UnknownContext
        );
        assert!(matches!(
            report_problem(&report("1.3.6.1.4.1.9.1")),
            SecurityProblem::OtherReport(Some(_))
        ));
    }
}

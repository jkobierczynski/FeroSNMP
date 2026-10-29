//! A tiny SNMPv1/v2c agent that serves one big made-up table, for trying
//! walks of 100,000 rows and more (the GUI's Results list, timing, Cancel).
//!
//! ```text
//! cargo run --release -p snmp-core --example big_agent -- [ROWS] [ADDRESS]
//! ```
//!
//! ROWS defaults to 100000 and ADDRESS to 127.0.0.1:11162. Any community is
//! accepted. The table is at 1.3.6.1.4.1.99999.1.1 with three columns:
//!
//! - `.1.1.1.1.N` INTEGER N (the index),
//! - `.1.1.1.2.N` OCTET STRING `port N`,
//! - `.1.1.1.3.N` Counter64 N * 1000.
//!
//! So walking `1.3.6.1.4.1.99999.1.1.1.2` returns ROWS values, and walking
//! `1.3.6.1.4.1.99999` three times as many. Only for testing: it answers
//! GET, GETNEXT and GETBULK and nothing else.

use std::net::UdpSocket;

use mib_core::Oid;
use snmp_core::{Value, tag};

const TABLE: [u32; 10] = [1, 3, 6, 1, 4, 1, 99999, 1, 1, 1];

fn main() {
    let mut args = std::env::args().skip(1);
    let rows: u32 = args
        .next()
        .map(|n| n.parse().expect("ROWS must be a number"))
        .unwrap_or(100_000);
    let address = args.next().unwrap_or_else(|| "127.0.0.1:11162".to_string());
    let socket = UdpSocket::bind(&address).expect("cannot listen on the address");
    eprintln!("big_agent: {rows} rows at 1.3.6.1.4.1.99999.1.1 on {address} (Ctrl+C to stop)");
    let mut buffer = vec![0u8; 65_535];
    loop {
        let Ok((len, peer)) = socket.recv_from(&mut buffer) else {
            continue;
        };
        if let Some(reply) = answer(&buffer[..len], rows) {
            let _ = socket.send_to(&reply, peer);
        }
    }
}

/// The value at an instance of the table.
fn value(column: u32, row: u32) -> Value {
    match column {
        1 => Value::Integer(i64::from(row)),
        2 => Value::OctetString(format!("port {row}").into_bytes()),
        _ => Value::Counter64(u64::from(row) * 1000),
    }
}

/// The instance at `arcs`, if it is one: (column, row).
fn instance(arcs: &[u32], rows: u32) -> Option<(u32, u32)> {
    match arcs.strip_prefix(&TABLE[..])? {
        &[column, row] if (1..=3).contains(&column) && (1..=rows).contains(&row) => {
            Some((column, row))
        }
        _ => None,
    }
}

/// The first instance after `arcs`, in OID order.
fn next(arcs: &[u32], rows: u32) -> Option<(u32, u32)> {
    let first_of = |column: u32| (column <= 3 && rows > 0).then_some((column.max(1), 1));
    let table = &TABLE[..];
    if arcs < table {
        return first_of(1);
    }
    let suffix = arcs.strip_prefix(table)?; // None: after the table
    match *suffix {
        [] => first_of(1),
        [column] => first_of(column),
        [0, ..] => first_of(1),
        // `.2.5` and `.2.5.1` are both followed by `.2.6`.
        [column, row, ..] if row < rows => (column <= 3).then_some((column, row + 1)),
        [column, ..] => first_of(column + 1),
    }
}

fn answer(bytes: &[u8], rows: u32) -> Option<Vec<u8>> {
    let pdu = snmp2::Pdu::from_bytes(bytes).ok()?;
    let v1 = pdu.version().ok()? == snmp2::Version::V1;
    let requested: Vec<Vec<u32>> = pdu
        .varbinds
        .clone()
        .map(|(oid, _)| oid.iter().map(|arcs| arcs.map(|a| a as u32).collect()))
        .collect::<Option<_>>()?;
    let at = |(column, row): (u32, u32)| {
        let mut arcs = TABLE.to_vec();
        arcs.extend([column, row]);
        (arcs, value(column, row))
    };
    let mut varbinds = Vec::new();
    let mut status = 0;
    let mut index = 0;
    match pdu.message_type {
        snmp2::MessageType::GetRequest => {
            for (i, arcs) in requested.iter().enumerate() {
                match instance(arcs, rows) {
                    Some(found) => varbinds.push(at(found)),
                    None if v1 => {
                        (status, index) = (2, i + 1);
                        break;
                    }
                    None => varbinds.push((arcs.clone(), Value::NoSuchObject)),
                }
            }
        }
        snmp2::MessageType::GetNextRequest => {
            for (i, arcs) in requested.iter().enumerate() {
                match next(arcs, rows) {
                    Some(found) => varbinds.push(at(found)),
                    None if v1 => {
                        (status, index) = (2, i + 1);
                        break;
                    }
                    None => varbinds.push((arcs.clone(), Value::EndOfMibView)),
                }
            }
        }
        snmp2::MessageType::GetBulkRequest => {
            let (non_repeaters, repetitions) = (pdu.error_status as usize, pdu.error_index);
            let split = non_repeaters.min(requested.len());
            for arcs in &requested[..split] {
                varbinds.push(next(arcs, rows).map_or((arcs.clone(), Value::EndOfMibView), at));
            }
            let mut cursors: Vec<Vec<u32>> = requested[split..].to_vec();
            // Keep responses well under the usual 1472-byte UDP payload
            // limit of a real network: about 40 bytes per value.
            for _ in 0..repetitions.min(30) {
                for cursor in &mut cursors {
                    match next(cursor, rows) {
                        Some(found) => {
                            let (arcs, value) = at(found);
                            *cursor = arcs.clone();
                            varbinds.push((arcs, value));
                        }
                        None => varbinds.push((cursor.clone(), Value::EndOfMibView)),
                    }
                }
            }
        }
        _ => return None,
    }
    if status != 0 {
        varbinds = requested
            .into_iter()
            .map(|arcs| (arcs, Value::Null))
            .collect();
    }
    Some(response(
        if v1 { 0 } else { 1 },
        pdu.community,
        pdu.req_id,
        status,
        index,
        &varbinds,
    ))
}

fn response(
    version: i64,
    community: &[u8],
    request_id: i32,
    status: u32,
    index: usize,
    varbinds: &[(Vec<u32>, Value)],
) -> Vec<u8> {
    let mut list = Vec::new();
    for (arcs, value) in varbinds {
        let oid = Value::ObjectIdentifier(Oid::new(arcs.clone()));
        let mut pair = tlv(tag::OBJECT_IDENTIFIER, &oid.content_octets());
        pair.extend(tlv(value.tag(), &value.content_octets()));
        list.extend(tlv(0x30, &pair));
    }
    let mut pdu = integer(i64::from(request_id));
    pdu.extend(integer(i64::from(status)));
    pdu.extend(integer(index as i64));
    pdu.extend(tlv(0x30, &list));
    let mut message = integer(version);
    message.extend(tlv(tag::OCTET_STRING, community));
    message.extend(tlv(0xa2, &pdu));
    tlv(0x30, &message)
}

fn integer(n: i64) -> Vec<u8> {
    tlv(tag::INTEGER, &Value::Integer(n).content_octets())
}

/// Tag, BER length, content.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes: Vec<u8> = len
            .to_be_bytes()
            .into_iter()
            .skip_while(|&b| b == 0)
            .collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

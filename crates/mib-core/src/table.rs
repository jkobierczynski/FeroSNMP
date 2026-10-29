//! Tables: which columns a table has, and the INDEX codec that turns the
//! instance suffix of a table cell into index values and back (RFC 2578,
//! section 7.7).
//!
//! The suffix of `ifDescr.3` is `[3]`; the suffix of
//! `ipNetToMediaPhysAddress.2.192.0.2.1` is `[2, 192, 0, 2, 1]`, which the
//! INDEX `{ ipNetToMediaIfIndex, ipNetToMediaNetAddress }` splits into the
//! integer `2` and the address `192.0.2.1`.

use std::fmt;

use crate::details::{Access, BaseType, Syntax};
use crate::oid::Oid;

/// Everything needed to show a table as rows and columns.
///
/// Get one with [`MibSet::table`](crate::MibSet::table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// Name of the table, such as `"ifTable"`.
    pub name: String,
    /// Module that defines the table.
    pub module: Option<String>,
    /// OID of the table.
    pub oid: Oid,
    /// Name of the row (entry), such as `"ifEntry"`.
    pub row: String,
    /// OID of the row; every column lies directly below it.
    pub row_oid: Oid,
    /// The row named in the row's `AUGMENTS` clause, such as `"ifEntry"` for
    /// `ifXEntry`. The index then comes from that row.
    pub augments: Option<String>,
    /// The INDEX components, in order.
    pub index: Vec<IndexColumn>,
    /// The columns, in arc order.
    pub columns: Vec<TableColumn>,
}

impl Table {
    /// Split the OID of a table cell into the position of its column in
    /// [`columns`](Self::columns) and the instance suffix. `None` when the
    /// OID is not a cell of a known column of this table.
    pub fn split<'o>(&self, oid: &'o Oid) -> Option<(usize, &'o [u32])> {
        let arcs = oid.arcs();
        let row = self.row_oid.arcs();
        if arcs.len() <= row.len() + 1 || !arcs.starts_with(row) {
            return None;
        }
        let arc = arcs[row.len()];
        let column = self.columns.iter().position(|c| c.arc == arc)?;
        Some((column, &arcs[row.len() + 1..]))
    }

    /// Decode an instance suffix with this table's INDEX; see
    /// [`decode_index`].
    pub fn decode_index(&self, suffix: &[u32]) -> Result<Vec<IndexValue>, IndexError> {
        decode_index(&self.index, suffix)
    }

    /// Encode index values into an instance suffix; see [`encode_index`].
    pub fn encode_index(&self, values: &[IndexValue]) -> Result<Vec<u32>, IndexError> {
        encode_index(&self.index, values)
    }
}

/// One column of a [`Table`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableColumn {
    /// Name, such as `"ifDescr"`.
    pub name: String,
    /// The last arc of the column's OID (`2` for `ifDescr`).
    pub arc: u32,
    /// OID of the column.
    pub oid: Oid,
    /// `MAX-ACCESS` of the column.
    pub access: Option<Access>,
    /// Type of the column.
    pub syntax: Option<Syntax>,
}

impl TableColumn {
    /// True if an agent can return values for this column. Index columns
    /// are often `not-accessible`: their values are only in the suffix.
    pub fn is_readable(&self) -> bool {
        matches!(
            self.access,
            None | Some(Access::ReadOnly | Access::ReadWrite | Access::ReadCreate)
        )
    }
}

/// One component of a table's INDEX, with what is needed to decode it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexColumn {
    /// Name of the index object, such as `"ifIndex"`. For an INDEX that
    /// names a type instead of an object (`INDEX { INTEGER }`, allowed in
    /// SMIv1), the type name.
    pub name: String,
    /// Module that defines the index object.
    pub module: Option<String>,
    /// True if marked `IMPLIED`: the length of the value is not encoded.
    pub implied: bool,
    /// Type of the index object, when it resolves.
    pub syntax: Option<Syntax>,
    /// How the value is written into the suffix.
    pub encoding: IndexEncoding,
}

/// How an INDEX component is written into an instance suffix (RFC 2578,
/// section 7.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexEncoding {
    /// An integer-valued type: one arc.
    Integer,
    /// `IpAddress`: four arcs, one per byte.
    IpAddress,
    /// An `OCTET STRING` of fixed size, such as `SIZE (6)`: that many arcs,
    /// one per byte, without a length.
    FixedOctets(usize),
    /// A variable-length `OCTET STRING`: the length, then one arc per byte.
    Octets,
    /// An `IMPLIED` `OCTET STRING`: one arc per byte up to the end of the
    /// suffix.
    ImpliedOctets,
    /// An `OBJECT IDENTIFIER`: the number of arcs, then the arcs.
    ObjectIdentifier,
    /// An `IMPLIED` `OBJECT IDENTIFIER`: the arcs up to the end of the suffix.
    ImpliedObjectIdentifier,
    /// The type did not resolve (usually a missing import): the rest of the
    /// suffix, as arcs.
    Unknown,
}

impl IndexEncoding {
    /// The encoding for a type, as the rules in RFC 2578, 7.7 say.
    pub fn for_syntax(syntax: Option<&Syntax>, implied: bool) -> IndexEncoding {
        let Some(syntax) = syntax else {
            return IndexEncoding::Unknown;
        };
        match syntax.base_type {
            BaseType::Integer32
            | BaseType::Unsigned32
            | BaseType::Gauge32
            | BaseType::Counter32
            | BaseType::TimeTicks => IndexEncoding::Integer,
            BaseType::IpAddress => IndexEncoding::IpAddress,
            BaseType::OctetString | BaseType::Opaque | BaseType::Bits => {
                if implied {
                    IndexEncoding::ImpliedOctets
                } else if let Some(size) = fixed_size(syntax) {
                    IndexEncoding::FixedOctets(size)
                } else {
                    IndexEncoding::Octets
                }
            }
            BaseType::ObjectIdentifier if implied => IndexEncoding::ImpliedObjectIdentifier,
            BaseType::ObjectIdentifier => IndexEncoding::ObjectIdentifier,
            _ => IndexEncoding::Unknown,
        }
    }
}

/// The size of an `OCTET STRING` that allows exactly one size above zero.
fn fixed_size(syntax: &Syntax) -> Option<usize> {
    use crate::details::RangeBound;
    match syntax.sizes.as_slice() {
        [range] if range.min == range.max => match range.min {
            RangeBound::Signed(size) if size > 0 => usize::try_from(size).ok(),
            _ => None,
        },
        _ => None,
    }
}

/// One decoded INDEX value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IndexValue {
    /// An integer-valued index.
    Integer(u32),
    /// An `IpAddress`.
    IpAddress([u8; 4]),
    /// An `OCTET STRING` (text or bytes; the type's display hint says which).
    Octets(Vec<u8>),
    /// An `OBJECT IDENTIFIER`.
    ObjectIdentifier(Oid),
    /// The arcs of an index whose type is unknown.
    Arcs(Vec<u32>),
}

impl fmt::Display for IndexValue {
    /// The value without MIB information: numbers, dotted addresses and
    /// OIDs, quoted text when the bytes are printable and hex otherwise.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexValue::Integer(n) => write!(f, "{n}"),
            IndexValue::IpAddress([a, b, c, d]) => write!(f, "{a}.{b}.{c}.{d}"),
            IndexValue::Octets(bytes) => {
                let printable = bytes.iter().all(|&b| (0x20..0x7f).contains(&b));
                if printable {
                    write!(f, "\"{}\"", String::from_utf8_lossy(bytes))
                } else {
                    for (i, byte) in bytes.iter().enumerate() {
                        let space = if i == 0 { "" } else { " " };
                        write!(f, "{space}{byte:02X}")?;
                    }
                    Ok(())
                }
            }
            IndexValue::ObjectIdentifier(oid) => write!(f, "{oid}"),
            IndexValue::Arcs(arcs) => write!(f, "{}", Oid::new(arcs.clone())),
        }
    }
}

/// An instance suffix that does not fit the INDEX.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message} (index {column}, at arc {position} of the instance)")]
pub struct IndexError {
    /// The INDEX component that could not be decoded.
    pub column: String,
    /// Position in the suffix (from 1) where the problem is.
    pub position: usize,
    /// What is wrong.
    pub message: String,
}

/// Split an instance suffix into one value per INDEX component.
///
/// ```
/// use mib_core::{IndexColumn, IndexEncoding, IndexValue, decode_index};
///
/// let column = |name: &str, encoding| IndexColumn {
///     name: name.to_string(),
///     module: None,
///     implied: false,
///     syntax: None,
///     encoding,
/// };
/// // ipNetToMediaTable: INDEX { ipNetToMediaIfIndex, ipNetToMediaNetAddress }
/// let index = [
///     column("ipNetToMediaIfIndex", IndexEncoding::Integer),
///     column("ipNetToMediaNetAddress", IndexEncoding::IpAddress),
/// ];
/// let values = decode_index(&index, &[2, 192, 0, 2, 1]).unwrap();
/// assert_eq!(values, [IndexValue::Integer(2), IndexValue::IpAddress([192, 0, 2, 1])]);
/// ```
pub fn decode_index(index: &[IndexColumn], suffix: &[u32]) -> Result<Vec<IndexValue>, IndexError> {
    let mut values = Vec::with_capacity(index.len());
    let mut rest = suffix;
    for column in index {
        let position = suffix.len() - rest.len() + 1;
        let error = |message: String| IndexError {
            column: column.name.clone(),
            position,
            message,
        };
        let take = |rest: &mut &[u32], n: usize| -> Result<Vec<u32>, IndexError> {
            if rest.len() < n {
                return Err(error(format!(
                    "{n} arc{} needed, {} left",
                    if n == 1 { "" } else { "s" },
                    rest.len()
                )));
            }
            let (taken, left) = rest.split_at(n);
            *rest = left;
            Ok(taken.to_vec())
        };
        let length = |rest: &mut &[u32]| -> Result<usize, IndexError> {
            let n = take(rest, 1)?[0] as usize;
            if n > rest.len() {
                return Err(error(format!(
                    "the length says {n}, but only {} arc{} follow",
                    rest.len(),
                    if rest.len() == 1 { "" } else { "s" }
                )));
            }
            Ok(n)
        };
        let octets = |arcs: Vec<u32>| -> Result<IndexValue, IndexError> {
            arcs.iter()
                .map(|&arc| u8::try_from(arc))
                .collect::<Result<Vec<u8>, _>>()
                .map(IndexValue::Octets)
                .map_err(|_| error("a byte of a string must be 0..255".to_string()))
        };
        let value = match column.encoding {
            IndexEncoding::Integer => IndexValue::Integer(take(&mut rest, 1)?[0]),
            IndexEncoding::IpAddress => match octets(take(&mut rest, 4)?)? {
                IndexValue::Octets(bytes) => {
                    IndexValue::IpAddress([bytes[0], bytes[1], bytes[2], bytes[3]])
                }
                _ => unreachable!("octets() returns Octets"),
            },
            IndexEncoding::FixedOctets(size) => octets(take(&mut rest, size)?)?,
            IndexEncoding::Octets => {
                let n = length(&mut rest)?;
                octets(take(&mut rest, n)?)?
            }
            IndexEncoding::ImpliedOctets => {
                let n = rest.len();
                octets(take(&mut rest, n)?)?
            }
            IndexEncoding::ObjectIdentifier => {
                let n = length(&mut rest)?;
                IndexValue::ObjectIdentifier(Oid::new(take(&mut rest, n)?))
            }
            IndexEncoding::ImpliedObjectIdentifier => {
                let n = rest.len();
                IndexValue::ObjectIdentifier(Oid::new(take(&mut rest, n)?))
            }
            IndexEncoding::Unknown => {
                let n = rest.len();
                IndexValue::Arcs(take(&mut rest, n)?)
            }
        };
        values.push(value);
    }
    if !rest.is_empty() {
        let last = index.last().map(|c| c.name.clone()).unwrap_or_default();
        return Err(IndexError {
            column: last,
            position: suffix.len() - rest.len() + 1,
            message: format!(
                "{} arc{} left over after the last index",
                rest.len(),
                if rest.len() == 1 { "" } else { "s" }
            ),
        });
    }
    Ok(values)
}

/// Write index values into an instance suffix: the reverse of
/// [`decode_index`].
pub fn encode_index(index: &[IndexColumn], values: &[IndexValue]) -> Result<Vec<u32>, IndexError> {
    if index.len() != values.len() {
        return Err(IndexError {
            column: index.last().map(|c| c.name.clone()).unwrap_or_default(),
            position: 1,
            message: format!(
                "{} index values needed, {} given",
                index.len(),
                values.len()
            ),
        });
    }
    let mut suffix = Vec::new();
    for (column, value) in index.iter().zip(values) {
        let mismatch = || IndexError {
            column: column.name.clone(),
            position: suffix.len() + 1,
            message: format!("{value} does not fit index type {:?}", column.encoding),
        };
        let bytes = |b: &[u8]| b.iter().map(|&b| u32::from(b)).collect::<Vec<_>>();
        match (column.encoding, value) {
            (IndexEncoding::Integer, IndexValue::Integer(n)) => suffix.push(*n),
            (IndexEncoding::IpAddress, IndexValue::IpAddress(address)) => {
                suffix.extend(bytes(address));
            }
            (IndexEncoding::FixedOctets(size), IndexValue::Octets(b)) if b.len() == size => {
                suffix.extend(bytes(b));
            }
            (IndexEncoding::Octets, IndexValue::Octets(b)) => {
                suffix.push(b.len() as u32);
                suffix.extend(bytes(b));
            }
            (IndexEncoding::ImpliedOctets, IndexValue::Octets(b)) => suffix.extend(bytes(b)),
            (IndexEncoding::ObjectIdentifier, IndexValue::ObjectIdentifier(oid)) => {
                suffix.push(oid.len() as u32);
                suffix.extend_from_slice(oid.arcs());
            }
            (IndexEncoding::ImpliedObjectIdentifier, IndexValue::ObjectIdentifier(oid)) => {
                suffix.extend_from_slice(oid.arcs());
            }
            (IndexEncoding::Unknown, IndexValue::Arcs(arcs)) => suffix.extend_from_slice(arcs),
            _ => return Err(mismatch()),
        }
    }
    Ok(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, encoding: IndexEncoding) -> IndexColumn {
        IndexColumn {
            name: name.to_string(),
            module: None,
            implied: matches!(
                encoding,
                IndexEncoding::ImpliedOctets | IndexEncoding::ImpliedObjectIdentifier
            ),
            syntax: None,
            encoding,
        }
    }

    fn round_trip(index: &[IndexColumn], suffix: &[u32], expected: &[IndexValue]) {
        let values = decode_index(index, suffix).unwrap();
        assert_eq!(values, expected);
        assert_eq!(encode_index(index, &values).unwrap(), suffix);
    }

    #[test]
    fn every_encoding_round_trips() {
        use IndexEncoding as E;
        round_trip(&[column("a", E::Integer)], &[7], &[IndexValue::Integer(7)]);
        round_trip(
            &[
                column("a", E::Octets),
                column("b", E::FixedOctets(2)),
                column("c", E::IpAddress),
                column("d", E::ObjectIdentifier),
                column("e", E::ImpliedOctets),
            ],
            &[2, 104, 105, 0, 255, 10, 0, 0, 1, 3, 1, 3, 6, 120, 121],
            &[
                IndexValue::Octets(b"hi".to_vec()),
                IndexValue::Octets(vec![0, 255]),
                IndexValue::IpAddress([10, 0, 0, 1]),
                IndexValue::ObjectIdentifier("1.3.6".parse().unwrap()),
                IndexValue::Octets(b"xy".to_vec()),
            ],
        );
        round_trip(
            &[column("a", E::ImpliedObjectIdentifier)],
            &[1, 3, 6, 1],
            &[IndexValue::ObjectIdentifier("1.3.6.1".parse().unwrap())],
        );
        // An empty string, and an IMPLIED one at the end.
        round_trip(
            &[column("a", E::Octets), column("b", E::ImpliedOctets)],
            &[0],
            &[IndexValue::Octets(vec![]), IndexValue::Octets(vec![])],
        );
    }

    #[test]
    fn bad_suffixes_say_where() {
        use IndexEncoding as E;
        let index = [column("name", E::Octets), column("port", E::Integer)];
        let error = decode_index(&index, &[5, 65, 66]).unwrap_err();
        assert_eq!(error.column, "name");
        assert_eq!(error.position, 1);
        assert_eq!(
            error.to_string(),
            "the length says 5, but only 2 arcs follow (index name, at arc 1 of the instance)"
        );

        let error = decode_index(&index, &[1, 65]).unwrap_err();
        assert_eq!((error.column.as_str(), error.position), ("port", 3));

        let error = decode_index(&index, &[1, 65, 7, 8]).unwrap_err();
        assert_eq!(error.message, "1 arc left over after the last index");

        let error = decode_index(&[column("addr", E::IpAddress)], &[10, 0, 0, 256]).unwrap_err();
        assert_eq!(error.message, "a byte of a string must be 0..255");
    }

    #[test]
    fn index_values_display_plainly() {
        assert_eq!(
            IndexValue::Octets(b"public".to_vec()).to_string(),
            "\"public\""
        );
        assert_eq!(IndexValue::Octets(vec![0, 0x1b]).to_string(), "00 1B");
        assert_eq!(
            IndexValue::IpAddress([192, 0, 2, 1]).to_string(),
            "192.0.2.1"
        );
    }
}

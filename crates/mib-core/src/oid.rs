//! Numeric object identifiers.

use std::fmt;
use std::str::FromStr;

/// A numeric object identifier such as `1.3.6.1.2.1.1.1`.
///
/// Parse one from text with [`str::parse`]; a leading dot (`.1.3.6`) is
/// accepted, as net-snmp prints OIDs that way. [`Display`](fmt::Display)
/// prints the dotted form without a leading dot.
///
/// ```
/// use mib_core::Oid;
///
/// let oid: Oid = ".1.3.6.1.2.1".parse().unwrap();
/// assert_eq!(oid.arcs(), &[1, 3, 6, 1, 2, 1]);
/// assert_eq!(oid.to_string(), "1.3.6.1.2.1");
/// ```
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Oid(Vec<u32>);

impl Oid {
    /// Build an OID from its arcs.
    pub fn new(arcs: impl Into<Vec<u32>>) -> Self {
        Oid(arcs.into())
    }

    /// The arcs (the numbers between the dots).
    pub fn arcs(&self) -> &[u32] {
        &self.0
    }

    /// Number of arcs.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True for the empty OID (the root of the tree).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// True if `self` equals `prefix` or lies below it in the tree.
    pub fn starts_with(&self, prefix: &Oid) -> bool {
        self.0.starts_with(&prefix.0)
    }

    /// The OID one level up, or `None` for the empty OID.
    pub fn parent(&self) -> Option<Oid> {
        let (_, parent) = self.0.split_last()?;
        Some(Oid(parent.to_vec()))
    }

    /// The OID one level down, with `arc` appended.
    pub fn child(&self, arc: u32) -> Oid {
        let mut arcs = self.0.clone();
        arcs.push(arc);
        Oid(arcs)
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, arc) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            write!(f, "{arc}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Oid({self})")
    }
}

impl From<Vec<u32>> for Oid {
    fn from(arcs: Vec<u32>) -> Self {
        Oid(arcs)
    }
}

impl From<&[u32]> for Oid {
    fn from(arcs: &[u32]) -> Self {
        Oid(arcs.to_vec())
    }
}

/// Error returned when text is not a valid numeric OID.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a numeric OID: {text:?}")]
pub struct ParseOidError {
    text: String,
}

impl FromStr for Oid {
    type Err = ParseOidError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || ParseOidError {
            text: text.to_string(),
        };
        let digits = text.strip_prefix('.').unwrap_or(text);
        if digits.is_empty() {
            return Err(error());
        }
        digits
            .split('.')
            .map(|arc| {
                // u32::from_str also accepts a leading '+', which is not valid in an OID.
                if arc.is_empty() || !arc.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(error());
                }
                arc.parse::<u32>().map_err(|_| error())
            })
            .collect::<Result<Vec<u32>, _>>()
            .map(Oid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_and_without_leading_dot() {
        let a: Oid = "1.3.6.1".parse().unwrap();
        let b: Oid = ".1.3.6.1".parse().unwrap();
        assert_eq!(a, b);
        assert_eq!(a.arcs(), &[1, 3, 6, 1]);
    }

    #[test]
    fn rejects_invalid_text() {
        for text in ["", ".", "1..3", "1.3.", "1.a", "1.+3", "1.99999999999"] {
            assert!(text.parse::<Oid>().is_err(), "{text:?} should not parse");
        }
    }

    #[test]
    fn display_round_trips() {
        let oid: Oid = "1.3.6.1.4.1.9".parse().unwrap();
        assert_eq!(oid.to_string().parse::<Oid>().unwrap(), oid);
    }

    #[test]
    fn tree_relations() {
        let oid = Oid::new([1, 3, 6]);
        assert_eq!(oid.parent(), Some(Oid::new([1, 3])));
        assert_eq!(oid.child(1), Oid::new([1, 3, 6, 1]));
        assert!(oid.child(1).starts_with(&oid));
        assert!(!oid.starts_with(&oid.child(1)));
        assert_eq!(Oid::default().parent(), None);
    }
}

//! Turning what a user typed (`sysDescr.0`, `IF-MIB::ifDescr.3`,
//! `1.3.6.1.2.1.1`) into the OID to ask an agent for.

use mib_core::{MibSet, Oid};

/// Why [`resolve_object`] found no OID.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ObjectError {
    /// Nothing was typed.
    #[error("no object given: type a name such as sysDescr.0, or an OID")]
    Empty,
    /// No loaded MIB defines the name.
    #[error("unknown object {text:?}: no loaded MIB defines {name}")]
    Unknown {
        /// The whole text, trimmed.
        text: String,
        /// The name looked up, such as `ifDescr` or `IF-MIB::ifDescr`.
        name: String,
    },
    /// The part after the name is not numbers separated by dots.
    #[error(
        "{text:?}: the instance after the name must be numbers separated by dots, \
         such as ifDescr.3"
    )]
    BadInstance {
        /// The whole text, trimmed.
        text: String,
    },
}

/// The OID for an object as a user writes it: a numeric OID
/// (`1.3.6.1.2.1.1.1.0`, also with a leading dot), a name (`sysDescr`), or
/// `MODULE::name`, each optionally followed by an instance suffix
/// (`ifDescr.3`, `IF-MIB::ifDescr.3`).
///
/// ```
/// # let mibs = mib_core::MibLibrary::new().with_system_folders(false).load().unwrap();
/// let oid = snmp_core::resolve_object(&mibs, "enterprises.9.1").unwrap();
/// assert_eq!(oid.to_string(), "1.3.6.1.4.1.9.1");
/// ```
pub fn resolve_object(mibs: &MibSet, text: &str) -> Result<Oid, ObjectError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ObjectError::Empty);
    }
    if let Ok(oid) = text.parse::<Oid>() {
        return Ok(oid);
    }
    let (module, rest) = match text.split_once("::") {
        Some((module, rest)) => (Some(module), rest),
        None => (None, text),
    };
    let (name, suffix) = match rest.split_once('.') {
        Some((name, suffix)) => (name, Some(suffix)),
        None => (rest, None),
    };
    let lookup = match module {
        Some(module) => format!("{module}::{name}"),
        None => name.to_string(),
    };
    let node = mibs.find(&lookup).ok_or_else(|| ObjectError::Unknown {
        text: text.to_string(),
        name: lookup.clone(),
    })?;
    let mut arcs = node.oid().arcs().to_vec();
    if let Some(suffix) = suffix {
        let suffix: Oid = suffix.parse().map_err(|_| ObjectError::BadInstance {
            text: text.to_string(),
        })?;
        arcs.extend_from_slice(suffix.arcs());
    }
    Ok(Oid::new(arcs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mib_core::MibLibrary;

    #[test]
    fn objects_resolve_to_oids() {
        let mibs = MibLibrary::new().with_system_folders(false).load().unwrap();
        let oid = |text: &str| resolve_object(&mibs, text).map(|o| o.to_string());
        assert_eq!(oid("1.3.6.1.2.1.1.1.0").unwrap(), "1.3.6.1.2.1.1.1.0");
        assert_eq!(oid(".1.3.6").unwrap(), "1.3.6");
        assert_eq!(oid("enterprises").unwrap(), "1.3.6.1.4.1");
        assert_eq!(oid(" enterprises.9.1 ").unwrap(), "1.3.6.1.4.1.9.1");
        assert_eq!(oid("SNMPv2-SMI::mib-2.1").unwrap(), "1.3.6.1.2.1.1");
        assert_eq!(oid(""), Err(ObjectError::Empty));
        assert!(matches!(
            resolve_object(&mibs, "noSuchThing.0"),
            Err(ObjectError::Unknown { name, .. }) if name == "noSuchThing"
        ));
        assert!(matches!(
            resolve_object(&mibs, "enterprises.x"),
            Err(ObjectError::BadInstance { .. })
        ));
        assert!(oid("NO-SUCH-MIB::enterprises").is_err());
    }
}

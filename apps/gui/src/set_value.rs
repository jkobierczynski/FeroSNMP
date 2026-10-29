//! What the Set value dialog shows and checks, as plain Rust: the object
//! to set and what its MIB allows, the value as typed, and whether it can
//! be sent. The dialog (in `ui::set_dialog`) only moves text between this
//! and its controls.

use mib_core::{MibSet, NodeKind, Oid};
use snmp_core::{Formatter, InputError, Value, ValueInput, ValueType, Varbind};

/// The object the dialog sets, as far as the MIBs know it.
#[derive(Debug, Clone)]
pub struct SetForm {
    /// What the MIBs say about the instance; `None` when the Object text
    /// is not an object at all.
    input: Result<ValueInput, String>,
    mibs: Option<MibSet>,
}

/// Whether the value can be sent, and what the status line says.
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    /// Ready to send. The message says what goes out, and why the MIB would
    /// refuse it when the check was turned off.
    Ready { varbind: Varbind, message: String },
    /// Not sent. With `can_force`, "Send anyway" would send it.
    Refused { message: String, can_force: bool },
}

impl SetForm {
    /// Look up `object`, as typed in the dialog: a name or numeric OID. A
    /// scalar given without its instance means instance 0, as for Get.
    pub fn new(mibs: Option<&MibSet>, object: &str) -> SetForm {
        let object = object.trim();
        let oid = if object.is_empty() {
            Err("Type the instance to set, such as sysContact.0.".to_string())
        } else {
            match mibs {
                Some(set) => snmp_core::resolve_object(set, object)
                    .map(|oid| match set.node_at(&oid) {
                        Some(node) if node.kind() == NodeKind::Scalar => oid.child(0),
                        _ => oid,
                    })
                    .map_err(|error| format!("{error}.")),
                None => object
                    .trim_start_matches('.')
                    .parse::<Oid>()
                    .map_err(|_| "The MIBs are still loading: give a numeric OID.".to_string()),
            }
        };
        SetForm {
            input: oid.map(|oid| ValueInput::new(mibs, oid)),
            mibs: mibs.cloned(),
        }
    }

    /// The instance by name, such as `SNMPv2-MIB::sysContact.0`.
    pub fn name(&self) -> Option<&str> {
        self.input.as_ref().ok().map(|input| input.name.as_str())
    }

    /// The instance.
    pub fn oid(&self) -> Option<&Oid> {
        self.input.as_ref().ok().map(|input| &input.oid)
    }

    /// One line on the object: its type, what the MIB allows and its
    /// access, such as `INTEGER (off(1), on(2)), read-write`.
    pub fn about(&self) -> String {
        let input = match &self.input {
            Ok(input) => input,
            Err(message) => return message.clone(),
        };
        if input.syntax.is_none() {
            return "No loaded MIB defines this object: choose the type in \u{201c}Send as\u{201d}."
                .to_string();
        }
        let mut text = input.type_text();
        if let Some(access) = input.access {
            text.push_str(&format!(", {access}"));
        }
        text
    }

    /// Values to offer in the Value list: the labels of an enumeration.
    pub fn choices(&self) -> Vec<String> {
        let Ok(input) = &self.input else {
            return Vec::new();
        };
        input
            .syntax
            .as_ref()
            .map(|s| s.enums.iter().map(|e| e.label.clone()).collect())
            .unwrap_or_default()
    }

    /// The entries of the "Send as" list: the MIB's type first, then every
    /// type. [`value_type`](Self::value_type) turns a position back.
    pub fn type_names(&self) -> Vec<String> {
        let first = match self.input.as_ref().ok().and_then(ValueInput::value_type) {
            Some(value_type) => format!("The MIB type ({value_type})"),
            None => "Choose a type\u{2026}".to_string(),
        };
        std::iter::once(first)
            .chain(ValueType::ALL.iter().map(|t| t.name().to_string()))
            .collect()
    }

    /// The type for a position in [`type_names`](Self::type_names): `None`
    /// for the MIB's type.
    pub fn value_type(position: usize) -> Option<ValueType> {
        position
            .checked_sub(1)
            .and_then(|i| ValueType::ALL.get(i).copied())
    }

    /// The current value as text to edit, with the position in
    /// [`type_names`](Self::type_names) to read it back with.
    pub fn edit_text(&self, value: &Value) -> (usize, String) {
        let Ok(input) = &self.input else {
            return (0, String::new());
        };
        let (value_type, text) = input.edit_text(value);
        let mib_type = input.value_type();
        // Text for a text object reads back with the MIB type; anything else
        // (hex for a binary string, a type the MIB does not know) is named.
        let position = if mib_type == Some(value_type) {
            0
        } else {
            ValueType::ALL
                .iter()
                .position(|&t| t == value_type)
                .map_or(0, |i| i + 1)
        };
        (position, text)
    }

    /// Check `text` as the value: read it with `value_type` (the MIB's type
    /// when `None`), and unless `force`, check it against the MIB.
    pub fn check(&self, text: &str, value_type: Option<ValueType>, force: bool) -> Check {
        let input = match &self.input {
            Ok(input) => input,
            Err(message) => {
                return Check::Refused {
                    message: message.clone(),
                    can_force: false,
                };
            }
        };
        // Nothing typed yet: say what to type rather than why "" is wrong.
        // An empty string is a value, though, for a string.
        let typed = value_type.or_else(|| input.value_type());
        if text.trim().is_empty()
            && !matches!(
                typed,
                Some(ValueType::Text | ValueType::Hex | ValueType::Bits)
            )
        {
            let message = match (input.allowed(), &input.syntax) {
                (Some(allowed), Some(_)) => format!("Type the new value ({allowed})."),
                _ => "Type the new value.".to_string(),
            };
            return Check::Refused {
                message,
                can_force: false,
            };
        }
        let refused = |error: InputError| Check::Refused {
            can_force: error.can_force(),
            message: sentence(&error.to_string()),
        };
        let (varbind, warning) = match input.build(text, value_type, true) {
            Ok(varbind) => (varbind, None),
            // A table or a group holds no value; sending one anyway only
            // ends in a confusing question about its type.
            Err(error @ InputError::NotAnObject { .. }) => return refused(error),
            Err(error) if force && error.can_force() => {
                match input.build(text, value_type, false) {
                    Ok(varbind) => (varbind, Some(error)),
                    Err(error) => return refused(error),
                }
            }
            Err(error) => return refused(error),
        };
        let shown = Formatter::new(self.mibs.as_ref()).value_text(&varbind);
        let mut message = format!("Sends {} {}", shown.type_name, shown.text);
        if let Some(warning) = warning {
            message.push_str(&format!(
                ", although {}",
                sentence(&lowercase_first(&warning.to_string()))
            ));
        } else {
            message.push('.');
        }
        Check::Ready { varbind, message }
    }
}

fn sentence(text: &str) -> String {
    if text.ends_with('.') {
        text.to_string()
    } else {
        format!("{text}.")
    }
}

fn lowercase_first(text: &str) -> String {
    // Names such as `FEROSNMP-TEST-MIB::fstMode.0` keep their case.
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if first.is_uppercase() && second.is_lowercase() => first
            .to_lowercase()
            .chain(text[first.len_utf8()..].chars())
            .collect(),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_mib;

    fn form(object: &str) -> SetForm {
        SetForm::new(Some(&test_mib()), object)
    }

    fn ready(check: Check) -> (Value, String) {
        match check {
            Check::Ready { varbind, message } => (varbind.value, message),
            other => panic!("{other:?}"),
        }
    }

    fn refused(check: Check) -> (String, bool) {
        match check {
            Check::Refused { message, can_force } => (message, can_force),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_scalar_means_its_instance() {
        let form = form("fstMode");
        assert_eq!(form.name(), Some("FEROSNMP-TEST-MIB::fstMode.0"));
        assert_eq!(form.about(), "INTEGER (off(1), on(2), auto(3)), read-write");
        assert_eq!(form.choices(), ["off", "on", "auto"]);
        assert_eq!(form.type_names()[0], "The MIB type (INTEGER)");
        assert_eq!(form.type_names().len(), 11);
    }

    #[test]
    fn values_are_checked_as_typed() {
        let form = form("fstMode.0");
        assert_eq!(
            ready(form.check("auto", None, false)),
            (Value::Integer(3), "Sends INTEGER auto(3).".to_string())
        );
        let (message, can_force) = refused(form.check("7", None, false));
        assert_eq!(
            message,
            "FEROSNMP-TEST-MIB::fstMode.0: 7 is not one of off(1), on(2), auto(3)."
        );
        assert!(can_force);
        // Sent anyway, with the reason the MIB would refuse it.
        let (value, message) = ready(form.check("7", None, true));
        assert_eq!(value, Value::Integer(7));
        assert_eq!(
            message,
            "Sends INTEGER 7, although FEROSNMP-TEST-MIB::fstMode.0: 7 is not one of off(1), \
             on(2), auto(3)."
        );
        // Text that is no value at all cannot be forced.
        let (_, can_force) = refused(form.check("maybe", None, true));
        assert!(!can_force);
    }

    #[test]
    fn read_only_objects_are_refused_until_forced() {
        let form = form("fstStatus.0");
        assert!(form.about().ends_with("read-only"), "{}", form.about());
        let (message, can_force) = refused(form.check("up", None, false));
        assert!(message.contains("read-only in the MIB"), "{message}");
        assert!(can_force);
        assert_eq!(ready(form.check("up", None, true)).0, Value::Integer(1));
    }

    #[test]
    fn a_type_can_be_chosen() {
        let unknown = form("1.3.6.1.4.1.99999.1.0");
        assert!(unknown.about().starts_with("No loaded MIB"));
        assert_eq!(unknown.type_names()[0], "Choose a type\u{2026}");
        assert!(matches!(
            unknown.check("5", None, false),
            Check::Refused { .. }
        ));
        let unsigned = ValueType::ALL
            .iter()
            .position(|&t| t == ValueType::Unsigned32)
            .unwrap()
            + 1;
        assert_eq!(SetForm::value_type(unsigned), Some(ValueType::Unsigned32));
        assert_eq!(SetForm::value_type(0), None);
        assert_eq!(
            ready(unknown.check("5", SetForm::value_type(unsigned), false)).0,
            Value::Gauge32(5)
        );
    }

    #[test]
    fn current_values_are_edited_as_text() {
        let contact = form("fstContact.0");
        assert_eq!(
            contact.edit_text(&Value::OctetString(b"Jurgen".to_vec())),
            (0, "Jurgen".to_string())
        );
        // Binary content comes back as hex, read back as hex.
        let (position, text) = contact.edit_text(&Value::OctetString(vec![0, 0xff]));
        assert_eq!(text, "00 ff");
        assert_eq!(SetForm::value_type(position), Some(ValueType::Hex));
        assert_eq!(
            ready(contact.check(&text, SetForm::value_type(position), false)).0,
            Value::OctetString(vec![0, 0xff])
        );
        assert_eq!(
            refused(form("fstMode.0").check(" ", None, false)).0,
            "Type the new value (off(1), on(2), auto(3))."
        );
        assert_eq!(
            ready(contact.check("", None, false)).0,
            Value::OctetString(vec![])
        );
        assert_eq!(
            form("fstMode.0").edit_text(&Value::Integer(2)),
            (0, "on".to_string())
        );
    }

    #[test]
    fn not_an_object() {
        let table = form("fstPortTable");
        assert!(table.name().is_some());
        let (message, _) = refused(table.check("1", None, true));
        assert_eq!(
            message,
            "FEROSNMP-TEST-MIB::fstPortTable is a table, not an object that holds a value."
        );
        let unknown = form("noSuchObject.0");
        assert!(unknown.name().is_none());
        assert!(
            unknown.about().contains("noSuchObject"),
            "{}",
            unknown.about()
        );
        assert!(
            SetForm::new(None, "sysContact.0")
                .about()
                .contains("still loading")
        );
        assert_eq!(
            SetForm::new(None, ".1.3.6.1.2.1.1.4.0")
                .oid()
                .map(ToString::to_string),
            Some("1.3.6.1.2.1.1.4.0".to_string())
        );
    }
}

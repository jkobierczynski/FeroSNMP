//! What the details pane shows for a node: a list of properties and the
//! description text.

use mib_core::{MibSet, Node};

/// The properties of a node as (label, value) rows, leaving out the ones that
/// do not apply.
pub fn properties(set: &MibSet, node: Node<'_>) -> Vec<(&'static str, String)> {
    let details = node.details();
    let mut rows: Vec<(&'static str, String)> = Vec::new();
    let mut add = |label: &'static str, value: String| {
        if !value.is_empty() {
            rows.push((label, value));
        }
    };

    add("Name", details.name.clone());
    add("OID", details.oid.to_string());
    add("Kind", details.kind.to_string());
    if let Some(syntax) = &details.syntax {
        add("Type", syntax.type_chain.join(" \u{2192} "));
        add(
            "Display hint",
            syntax.display_hint.clone().unwrap_or_default(),
        );
        add("Values", join(&syntax.enums, ", "));
        add("Bits", join(&syntax.bits, ", "));
        add("Range", join(&syntax.ranges, " | "));
        if !syntax.sizes.is_empty() {
            add("Size", format!("SIZE ({})", join(&syntax.sizes, " | ")));
        }
    }
    add("Units", details.units.clone().unwrap_or_default());
    add(
        "Access",
        details.access.map(|a| a.to_string()).unwrap_or_default(),
    );
    add(
        "Status",
        details.status.map(|s| s.to_string()).unwrap_or_default(),
    );
    add("Index", details.index_text());
    add("Augments", details.augments.clone().unwrap_or_default());
    if let Some(module) = &details.module {
        add("Module", module.clone());
        if let Some(info) = set.module(module) {
            let file = match (&info.file, info.embedded) {
                (_, true) => "built in".to_string(),
                (Some(file), false) => file.display().to_string(),
                (None, false) => String::new(),
            };
            add("File", file);
            add("SMI version", info.smi_version.to_string());
        }
    }
    // The same name in other modules is a common source of confusion with
    // vendor MIBs, so say so.
    if !details.name.is_empty() {
        let others: Vec<String> = set
            .find_all(&details.name)
            .into_iter()
            .filter(|other| *other != node)
            .map(|other| format!("{} ({})", other.qualified_name(), other.oid()))
            .collect();
        add("Also defined", others.join(", "));
    }
    rows
}

/// The DESCRIPTION, followed by the REFERENCE if there is one, with the MIB's
/// indentation removed. Lines end in `\n`.
pub fn description(node: Node<'_>) -> String {
    let details = node.details();
    let mut text = String::new();
    if let Some(description) = &details.description {
        push_lines(&mut text, description);
    }
    if let Some(reference) = &details.reference {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("Reference:\n");
        push_lines(&mut text, reference);
    }
    text
}

fn push_lines(out: &mut String, text: &str) {
    for line in mib_core::text_lines(text) {
        out.push_str(line);
        out.push('\n');
    }
}

fn join<T: ToString>(items: &[T], separator: &str) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(separator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ietf_set;

    fn value<'a>(rows: &'a [(&str, String)], label: &str) -> Option<&'a str> {
        rows.iter()
            .find(|row| row.0 == label)
            .map(|row| row.1.as_str())
    }

    #[test]
    fn column_properties() {
        let set = ietf_set();
        let node = set.find("ENTITY-MIB::entPhysicalClass").unwrap();
        let rows = properties(&set, node);
        assert_eq!(value(&rows, "OID"), Some("1.3.6.1.2.1.47.1.1.1.1.5"));
        assert_eq!(value(&rows, "Kind"), Some("column"));
        assert_eq!(value(&rows, "Access"), Some("read-only"));
        assert_eq!(value(&rows, "Index"), Some("entPhysicalIndex"));
        assert_eq!(value(&rows, "Module"), Some("ENTITY-MIB"));
        assert!(
            value(&rows, "Values")
                .unwrap()
                .starts_with("other(1), unknown(2)")
        );
        assert!(value(&rows, "File").unwrap().contains("ENTITY-MIB"));
        assert_eq!(value(&rows, "SMI version"), Some("SMIv2"));
        assert_eq!(value(&rows, "Units"), None, "empty rows are left out");
    }

    #[test]
    fn foundation_modules_are_built_in() {
        let set = ietf_set();
        let rows = properties(&set, set.find("SNMPv2-SMI::enterprises").unwrap());
        assert_eq!(value(&rows, "File"), Some("built in"));
    }

    #[test]
    fn description_is_unindented() {
        let set = ietf_set();
        let text = description(set.find("entPhysicalClass").unwrap());
        assert!(
            text.starts_with(
                "An indication of the general hardware type of the physical\nentity.\n\nAn agent"
            ),
            "{text}"
        );
    }
}

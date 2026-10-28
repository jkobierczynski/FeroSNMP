//! The `mib show`, `mib tree` and `mib search` commands.

use std::io::Write;

use mib_core::{MibSet, Node, NodeDetails, NodeKind, Oid};

use crate::wildcard::Pattern;
use crate::{CliError, Status};

/// Width of the labels in `mib show`, so the values line up.
const LABEL_WIDTH: usize = 14;

/// `ferosnmp mib show`: print the details of each queried node.
pub fn show(set: &MibSet, queries: &[String], out: &mut impl Write) -> Result<Status, CliError> {
    let mut status = Status::Success;
    for (i, query) in queries.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        match set.resolve(query) {
            Some(node) => write_details(set, query, node, out)?,
            None => {
                // Flush first, so the message appears after the nodes
                // printed before it, not above them.
                out.flush()?;
                not_found(set, query);
                status = Status::Problems;
            }
        }
    }
    Ok(status)
}

fn write_details(
    set: &MibSet,
    query: &str,
    node: Node<'_>,
    out: &mut impl Write,
) -> Result<(), CliError> {
    let details = node.details();
    writeln!(out, "{}", node.qualified_name())?;

    // A label and a value on one line. Values that are empty are left out.
    let mut field = |label: &str, value: &str| -> std::io::Result<()> {
        if value.is_empty() {
            Ok(())
        } else {
            writeln!(out, "  {label:LABEL_WIDTH$}{value}")
        }
    };

    field("OID", &details.oid.to_string())?;
    if let Some(suffix) = instance_suffix(set, query, &details.oid) {
        field("Instance", &suffix)?;
    }
    field("Kind", details.kind.as_str())?;
    if let Some(syntax) = &details.syntax {
        field("Type", &syntax.type_chain.join(" -> "))?;
        field("Display hint", syntax.display_hint.as_deref().unwrap_or(""))?;
        field("Values", &join(&syntax.enums, ", "))?;
        field("Bits", &join(&syntax.bits, ", "))?;
        field("Range", &join(&syntax.ranges, " | "))?;
        if !syntax.sizes.is_empty() {
            field("Size", &format!("SIZE ({})", join(&syntax.sizes, " | ")))?;
        }
    }
    field("Units", details.units.as_deref().unwrap_or(""))?;
    field("Access", details.access.map_or("", |a| a.as_str()))?;
    field("Status", details.status.map_or("", |s| s.as_str()))?;
    field("Index", &index_text(&details))?;
    field("Augments", details.augments.as_deref().unwrap_or(""))?;
    field("Also defined", &other_definitions(set, query, node))?;

    if let Some(text) = &details.description {
        writeln!(out, "  Description")?;
        write_block(text, out)?;
    }
    if let Some(text) = &details.reference {
        writeln!(out, "  Reference")?;
        write_block(text, out)?;
    }
    Ok(())
}

/// For a query that names an instance (`ifDescr.7` or `1.3.6.1.2.1.2.2.1.2.7`),
/// the part after the node's OID, as `.7`.
fn instance_suffix(set: &MibSet, query: &str, node_oid: &Oid) -> Option<String> {
    // Names with a suffix (`ifDescr.7`) are turned into a numeric OID by
    // replacing the name; a plain numeric OID is used as it is.
    let query = query.trim().trim_start_matches('.');
    let numeric = match query.split_once('.') {
        Some((name, rest)) if !name.chars().all(|c| c.is_ascii_digit()) => {
            format!("{}.{rest}", set.find(name)?.oid())
        }
        _ => query.to_string(),
    };
    let oid: Oid = numeric.parse().ok()?;
    let suffix = oid.arcs().get(node_oid.len()..)?;
    if suffix.is_empty() {
        return None;
    }
    Some(suffix.iter().map(|arc| format!(".{arc}")).collect())
}

/// `testPortSlot, IMPLIED testPortName`
fn index_text(details: &NodeDetails) -> String {
    details
        .index
        .iter()
        .map(|part| {
            if part.implied {
                format!("IMPLIED {}", part.name)
            } else {
                part.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Other modules that define the same name, when the query did not say which
/// module it meant.
fn other_definitions(set: &MibSet, query: &str, node: Node<'_>) -> String {
    if query.contains("::") {
        return String::new();
    }
    set.find_all(node.name())
        .into_iter()
        .filter(|other| *other != node)
        .map(|other| format!("{} ({})", other.qualified_name(), other.oid()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Print a DESCRIPTION or REFERENCE text indented under its label.
///
/// MIB files indent continuation lines to line up with the opening quote, so
/// that common indentation is removed first.
fn write_block(text: &str, out: &mut impl Write) -> std::io::Result<()> {
    let lines: Vec<&str> = text.lines().collect();
    let indent = lines
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    for (i, line) in lines.iter().enumerate() {
        let line = if i == 0 {
            line.trim()
        } else {
            line.get(indent..).unwrap_or(line.trim_start()).trim_end()
        };
        if line.is_empty() {
            writeln!(out)?;
        } else {
            writeln!(out, "    {line}")?;
        }
    }
    Ok(())
}

fn join<T: ToString>(items: &[T], separator: &str) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(separator)
}

/// Tell the user (on stderr) that a query matched nothing, and how many
/// modules were searched, since a missing MIB folder is the usual cause.
fn not_found(set: &MibSet, query: &str) {
    eprintln!(
        "ferosnmp: {query}: not found in the {} loaded modules (add MIB folders with -M)",
        set.modules().len()
    );
}

/// `ferosnmp mib tree`: print the tree below `root`, one node per line,
/// indented by level.
pub fn tree(
    set: &MibSet,
    root: Option<&str>,
    depth: Option<usize>,
    out: &mut impl Write,
) -> Result<Status, CliError> {
    // Each entry is a node and its level below the start.
    let mut stack: Vec<(Node<'_>, usize)> = Vec::new();
    match root {
        Some(query) => {
            let Some(node) = set.resolve(query) else {
                not_found(set, query);
                return Ok(Status::Problems);
            };
            stack.push((node, 0));
        }
        None => {
            // The real root is an unnamed node above `iso`, `ccitt` and
            // `joint-iso-ccitt`; start with those. Pushed in reverse so the
            // lowest arc comes off the stack first.
            let mut top: Vec<_> = set.root().children().collect();
            top.reverse();
            stack.extend(top.into_iter().map(|node| (node, 0)));
        }
    }

    while let Some((node, level)) = stack.pop() {
        write_tree_line(node, level, out)?;
        if depth.is_none_or(|max| level < max) {
            let mut children: Vec<_> = node.children().collect();
            children.reverse();
            stack.extend(children.into_iter().map(|child| (child, level + 1)));
        }
    }
    Ok(Status::Success)
}

/// One line of `mib tree`, such as `    sysDescr(1)  scalar`. The first line
/// also shows the full OID, so the reader knows where the tree starts.
fn write_tree_line(node: Node<'_>, level: usize, out: &mut impl Write) -> std::io::Result<()> {
    let indent = "  ".repeat(level);
    let name = node.name();
    let label = if name.is_empty() {
        node.arc().to_string()
    } else {
        format!("{name}({})", node.arc())
    };
    write!(out, "{indent}{label}")?;
    if level == 0 {
        write!(out, "  {}", node.oid())?;
    }
    match node.kind() {
        // Plain OBJECT IDENTIFIER nodes are the branches; only mark the rest.
        NodeKind::Internal | NodeKind::Node => writeln!(out),
        kind => writeln!(out, "  [{kind}]"),
    }
}

/// `ferosnmp mib search`: print every node whose name matches, in OID order.
/// A pattern with `::` is matched against `MODULE::name`.
pub fn search(set: &MibSet, pattern: &str, out: &mut impl Write) -> Result<Status, CliError> {
    let compiled = Pattern::new(pattern);
    let qualified = pattern.contains("::");

    let mut matches: Vec<Node<'_>> = Vec::new();
    let mut stack: Vec<Node<'_>> = vec![set.root()];
    while let Some(node) = stack.pop() {
        if !node.name().is_empty() {
            let text = if qualified {
                node.qualified_name()
            } else {
                node.name().to_string()
            };
            if compiled.matches(&text) {
                matches.push(node);
            }
        }
        let mut children: Vec<_> = node.children().collect();
        children.reverse();
        stack.extend(children);
    }

    if matches.is_empty() {
        eprintln!("ferosnmp: no names match {pattern:?}");
        return Ok(Status::Problems);
    }

    // Line the columns up: name, OID, kind.
    let rows: Vec<(String, String, NodeKind)> = matches
        .iter()
        .map(|node| (node.qualified_name(), node.oid().to_string(), node.kind()))
        .collect();
    let name_width = rows.iter().map(|row| row.0.len()).max().unwrap_or(0);
    let oid_width = rows.iter().map(|row| row.1.len()).max().unwrap_or(0);
    for (name, oid, kind) in &rows {
        writeln!(out, "{name:name_width$}  {oid:oid_width$}  {kind}")?;
    }
    Ok(Status::Success)
}

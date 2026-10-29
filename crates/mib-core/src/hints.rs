//! Fix hints: a concrete suggestion for each kind of problem that has a
//! common, mechanical fix, and a few checks of our own that mib-rs does not
//! make.
//!
//! The hints use what is known after loading: which modules define a symbol,
//! which files were found, and which folders were searched. Every hint has a
//! fixture MIB in `tests/hints/` that triggers it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mib_rs::SyntaxKind;
use mib_rs::types::ResolutionDomain;

use crate::convert;
use crate::diagnostic::{Diagnostic, Location, Severity};
use crate::library::Strictness;
use crate::source::line_column;

/// Everything the hints need besides the diagnostic itself.
pub(crate) struct HintContext<'a> {
    pub mib: &'a mib_rs::Mib,
    /// The folders that were searched for modules, in search order.
    pub folders: &'a [PathBuf],
}

/// Fill in [`Diagnostic::hint`] and [`Diagnostic::symbol`] where we can.
pub(crate) fn add_hints(context: &HintContext<'_>, diagnostics: &mut [Diagnostic]) {
    for diagnostic in diagnostics {
        if diagnostic.hint.is_some() {
            continue; // one of our own checks, which come with their hint
        }
        let quoted = quoted_strings(&diagnostic.message);
        diagnostic.symbol = symbol(&diagnostic.code, &quoted);
        diagnostic.hint = hint(context, diagnostic, &quoted);
    }
}

/// The symbol a diagnostic is about. mib-rs quotes names in its messages:
/// the first quoted name is the definition the problem is in, the last one
/// the name it refers to.
fn symbol(code: &str, quoted: &[String]) -> Option<String> {
    match code {
        "type-unknown"
        | "oid-orphan"
        | "index-unresolved"
        | "objects-unresolved"
        | "group-member-unresolved" => quoted.last().cloned(),
        "import-not-found"
        | "import-module-not-found"
        | "identifier-underscore"
        | "identifier-hyphen-end"
        | "identifier-hyphen-smiv2"
        | "identifier-length-64"
        | "identifier-length-32"
        | "bad-identifier-case" => quoted.first().cloned(),
        _ => None,
    }
}

fn hint(context: &HintContext<'_>, diagnostic: &Diagnostic, quoted: &[String]) -> Option<String> {
    let module = diagnostic.module.as_deref().unwrap_or("");
    let symbol = diagnostic.symbol.as_deref();
    match diagnostic.code.as_str() {
        "type-unknown" => missing_import(context, module, symbol?, Wanted::Type),
        "oid-orphan" | "index-unresolved" | "objects-unresolved" | "group-member-unresolved" => {
            missing_import(context, module, symbol?, Wanted::Value)
        }
        "import-not-found" => wrong_import(context, symbol?, quoted.get(1)?),
        "import-module-not-found" => missing_module(context, symbol?, quoted.get(1)?),

        "identifier-underscore" | "identifier-hyphen-smiv2" => Some(format!(
            "Rename it to {}: SMIv2 names are letters and digits only, \
             in camelCase. Change every reference to it too.",
            camel_case(symbol?)
        )),
        "identifier-hyphen-end" => Some(format!(
            "Remove the hyphen at the end: {}.",
            symbol?.trim_end_matches('-')
        )),
        "identifier-length-64" => {
            Some("Shorten the name to at most 64 characters (32 or fewer is best).".to_string())
        }
        "identifier-length-32" => Some(
            "Names over 32 characters do not work with some older tools; \
             shorten it if you can."
                .to_string(),
        ),
        "bad-identifier-case" => {
            let name = symbol?;
            let mut chars = name.chars();
            let first = chars.next()?.to_lowercase();
            Some(format!(
                "Names of objects and other values start with a lowercase \
                 letter; rename it to {first}{}. Names starting with an \
                 uppercase letter are for types and modules.",
                chars.as_str()
            ))
        }

        "access-in-smiv2" => Some("Write MAX-ACCESS instead of ACCESS (RFC 2578).".to_string()),
        "maxaccess-in-smiv1" => Some(
            "Write ACCESS instead of MAX-ACCESS, or make the whole module SMIv2: \
             import from SNMPv2-SMI and add a MODULE-IDENTITY."
                .to_string(),
        ),
        "status-invalid-smiv2" if diagnostic.message.contains("mandatory") => {
            Some("Write STATUS current instead of mandatory (RFC 3584).".to_string())
        }
        "status-invalid-smiv2" => Some(
            "SMIv2 has no STATUS optional; use deprecated or obsolete \
             (RFC 3584)."
                .to_string(),
        ),
        "integer-in-smiv2" => Some(
            "Use Integer32 for a number, or list the values \
             (INTEGER { up(1), down(2) }) for an enumeration."
                .to_string(),
        ),
        "trap-in-smiv2" => Some(TRAP_TYPE_HINT.to_string()),
        "type-assignment-smiv2" => Some(
            "Define the type as a TEXTUAL-CONVENTION, with STATUS and DESCRIPTION.".to_string(),
        ),
        _ => None,
    }
}

const TRAP_TYPE_HINT: &str = "Rewrite it as a NOTIFICATION-TYPE: OBJECTS instead of VARIABLES, \
     add STATUS current, drop ENTERPRISE, and give it the OID \
     { <enterprise> 0 <trap number> } so it keeps its trap OID (RFC 3584).";

/// Which kind of definition a reference needs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wanted {
    /// A type, in SYNTAX.
    Type,
    /// Anything with an OID: an object, a node, a notification...
    Value,
}

/// The modules that define `symbol`, other than `except`, in name order.
fn defining_modules(mib: &mib_rs::Mib, symbol: &str, wanted: Wanted, except: &str) -> Vec<String> {
    let domain = match wanted {
        Wanted::Type => ResolutionDomain::Type,
        Wanted::Value => ResolutionDomain::Oid,
    };
    let Ok(trace) = mib.trace_symbol(symbol, None, domain) else {
        return Vec::new();
    };
    let modules: BTreeSet<String> = trace
        .candidates
        .into_iter()
        .filter(|candidate| candidate.applicable && candidate.module_name != except)
        .map(|candidate| candidate.module_name)
        .collect();
    modules.into_iter().collect()
}

/// A reference to a name that is not defined or imported in its module.
fn missing_import(
    context: &HintContext<'_>,
    module: &str,
    symbol: &str,
    wanted: Wanted,
) -> Option<String> {
    let mib = context.mib;
    if let Some(declared) = mib
        .module(module)
        .and_then(|m| m.imports().iter().find(|import| imports(import, symbol)))
    {
        // The import is there but failed; that diagnostic has the real hint.
        return Some(format!(
            "{symbol} is imported FROM {}, but that import failed; fix the \
             IMPORTS first.",
            declared.module
        ));
    }
    let modules = defining_modules(mib, symbol, wanted, module);
    match modules.as_slice() {
        [] => Some(format!(
            "No loaded module defines {symbol}. Check the spelling, or load \
             the MIB that defines it (add its folder to the MIB folders)."
        )),
        [only] => Some(format!(
            "{symbol} is defined in {only}: add `{symbol} FROM {only}` to the IMPORTS."
        )),
        [first, others @ ..] => Some(format!(
            "{symbol} is defined in {first}: add `{symbol} FROM {first}` to \
             the IMPORTS (it is also defined in {}).",
            others.join(", ")
        )),
    }
}

fn imports(import: &mib_rs::mib::Import, symbol: &str) -> bool {
    import
        .symbols
        .iter()
        .any(|imported| imported.name == symbol)
}

/// `IMPORTS symbol FROM declared`, where `declared` was found but does not
/// define `symbol`.
fn wrong_import(context: &HintContext<'_>, symbol: &str, declared: &str) -> Option<String> {
    let wanted = if starts_uppercase(symbol) {
        Wanted::Type
    } else {
        Wanted::Value
    };
    let modules = defining_modules(context.mib, symbol, wanted, declared);
    match modules.first() {
        Some(module) => Some(format!(
            "{declared} does not define {symbol}, but {module} does: change the \
             IMPORTS to `{symbol} FROM {module}`."
        )),
        None => Some(format!(
            "{declared} does not define {symbol}, and no other loaded module \
             does. Check the spelling, or load the newer version of {declared} \
             that added it."
        )),
    }
}

/// `IMPORTS symbol FROM missing`, where no module called `missing` was found.
fn missing_module(context: &HintContext<'_>, symbol: &str, missing: &str) -> Option<String> {
    // A file named after the module that defines a module with another name:
    // the classic vendor mistake, and invisible without looking inside.
    for module in context.mib.modules() {
        let Some(file) = module_file(module) else {
            continue;
        };
        if module.name() != missing && file_stem_matches(&file, missing) {
            return Some(format!(
                "The file {} is named {missing}, but the module in it is \
                 called {}: change the IMPORTS to FROM {}.",
                file.display(),
                module.name(),
                module.name()
            ));
        }
    }

    let wanted = if starts_uppercase(symbol) {
        Wanted::Type
    } else {
        Wanted::Value
    };
    if let Some(module) = defining_modules(context.mib, symbol, wanted, "").first() {
        return Some(format!(
            "No module {missing} was found, but {module} defines {symbol}: \
             change the IMPORTS to `{symbol} FROM {module}`, or add the folder \
             that has {missing} to the MIB folders."
        ));
    }

    let searched = if context.folders.is_empty() {
        "only the built-in modules were searched".to_string()
    } else {
        let folders: Vec<String> = context
            .folders
            .iter()
            .map(|folder| folder.display().to_string())
            .collect();
        format!("searched: {}", folders.join(", "))
    };
    Some(format!(
        "{missing} is not in any MIB folder ({searched}). Get the MIB file \
         from the vendor and add its folder to the MIB folders \
         (Settings in the GUI, -M on the command line)."
    ))
}

/// The file a module was loaded from, if it came from a file.
fn module_file(module: mib_rs::Module<'_>) -> Option<PathBuf> {
    match module.source_origin()? {
        mib_rs::SourceOrigin::File { path } => Some(path.clone()),
        _ => None,
    }
}

/// True if the file name, without a MIB extension, is `module` (ignoring case).
fn file_stem_matches(file: &Path, module: &str) -> bool {
    let Some(name) = file.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let stem = match name.rsplit_once('.') {
        Some((stem, extension))
            if ["mib", "smi", "txt", "my"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known)) =>
        {
            stem
        }
        _ => name,
    };
    stem.eq_ignore_ascii_case(module)
}

fn starts_uppercase(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
}

/// `bad_name-here` as `badNameHere`.
fn camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper_next = false;
    for c in name.chars() {
        if c == '_' || c == '-' {
            upper_next = !out.is_empty();
        } else if upper_next {
            out.extend(c.to_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// The texts between double quotes in a message, in order. mib-rs writes
/// names with Rust's `{:?}`, so `\"` and `\\` are unescaped.
fn quoted_strings(message: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut chars = message.chars();
    while chars.by_ref().any(|c| c == '"') {
        let mut text = String::new();
        let mut closed = false;
        while let Some(c) = chars.next() {
            match c {
                '\\' => text.extend(chars.next()),
                '"' => {
                    closed = true;
                    break;
                }
                c => text.push(c),
            }
        }
        if closed {
            strings.push(text);
        }
    }
    strings
}

// ----- checks of our own -----

/// Problems mib-rs does not report: SMIv1 constructs in a module that is
/// otherwise SMIv2, and (only when strict, like other informational
/// diagnostics) files not named after their module.
pub(crate) fn extra_diagnostics(mib: &mib_rs::Mib, strictness: Strictness) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for module in mib.modules() {
        if module.is_base() {
            continue;
        }
        let Some(document) = module.source() else {
            continue;
        };
        let file = convert::source_file(document.origin(), document.label());
        if module.language() == mib_rs::Language::Unknown {
            diagnostics.extend(mixed_smi(module.name(), &file, document.bytes()));
        }
        if strictness == Strictness::Strict
            && let Some(path) = module_file(module)
            && !file_stem_matches(&path, module.name())
        {
            diagnostics.push(file_name_mismatch(module.name(), &path, document.bytes()));
        }
    }
    diagnostics
}

/// mib-rs decides a module is SMIv1 or SMIv2 from its imports, MODULE-IDENTITY
/// and TRAP-TYPEs. A module with evidence for both is "unknown", and then none
/// of the version checks run, so its mistakes would go unreported. Find the
/// SMIv1 constructs in such a module ourselves.
fn mixed_smi(module: &str, file: &Path, bytes: &[u8]) -> Vec<Diagnostic> {
    let candidate = mib_rs::SourceCandidate::new(
        mib_rs::CandidateId::new(file.display().to_string()),
        mib_rs::SourceOrigin::File {
            path: file.to_path_buf(),
        },
        file.display().to_string(),
        bytes.to_vec(),
    );
    let Ok((tree, _)) = mib_rs::cst::parse(candidate) else {
        return Vec::new();
    };

    // The module's own node, in case the file defines several modules.
    let Some(node) = tree.nodes().find(|node| {
        node.kind() == SyntaxKind::Module
            && node
                .descendant_tokens()
                .find(|token| token.kind() == SyntaxKind::UppercaseIdent)
                .is_some_and(|token| token.text() == module.as_bytes())
    }) else {
        return Vec::new();
    };
    let Some(identity) = node
        .descendant_nodes()
        .find(|node| node.kind() == SyntaxKind::ModuleIdentityDefinition)
    else {
        // No MODULE-IDENTITY: the SMIv2 evidence is an import, and an SMIv1
        // module importing one SMIv2 type is common and harmless.
        return Vec::new();
    };
    let (identity_line, _) = line_column(bytes, identity.range().start().as_usize());
    let is_smiv2 = format!("this module is SMIv2 (MODULE-IDENTITY at line {identity_line})");

    let mut found: Vec<(usize, usize, String, String)> = Vec::new();
    for child in node.descendant_nodes() {
        match child.kind() {
            SyntaxKind::TrapTypeDefinition => found.push((
                child.range().start().as_usize(),
                child.range().end().as_usize(),
                format!("TRAP-TYPE is SMIv1, but {is_smiv2}"),
                TRAP_TYPE_HINT.to_string(),
            )),
            SyntaxKind::ImportGroup => {
                // `symbols FROM MODULE`: the module is the last identifier.
                let Some(from) = child
                    .descendant_tokens()
                    .filter(|token| token.kind() == SyntaxKind::UppercaseIdent)
                    .last()
                else {
                    continue;
                };
                let name = String::from_utf8_lossy(from.text()).into_owned();
                if ["RFC1155-SMI", "RFC-1212", "RFC-1215"].contains(&name.as_str()) {
                    found.push((
                        from.range().start().as_usize(),
                        from.range().end().as_usize(),
                        format!("{name} is the SMIv1 framework, but {is_smiv2}"),
                        "Import the SMI from SNMPv2-SMI instead (and DisplayString \
                         and other textual conventions from SNMPv2-TC)."
                            .to_string(),
                    ));
                }
            }
            _ => {}
        }
    }
    for token in node.descendant_tokens() {
        let (message, hint) = match token.kind() {
            SyntaxKind::KwAccess => (
                format!("ACCESS is SMIv1, but {is_smiv2}"),
                "Write MAX-ACCESS instead of ACCESS (RFC 2578).",
            ),
            SyntaxKind::KwMandatory => (
                format!("STATUS mandatory is SMIv1, but {is_smiv2}"),
                "Write STATUS current instead of mandatory (RFC 3584).",
            ),
            SyntaxKind::KwOptional => (
                format!("STATUS optional is SMIv1, but {is_smiv2}"),
                "SMIv2 has no STATUS optional; use deprecated or obsolete (RFC 3584).",
            ),
            _ => continue,
        };
        found.push((
            token.range().start().as_usize(),
            token.range().end().as_usize(),
            message,
            hint.to_string(),
        ));
    }
    found.sort_by_key(|entry| entry.0);

    found
        .into_iter()
        .map(|(start, end, message, hint)| Diagnostic {
            severity: Severity::Warning,
            code: "smiv1-in-smiv2".to_string(),
            module: Some(module.to_string()),
            location: Some(location(file, bytes, start, end)),
            message,
            hint: Some(hint),
            symbol: None,
        })
        .collect()
}

/// A module in a file not named after it. FeroSNMP does not mind, but
/// net-snmp, smilint and most other tools find modules by file name.
fn file_name_mismatch(module: &str, file: &Path, bytes: &[u8]) -> Diagnostic {
    let location = find_definitions(bytes, module)
        .map(|start| location(file, bytes, start, start + module.len()));
    let file_name = file
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    Diagnostic {
        severity: Severity::Info,
        code: "module-file-name-mismatch".to_string(),
        module: Some(module.to_string()),
        location: location.or_else(|| {
            Some(Location {
                file: file.to_path_buf(),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            })
        }),
        message: format!("module {module} is in a file named {file_name}"),
        hint: Some(format!(
            "Rename the file to {module}.mib (or {module} with no extension); \
             net-snmp and smilint look modules up by file name and will not \
             find it otherwise."
        )),
        symbol: None,
    }
}

/// Where `module DEFINITIONS` starts in the text.
fn find_definitions(bytes: &[u8], module: &str) -> Option<usize> {
    let name = module.as_bytes();
    let mut from = 0;
    while let Some(found) = find(&bytes[from..], name) {
        let start = from + found;
        let rest = &bytes[start + name.len()..];
        let after_space = rest
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let boundary = start == 0 || !is_identifier_byte(bytes[start - 1]);
        if boundary && after_space > 0 && rest[after_space..].starts_with(b"DEFINITIONS") {
            return Some(start);
        }
        from = start + 1;
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn is_identifier_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn location(file: &Path, bytes: &[u8], start: usize, end: usize) -> Location {
    let (line, column) = line_column(bytes, start);
    let (end_line, end_column) = line_column(bytes, end);
    Location {
        file: file.to_path_buf(),
        line,
        column,
        end_line,
        end_column,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_strings_are_unescaped() {
        assert_eq!(
            quoted_strings(r#"unresolved import: "a\"b" from "X-MIB" (module_not_found)"#),
            ["a\"b", "X-MIB"]
        );
        assert!(quoted_strings("no quotes").is_empty());
    }

    #[test]
    fn camel_case_drops_separators() {
        assert_eq!(camel_case("bad_underscore"), "badUnderscore");
        assert_eq!(camel_case("bad-hyphen-name"), "badHyphenName");
        assert_eq!(camel_case("_x"), "x");
    }

    #[test]
    fn file_stems_ignore_case_and_mib_extensions() {
        assert!(file_stem_matches(Path::new("dir/IF-MIB"), "IF-MIB"));
        assert!(file_stem_matches(Path::new("dir/if-mib.my"), "IF-MIB"));
        assert!(!file_stem_matches(
            Path::new("dir/rfc1213.txt"),
            "RFC1213-MIB"
        ));
        assert!(!file_stem_matches(Path::new("dir/IF-MIB.bak"), "IF-MIB"));
    }

    #[test]
    fn definitions_line_is_found() {
        let text = b"-- The FOO-MIB module\nFOO-MIB\n  DEFINITIONS ::= BEGIN";
        assert_eq!(find_definitions(text, "FOO-MIB"), Some(22));
        assert_eq!(find_definitions(b"XFOO-MIB DEFINITIONS", "FOO-MIB"), None);
    }
}

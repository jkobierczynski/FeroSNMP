//! Problems found while reading MIB files.

use std::fmt;
use std::path::PathBuf;

/// How serious a [`Diagnostic`] is, from most to least severe.
///
/// The levels follow libsmi's `smilint`, so results can be compared with it.
/// The ordering puts the most severe level first: `Severity::Fatal <
/// Severity::Info`, so "at least as bad as an error" is `severity <=
/// Severity::Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The module could not be loaded at all.
    Fatal,
    /// A serious error; the module is loaded but parts are likely missing.
    Severe,
    /// A violation of the SMI rules.
    Error,
    /// A minor violation that most tools accept.
    Minor,
    /// A style problem.
    Style,
    /// Something that is allowed but suspicious.
    Warning,
    /// Information only.
    Info,
}

impl Severity {
    /// Lower-case name, as printed in diagnostics (`"error"`, `"warning"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Fatal => "fatal",
            Severity::Severe => "severe",
            Severity::Error => "error",
            Severity::Minor => "minor",
            Severity::Style => "style",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }

    /// True for [`Fatal`](Severity::Fatal), [`Severe`](Severity::Severe) and
    /// [`Error`](Severity::Error).
    pub fn is_error(self) -> bool {
        self <= Severity::Error
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where in a source file a diagnostic points. Lines and columns start at 1.
///
/// Columns count characters, not bytes, except in files that are not valid
/// UTF-8, where they count bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    /// The file, or `embedded:<MODULE>` for a built-in foundation module.
    pub file: PathBuf,
    /// Line of the first character.
    pub line: u32,
    /// Column of the first character.
    pub column: u32,
    /// Line just after the last character.
    pub end_line: u32,
    /// Column just after the last character.
    pub end_column: u32,
}

/// One problem found while reading or resolving MIB modules.
///
/// Its [`Display`](fmt::Display) form is the usual compiler style, without the
/// hint:
///
/// ```text
/// corpus/ietf/BFD-STD-MIB:14:6: error: BFD-STD-MIB: unresolved import: ... [import-module-not-found]
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    /// How serious the problem is.
    pub severity: Severity,
    /// Stable, machine-readable identifier such as `"import-module-not-found"`.
    pub code: String,
    /// The module the problem was found in, if known.
    pub module: Option<String>,
    /// Where the problem is, if it points at source text.
    pub location: Option<Location>,
    /// Human-readable description.
    pub message: String,
    /// A concrete suggestion for fixing the problem, when one is known, such
    /// as "add `DisplayString` to the IMPORTS FROM SNMPv2-TC".
    pub hint: Option<String>,
    /// The symbol the problem is about, when it names one: the identifier
    /// that could not be resolved, or the badly formed name. Use it with
    /// [`MibSet::trace`](crate::MibSet::trace) to see why it did not resolve.
    pub symbol: Option<String>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(location) = &self.location {
            write!(
                f,
                "{}:{}:{}: ",
                location.file.display(),
                location.line,
                location.column
            )?;
        }
        write!(f, "{}: ", self.severity)?;
        if let Some(module) = &self.module {
            write!(f, "{module}: ")?;
        }
        write!(f, "{} [{}]", self.message, self.code)
    }
}

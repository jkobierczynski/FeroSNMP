//! What the Diagnostics tab shows, and the export formats.
//!
//! The list is a virtual list view: Windows asks for the text of each visible
//! cell as it paints, so even tens of thousands of diagnostics cost nothing
//! until they are scrolled into view. This module answers those questions and
//! writes the exports; the window code only passes the text on.

use std::fmt::Write;
use std::path::Path;

use mib_core::{Diagnostic, Severity};
use serde_json::json;

/// A column of the Diagnostics list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Severity,
    Module,
    File,
    Line,
    Column,
    Message,
    Hint,
}

impl Field {
    /// Every column, in display order.
    pub const ALL: [Field; 7] = [
        Field::Severity,
        Field::Module,
        Field::File,
        Field::Line,
        Field::Column,
        Field::Message,
        Field::Hint,
    ];

    /// The column heading.
    pub fn title(self) -> &'static str {
        match self {
            Field::Severity => "Severity",
            Field::Module => "Module",
            Field::File => "File",
            Field::Line => "Line",
            Field::Column => "Column",
            Field::Message => "Message",
            Field::Hint => "Fix hint",
        }
    }

    /// Width at 96 DPI.
    pub fn width(self) -> i32 {
        match self {
            Field::Severity => 70,
            Field::Module => 170,
            Field::File => 170,
            Field::Line => 50,
            Field::Column => 55,
            Field::Message => 360,
            Field::Hint => 360,
        }
    }

    /// True for the number columns, which are right-aligned.
    pub fn is_number(self) -> bool {
        matches!(self, Field::Line | Field::Column)
    }
}

/// The diagnostics of a loaded set, in the order the list shows them: the
/// most severe first, then by file and position.
#[derive(Debug, Clone, Default)]
pub struct DiagnosticList {
    rows: Vec<Diagnostic>,
}

impl DiagnosticList {
    pub fn new(diagnostics: &[Diagnostic]) -> Self {
        let mut rows = diagnostics.to_vec();
        rows.sort_by(|a, b| {
            let position = |d: &Diagnostic| {
                d.location
                    .as_ref()
                    .map(|l| (l.file.clone(), l.line, l.column))
            };
            (a.severity, position(a), &a.message).cmp(&(b.severity, position(b), &b.message))
        });
        DiagnosticList { rows }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn get(&self, row: usize) -> Option<&Diagnostic> {
        self.rows.get(row)
    }

    pub fn rows(&self) -> &[Diagnostic] {
        &self.rows
    }

    /// The text of one cell.
    pub fn cell(&self, row: usize, column: Field) -> String {
        let Some(d) = self.rows.get(row) else {
            return String::new();
        };
        let location = d.location.as_ref();
        match column {
            Field::Severity => d.severity.to_string(),
            Field::Module => d.module.clone().unwrap_or_default(),
            Field::File => location
                .and_then(|l| l.file.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Field::Line => location.map(|l| l.line.to_string()).unwrap_or_default(),
            Field::Column => location.map(|l| l.column.to_string()).unwrap_or_default(),
            Field::Message => format!("{} [{}]", d.message, d.code),
            Field::Hint => d.hint.clone().unwrap_or_default(),
        }
    }

    /// The first row about `file`, at or after `line`, to keep the viewer on
    /// the same place after a reload.
    pub fn find(&self, file: &Path, line: u32) -> Option<usize> {
        let in_file = |d: &Diagnostic| d.location.as_ref().is_some_and(|l| l.file == file);
        let mut best: Option<(u32, usize)> = None;
        for (row, d) in self.rows.iter().enumerate() {
            if !in_file(d) {
                continue;
            }
            let distance = d.location.as_ref().unwrap().line.abs_diff(line);
            if best.is_none_or(|(known, _)| distance < known) {
                best = Some((distance, row));
            }
        }
        best.map(|(_, row)| row)
    }

    /// "3 errors, 12 warnings", for the tab and the status bar.
    pub fn summary(&self) -> String {
        let errors = self.rows.iter().filter(|d| d.severity.is_error()).count();
        let others = self.rows.len() - errors;
        format!(
            "{errors} {}, {others} {}",
            if errors == 1 { "error" } else { "errors" },
            if others == 1 { "warning" } else { "warnings" }
        )
    }
}

/// The file formats diagnostics can be exported in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One problem per line, as `ferosnmp lint` prints them.
    Text,
    /// The JSON document of `ferosnmp lint --format json`.
    Json,
}

impl ExportFormat {
    /// The format for a file name: JSON for `.json`, text for anything else.
    pub fn for_path(path: &Path) -> ExportFormat {
        match path.extension() {
            Some(extension) if extension.eq_ignore_ascii_case("json") => ExportFormat::Json,
            _ => ExportFormat::Text,
        }
    }
}

/// The diagnostics in `format`.
pub fn export(diagnostics: &[Diagnostic], modules: &[String], format: ExportFormat) -> String {
    match format {
        ExportFormat::Text => export_text(diagnostics, modules.len()),
        ExportFormat::Json => export_json(diagnostics, modules),
    }
}

/// Lines in the `ferosnmp lint` text format, with the hint indented below.
fn export_text(diagnostics: &[Diagnostic], modules: usize) -> String {
    let mut out = String::new();
    for d in diagnostics {
        // Diagnostic's Display is the lint format without the hint.
        let _ = writeln!(out, "{d}");
        if let Some(hint) = &d.hint {
            let _ = writeln!(out, "    hint: {hint}");
        }
    }
    let errors = diagnostics.iter().filter(|d| d.severity.is_error()).count();
    let _ = writeln!(
        out,
        "{modules} {} checked: {errors} {}, {} {}",
        if modules == 1 { "module" } else { "modules" },
        if errors == 1 { "error" } else { "errors" },
        diagnostics.len() - errors,
        if diagnostics.len() - errors == 1 {
            "warning"
        } else {
            "warnings"
        },
    );
    out
}

/// The `ferosnmp lint --format json` document.
fn export_json(diagnostics: &[Diagnostic], modules: &[String]) -> String {
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity <= Severity::Error)
        .count();
    let rows: Vec<_> = diagnostics
        .iter()
        .map(|d| {
            let location = d.location.as_ref();
            json!({
                "severity": d.severity.as_str(),
                "code": d.code,
                "module": d.module,
                "file": location.map(|l| l.file.display().to_string()),
                "line": location.map(|l| l.line),
                "column": location.map(|l| l.column),
                "end_line": location.map(|l| l.end_line),
                "end_column": location.map(|l| l.end_column),
                "message": d.message,
                "hint": d.hint,
            })
        })
        .collect();
    let document = json!({
        "modules": modules,
        "errors": errors,
        "warnings": diagnostics.len() - errors,
        "diagnostics": rows,
    });
    let mut text = serde_json::to_string_pretty(&document).expect("JSON values always serialize");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ietf_set;

    #[test]
    fn errors_come_first() {
        let set = ietf_set();
        let list = DiagnosticList::new(set.diagnostics());
        assert_eq!(list.len(), set.diagnostics().len());
        let severities: Vec<Severity> = list.rows().iter().map(|d| d.severity).collect();
        assert!(severities.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(list.cell(0, Field::Severity), "error");
        assert!(list.cell(0, Field::Message).ends_with(']'));
        assert!(
            !list.cell(0, Field::Hint).is_empty(),
            "the corpus errors have hints"
        );
        assert_eq!(list.cell(usize::MAX, Field::Module), "");
    }

    #[test]
    fn find_picks_the_nearest_line_in_the_file() {
        let set = ietf_set();
        let list = DiagnosticList::new(set.diagnostics());
        let first = list.get(0).unwrap().location.clone().unwrap();
        let row = list.find(&first.file, first.line + 1).unwrap();
        assert_eq!(
            list.get(row).unwrap().location.as_ref().unwrap().file,
            first.file
        );
        assert_eq!(list.find(Path::new("no-such-file"), 1), None);
    }

    #[test]
    fn exports_match_the_cli_formats() {
        let set = ietf_set();
        let list = DiagnosticList::new(set.diagnostics());
        let modules = vec!["A-MIB".to_string(), "B-MIB".to_string()];

        let text = export(list.rows(), &modules, ExportFormat::Text);
        assert!(text.contains("    hint: "), "{text}");
        assert!(
            text.ends_with(&format!("2 modules checked: {}\n", list.summary())),
            "{text}"
        );

        let json: serde_json::Value =
            serde_json::from_str(&export(list.rows(), &modules, ExportFormat::Json)).unwrap();
        assert_eq!(json["modules"][1], "B-MIB");
        assert_eq!(json["diagnostics"].as_array().unwrap().len(), list.len());
        assert_eq!(json["diagnostics"][0]["severity"], "error");
        assert!(json["diagnostics"][0]["hint"].is_string());
    }

    #[test]
    fn export_format_follows_the_extension() {
        assert_eq!(
            ExportFormat::for_path(Path::new("a.JSON")),
            ExportFormat::Json
        );
        assert_eq!(
            ExportFormat::for_path(Path::new("a.txt")),
            ExportFormat::Text
        );
        assert_eq!(ExportFormat::for_path(Path::new("a")), ExportFormat::Text);
    }
}

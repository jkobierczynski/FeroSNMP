//! The `lint` command: load MIB files the way the browser does and report
//! every problem found in them.

use std::collections::{BTreeSet, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use mib_core::{Diagnostic, LoadError, Severity, Strictness};
use serde_json::json;

use crate::sources::{MibSources, load_error};
use crate::{CliError, Status};

/// Output format of `lint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// One problem per line, `file:line:column: severity: ...`, as compilers do.
    Text,
    /// One JSON document, for other tools.
    Json,
}

/// Lowest severity that makes `lint` exit with 1. The levels are those of
/// libsmi's `smilint`, from most to least severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FailOn {
    Fatal,
    Severe,
    Error,
    Minor,
    Style,
    Warning,
    Info,
}

impl FailOn {
    fn severity(self) -> Severity {
        match self {
            FailOn::Fatal => Severity::Fatal,
            FailOn::Severe => Severity::Severe,
            FailOn::Error => Severity::Error,
            FailOn::Minor => Severity::Minor,
            FailOn::Style => Severity::Style,
            FailOn::Warning => Severity::Warning,
            FailOn::Info => Severity::Info,
        }
    }
}

/// Everything the `lint` command was asked to do.
#[derive(Debug)]
pub struct Options {
    pub paths: Vec<PathBuf>,
    pub strict: bool,
    pub format: Format,
    pub fail_on: FailOn,
    pub sources: MibSources,
}

/// One problem to report. Most come from mib-core's diagnostics; a few are
/// found by `lint` itself, such as a file that contains no module.
#[derive(Debug, Clone)]
struct Finding {
    severity: Severity,
    code: String,
    module: Option<String>,
    file: Option<PathBuf>,
    /// Line, column, end line and end column, when the problem points at text.
    position: Option<(u32, u32, u32, u32)>,
    message: String,
    /// How to fix it, when known.
    hint: Option<String>,
}

impl From<Diagnostic> for Finding {
    fn from(diagnostic: Diagnostic) -> Self {
        let (file, position) = match diagnostic.location {
            Some(l) => (
                Some(l.file),
                Some((l.line, l.column, l.end_line, l.end_column)),
            ),
            None => (None, None),
        };
        Finding {
            severity: diagnostic.severity,
            code: diagnostic.code,
            module: diagnostic.module,
            file,
            position,
            message: diagnostic.message,
            hint: diagnostic.hint,
        }
    }
}

/// What was given on the command line, sorted into folders and files.
#[derive(Debug, Default)]
struct Targets {
    /// Folders to lint every module in.
    folders: Vec<PathBuf>,
    /// Single files to lint.
    files: Vec<PathBuf>,
    /// The same, made absolute, to compare with the paths the loader reports.
    canonical_folders: Vec<PathBuf>,
    canonical_files: HashSet<PathBuf>,
}

impl Targets {
    fn from_paths(paths: &[PathBuf]) -> Result<Targets, CliError> {
        let mut targets = Targets::default();
        for path in paths {
            let canonical = path.canonicalize().map_err(|error| {
                CliError::Message(format!("cannot read {}: {error}", path.display()))
            })?;
            if canonical.is_dir() {
                targets.folders.push(path.clone());
                targets.canonical_folders.push(canonical);
            } else {
                targets.files.push(path.clone());
                targets.canonical_files.insert(canonical);
            }
        }
        Ok(targets)
    }

    /// The folders to load modules from: the folders asked for, and the
    /// folder of each file asked for, so that a file can import modules that
    /// sit next to it.
    fn search_folders(&self) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = Vec::new();
        let parents = self.files.iter().map(|file| match file.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        });
        for folder in self.folders.iter().cloned().chain(parents) {
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
        folders
    }

    /// True if `file` is one of the files asked for or inside a folder asked for.
    fn contains(&self, file: &Path) -> bool {
        let Ok(file) = file.canonicalize() else {
            return false;
        };
        self.canonical_files.contains(&file)
            || self
                .canonical_folders
                .iter()
                .any(|folder| file.starts_with(folder))
    }
}

/// `ferosnmp lint`.
pub fn run(options: &Options, out: &mut impl Write) -> Result<Status, CliError> {
    let targets = Targets::from_paths(&options.paths)?;
    let strictness = if options.strict {
        Strictness::Strict
    } else {
        // Not `Permissive`: that hides mistakes by finding imports wherever
        // they are, which is right for browsing but not for checking a MIB.
        Strictness::Normal
    };
    let library = options
        .sources
        .library(&targets.search_folders())
        .with_strictness(strictness);

    // Find the modules in the files to lint, by the name inside each file.
    let index = library.index().map_err(load_error)?;
    let mut modules: BTreeSet<String> = BTreeSet::new();
    let mut findings: Vec<Finding> = Vec::new();
    let mut files_with_modules: HashSet<PathBuf> = HashSet::new();
    for entry in &index {
        if !targets.contains(&entry.file) {
            continue;
        }
        if let Ok(file) = entry.file.canonicalize() {
            files_with_modules.insert(file);
        }
        if entry.shadowed {
            let used = index
                .iter()
                .find(|other| other.module == entry.module && !other.shadowed)
                .map_or_else(String::new, |other| other.file.display().to_string());
            findings.push(Finding {
                severity: Severity::Warning,
                code: "duplicate-module".to_string(),
                module: Some(entry.module.clone()),
                file: Some(entry.file.clone()),
                position: None,
                message: format!(
                    "module {} is also defined in {used}, which is loaded instead of this file",
                    entry.module
                ),
                hint: Some(
                    "Remove one of the two files, or move this one to a folder \
                     that is not searched."
                        .to_string(),
                ),
            });
        } else {
            modules.insert(entry.module.clone());
        }
    }
    findings.extend(missing_modules(&targets, &files_with_modules));

    // Load the modules and everything they import, and keep the diagnostics
    // about the modules being linted.
    if !modules.is_empty() {
        let names: Vec<&str> = modules.iter().map(String::as_str).collect();
        let diagnostics = match library.load_modules(&names) {
            Ok(set) => set.diagnostics().to_vec(),
            Err(LoadError::Fatal(diagnostics)) => diagnostics,
            Err(error) => return Err(load_error(error)),
        };
        findings.extend(
            diagnostics
                .into_iter()
                .filter(|d| is_about_targets(d, &modules, &targets))
                .map(Finding::from),
        );
    }

    findings.sort_by(|a, b| {
        (&a.file, a.position, &a.code, &a.message).cmp(&(&b.file, b.position, &b.code, &b.message))
    });

    match options.format {
        Format::Text => write_text(&findings, modules.len(), out)?,
        Format::Json => write_json(&findings, &modules, out)?,
    }

    let fail_at = options.fail_on.severity();
    if findings.iter().any(|f| f.severity <= fail_at) {
        Ok(Status::Problems)
    } else {
        Ok(Status::Success)
    }
}

/// A finding for every file or folder asked for in which no module was found.
fn missing_modules(targets: &Targets, files_with_modules: &HashSet<PathBuf>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for file in &targets.files {
        let found = file
            .canonicalize()
            .is_ok_and(|file| files_with_modules.contains(&file));
        if !found {
            findings.push(Finding {
                severity: Severity::Error,
                code: "no-module".to_string(),
                module: None,
                file: Some(file.clone()),
                position: None,
                message: "no MIB module found: the file must contain a \
                          `NAME DEFINITIONS ::= BEGIN` line, and have no \
                          extension or end in .mib, .smi, .txt or .my"
                    .to_string(),
                hint: None,
            });
        }
    }
    for (folder, canonical) in targets.folders.iter().zip(&targets.canonical_folders) {
        if !files_with_modules.iter().any(|f| f.starts_with(canonical)) {
            findings.push(Finding {
                severity: Severity::Error,
                code: "no-module".to_string(),
                module: None,
                file: Some(folder.clone()),
                position: None,
                message: "no MIB modules found in this folder".to_string(),
                hint: None,
            });
        }
    }
    findings
}

/// True if a diagnostic is about one of the modules or files being linted,
/// rather than about a module they import from elsewhere.
fn is_about_targets(
    diagnostic: &Diagnostic,
    modules: &BTreeSet<String>,
    targets: &Targets,
) -> bool {
    if let Some(module) = &diagnostic.module {
        return modules.contains(module);
    }
    match &diagnostic.location {
        Some(location) => targets.contains(&location.file),
        // Neither module nor file: a problem with the whole load. Report it.
        None => true,
    }
}

fn write_text(findings: &[Finding], modules: usize, out: &mut impl Write) -> std::io::Result<()> {
    for finding in findings {
        if let Some(file) = &finding.file {
            write!(out, "{}:", file.display())?;
            if let Some((line, column, _, _)) = finding.position {
                write!(out, "{line}:{column}:")?;
            }
            write!(out, " ")?;
        }
        write!(out, "{}: ", finding.severity)?;
        if let Some(module) = &finding.module {
            write!(out, "{module}: ")?;
        }
        writeln!(out, "{} [{}]", finding.message, finding.code)?;
        if let Some(hint) = &finding.hint {
            writeln!(out, "    hint: {hint}")?;
        }
    }

    let errors = findings.iter().filter(|f| f.severity.is_error()).count();
    let others = findings.len() - errors;
    writeln!(
        out,
        "{modules} {} checked: {errors} {}, {others} {}",
        plural(modules, "module", "modules"),
        plural(errors, "error", "errors"),
        plural(others, "warning", "warnings"),
    )
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

/// The JSON format:
///
/// ```json
/// {
///   "modules": ["BFD-STD-MIB", ...],
///   "errors": 12,
///   "warnings": 3,
///   "diagnostics": [
///     { "severity": "error", "code": "import-module-not-found",
///       "module": "BFD-STD-MIB", "file": "corpus/ietf/BFD-STD-MIB",
///       "line": 14, "column": 6, "end_line": 14, "end_column": 26,
///       "message": "...", "hint": "..." }
///   ]
/// }
/// ```
///
/// `module`, `file`, the positions and `hint` are `null` when not known. "errors"
/// counts fatal, severe and error; "warnings" everything else.
fn write_json(
    findings: &[Finding],
    modules: &BTreeSet<String>,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let errors = findings.iter().filter(|f| f.severity.is_error()).count();
    let diagnostics: Vec<_> = findings
        .iter()
        .map(|f| {
            let (line, column, end_line, end_column) = match f.position {
                Some((l, c, el, ec)) => (Some(l), Some(c), Some(el), Some(ec)),
                None => (None, None, None, None),
            };
            json!({
                "severity": f.severity.as_str(),
                "code": f.code,
                "module": f.module,
                "file": f.file.as_ref().map(|file| file.display().to_string()),
                "line": line,
                "column": column,
                "end_line": end_line,
                "end_column": end_column,
                "message": f.message,
                "hint": f.hint,
            })
        })
        .collect();
    let document = json!({
        "modules": modules,
        "errors": errors,
        "warnings": findings.len() - errors,
        "diagnostics": diagnostics,
    });
    serde_json::to_writer_pretty(&mut *out, &document)?;
    writeln!(out)
}

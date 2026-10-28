//! "Why this symbol?": how a name is resolved, and why it fails to resolve.
//!
//! [`MibSet::trace`](crate::MibSet::trace) runs a name through the same rules
//! the loader used (local definitions, IMPORTS, then the fallbacks the
//! strictness allows) and records each step, so the user can see which
//! definition was picked, or what was tried and why nothing matched.

use std::fmt;
use std::path::PathBuf;

use mib_rs::mib as m;
use mib_rs::types::ResolutionDomain;

use crate::convert;
use crate::library::Strictness;
use crate::oid::Oid;

/// What kind of definition a lookup is for. Names starting with an
/// uppercase letter are looked up as types, all others as values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LookupKind {
    /// A type, as named in SYNTAX.
    Type,
    /// Anything with an OID: an object, a node, a notification, a group...
    Value,
}

impl LookupKind {
    /// The kind of lookup for a name.
    pub fn for_name(name: &str) -> LookupKind {
        if name.starts_with(|c: char| c.is_ascii_uppercase()) {
            LookupKind::Type
        } else {
            LookupKind::Value
        }
    }

    fn domain(self) -> ResolutionDomain {
        match self {
            LookupKind::Type => ResolutionDomain::Type,
            LookupKind::Value => ResolutionDomain::Oid,
        }
    }
}

/// How a resolved name was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedBy {
    /// The module defines the name itself.
    Local,
    /// The module imports it from the module that defines it.
    Import,
    /// The module imports it from a module that imports it from elsewhere.
    ForwardedImport,
    /// Part of an import group was found directly and part forwarded.
    PartialImport,
    /// The import names an old or alternative module name (such as
    /// `RFC1213-MIB` for `SNMPv2-MIB`), which was mapped to the real one.
    ModuleAlias,
    /// A well-known name (`iso`, `Integer32`, ...) that always comes from
    /// its SMI module.
    BuiltIn,
    /// Not imported, but found in a standard SMI module, which the
    /// strictness allows.
    FoundationFallback,
    /// Not imported, but found by searching every loaded module, which only
    /// permissive strictness allows.
    GlobalFallback,
    /// No module was given, and only one module defines the name.
    OnlyDefinition,
}

impl ResolvedBy {
    /// A short explanation, such as "imported FROM its defining module".
    pub fn explanation(self) -> &'static str {
        match self {
            ResolvedBy::Local => "defined in the module itself",
            ResolvedBy::Import => "imported from the module that defines it",
            ResolvedBy::ForwardedImport => {
                "imported through a module that imports it from elsewhere"
            }
            ResolvedBy::PartialImport => "imported, partly through another module",
            ResolvedBy::ModuleAlias => {
                "imported under an old or alternative module name, which was mapped to the real module"
            }
            ResolvedBy::BuiltIn => "a well-known SMI name, always taken from its SMI module",
            ResolvedBy::FoundationFallback => {
                "not imported; found in a standard SMI module, which this strictness allows"
            }
            ResolvedBy::GlobalFallback => {
                "not imported; found by searching every loaded module (permissive strictness only)"
            }
            ResolvedBy::OnlyDefinition => "the only loaded definition with this name",
        }
    }

    fn from_strategy(strategy: m::ResolutionStrategy) -> Self {
        use m::ResolutionStrategy as S;
        match strategy {
            S::Local => ResolvedBy::Local,
            S::DirectImport => ResolvedBy::Import,
            S::ForwardedImport => ResolvedBy::ForwardedImport,
            S::PartialImport => ResolvedBy::PartialImport,
            S::AliasImport => ResolvedBy::ModuleAlias,
            S::IntrinsicFallback => ResolvedBy::BuiltIn,
            S::ConstrainedFallback => ResolvedBy::FoundationFallback,
            S::GlobalFallback => ResolvedBy::GlobalFallback,
            S::UniqueCandidate => ResolvedBy::OnlyDefinition,
        }
    }
}

/// The result of a trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceOutcome {
    /// The name resolves to the definition in `module`.
    Resolved {
        /// The module whose definition is used.
        module: String,
        /// How it was found.
        by: ResolvedBy,
    },
    /// No module was given and several modules define the name.
    Ambiguous,
    /// Nothing matched.
    Missing,
}

/// A loaded definition with the traced name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceDefinition {
    /// The module that defines it.
    pub module: String,
    /// What it is: "object", "type", "node", "notification", ...
    pub kind: String,
    /// The file the module came from.
    pub file: Option<PathBuf>,
    /// Its OID, for definitions that have one.
    pub oid: Option<Oid>,
    /// True if this kind of definition fits the lookup (a type for a type
    /// lookup, anything with an OID for a value lookup).
    pub applicable: bool,
    /// True if this is the definition the lookup picked.
    pub selected: bool,
}

/// What the module's IMPORTS say about the traced name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceImport {
    /// The module written after FROM.
    pub declared_module: String,
    /// The module the import was resolved to, if it was resolved.
    pub resolved_module: Option<String>,
    /// True if a module named [`declared_module`](Self::declared_module) is
    /// loaded (whether or not it defines the name).
    pub declared_module_loaded: bool,
    /// Modules the import was forwarded through, in order.
    pub via: Vec<String>,
}

/// A reference to the traced name that could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedUse {
    /// The module with the reference.
    pub module: String,
    /// What kind of reference: "import", "type", "oid", "index", ...
    pub kind: String,
    /// Why it failed.
    pub reason: String,
    /// How many references of this kind the module has.
    pub count: usize,
}

/// The explanation of one lookup, from [`MibSet::trace`](crate::MibSet::trace).
///
/// Its [`Display`](fmt::Display) form is a readable report, as printed by
/// `ferosnmp mib why`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolTrace {
    /// The name looked up, without a module.
    pub symbol: String,
    /// The module the name was looked up from, if one was given.
    pub scope: Option<String>,
    /// Whether a type or a value was looked up.
    pub lookup: LookupKind,
    /// The strictness the set was loaded with, which decides the fallbacks.
    pub strictness: Strictness,
    /// The result.
    pub outcome: TraceOutcome,
    /// Every loaded definition with this name.
    pub definitions: Vec<TraceDefinition>,
    /// What the scope module's IMPORTS say about the name.
    pub import: Option<TraceImport>,
    /// Whether a not-imported name may be taken from a standard SMI module.
    pub foundation_fallback: bool,
    /// Whether a not-imported name may be found in any loaded module.
    pub global_fallback: bool,
    /// References to the name, anywhere, that could not be resolved.
    pub unresolved: Vec<UnresolvedUse>,
}

/// Why a name could not be traced at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TraceError {
    /// Nothing was asked.
    #[error("no symbol given")]
    Empty,
    /// The query is not `name` or `MODULE::name`.
    #[error("{0:?} is not a name or MODULE::name")]
    Invalid(String),
    /// The module named in `MODULE::name` is not loaded.
    #[error("module {0} is not loaded")]
    ModuleNotFound(String),
    /// Several files define the module named in `MODULE::name`.
    #[error("module {0} is loaded from more than one file")]
    AmbiguousModule(String),
}

pub(crate) fn trace(
    mib: &mib_rs::Mib,
    strictness: Strictness,
    query: &str,
) -> Result<SymbolTrace, TraceError> {
    let query = query.trim();
    let symbol = query.rsplit_once("::").map_or(query, |(_, name)| name);
    let lookup = LookupKind::for_name(symbol);
    let trace = mib
        .trace_symbol(query, None, lookup.domain())
        .map_err(|error| match error {
            m::ResolutionTraceError::EmptyQuery => TraceError::Empty,
            m::ResolutionTraceError::InvalidQualifiedQuery(query) => TraceError::Invalid(query),
            m::ResolutionTraceError::ModuleNotFound(module) => TraceError::ModuleNotFound(module),
            m::ResolutionTraceError::AmbiguousModuleScope { module, .. } => {
                TraceError::AmbiguousModule(module)
            }
            // We never pass a separate scope, so the two cannot conflict.
            m::ResolutionTraceError::ConflictingScope { query_scope, .. } => {
                TraceError::Invalid(query_scope)
            }
        })?;

    let module_name = |id: m::ModuleId| mib.module_by_id(id).name().to_string();
    let selected = trace.target.as_ref().map(|target| &target.candidate);
    let definitions = trace
        .candidates
        .iter()
        .map(|candidate| {
            let module = mib.module_by_id(candidate.module);
            TraceDefinition {
                module: candidate.module_name.clone(),
                kind: candidate.kind.to_string(),
                file: module
                    .source()
                    .map(|source| convert::source_file(source.origin(), source.label())),
                oid: candidate.oid.as_ref().map(convert::oid),
                applicable: candidate.applicable,
                selected: Some(candidate) == selected,
            }
        })
        .collect();
    let outcome = match (&trace.outcome, &trace.target) {
        (m::ResolutionOutcome::Resolved, Some(target)) => TraceOutcome::Resolved {
            module: target.candidate.module_name.clone(),
            by: ResolvedBy::from_strategy(target.strategy),
        },
        (m::ResolutionOutcome::Ambiguous, _) => TraceOutcome::Ambiguous,
        _ => TraceOutcome::Missing,
    };
    let import = trace.import.as_ref().map(|import| TraceImport {
        declared_module: import.declared_module.clone(),
        resolved_module: import.target.map(module_name),
        declared_module_loaded: mib.module(&import.declared_module).is_some(),
        via: import
            .selected_path
            .iter()
            .copied()
            .map(module_name)
            .filter(|name| Some(name) != import.target.map(module_name).as_ref())
            .collect(),
    });
    Ok(SymbolTrace {
        symbol: trace.symbol.clone(),
        scope: trace.scope.as_ref().map(|scope| scope.module_name.clone()),
        lookup,
        strictness,
        outcome,
        definitions,
        import,
        foundation_fallback: trace.fallbacks.constrained || trace.fallbacks.intrinsic,
        global_fallback: trace.fallbacks.global,
        unresolved: unresolved_uses(&trace.unresolved),
    })
}

/// The unresolved references, one entry per module, kind and reason.
fn unresolved_uses(references: &[m::UnresolvedRef]) -> Vec<UnresolvedUse> {
    let mut uses: Vec<UnresolvedUse> = Vec::new();
    for reference in references {
        let kind = reference.kind.to_string();
        let reason = reason_text(&reference.reason);
        match uses.iter_mut().find(|known| {
            known.module == reference.module && known.kind == kind && known.reason == reason
        }) {
            Some(known) => known.count += 1,
            None => uses.push(UnresolvedUse {
                module: reference.module.clone(),
                kind,
                reason,
                count: 1,
            }),
        }
    }
    uses
}

/// mib-rs's reason codes (`module_not_found`, ...) in words.
fn reason_text(reason: &str) -> String {
    match reason {
        "module_not_found" => "the module it is imported from was not found",
        "symbol_not_exported" => "the module it is imported from does not define it",
        "unknown_type" => "no such type is defined or imported",
        "unknown_parent" => "the parent it hangs off is not defined or imported",
        "unknown_index_object" => "the INDEX object is not defined or imported",
        "unknown_object" => "the object is not defined or imported",
        "dependency_cycle" => "the modules import each other in a circle",
        other => return other.replace('_', " "),
    }
    .to_string()
}

impl fmt::Display for SymbolTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.lookup {
            LookupKind::Type => "type",
            LookupKind::Value => "value",
        };
        let strictness = match self.strictness {
            Strictness::Permissive => "permissive",
            Strictness::Normal => "normal",
            Strictness::Strict => "strict",
        };
        match &self.scope {
            Some(scope) => writeln!(
                f,
                "{} as used in {scope} ({what} lookup, {strictness} strictness)",
                self.symbol
            )?,
            None => writeln!(
                f,
                "{} ({what} lookup in all modules, {strictness} strictness)",
                self.symbol
            )?,
        }
        writeln!(f)?;

        match &self.outcome {
            TraceOutcome::Resolved { module, by } => {
                writeln!(
                    f,
                    "Resolves to {module}::{}: {}.",
                    self.symbol,
                    by.explanation()
                )?;
            }
            TraceOutcome::Ambiguous => {
                writeln!(
                    f,
                    "Ambiguous: several modules define {}. Ask for MODULE::{} to \
                     see which one a module uses.",
                    self.symbol, self.symbol
                )?;
            }
            TraceOutcome::Missing => writeln!(f, "Does not resolve.")?,
        }

        if let Some(scope) = &self.scope {
            match &self.import {
                Some(import) => {
                    write!(f, "IMPORTS in {scope}: FROM {}", import.declared_module)?;
                    match &import.resolved_module {
                        Some(module) if *module == import.declared_module => writeln!(f)?,
                        Some(module) => {
                            write!(f, ", resolved to {module}")?;
                            if !import.via.is_empty() {
                                write!(f, " through {}", import.via.join(", "))?;
                            }
                            writeln!(f)?;
                        }
                        None if import.declared_module_loaded => writeln!(
                            f,
                            ", but {} does not define {}",
                            import.declared_module, self.symbol
                        )?,
                        None => {
                            writeln!(f, ", but no module {} was found", import.declared_module)?
                        }
                    }
                }
                None if matches!(
                    self.outcome,
                    TraceOutcome::Resolved {
                        by: ResolvedBy::Local,
                        ..
                    }
                ) => {}
                None => writeln!(f, "IMPORTS in {scope}: {} is not imported", self.symbol)?,
            }
            let mut fallbacks = Vec::new();
            if self.foundation_fallback {
                fallbacks.push("standard SMI modules");
            }
            if self.global_fallback {
                fallbacks.push("every loaded module");
            }
            if fallbacks.is_empty() {
                writeln!(
                    f,
                    "Fallbacks: none; at {strictness} strictness a name must be defined or imported"
                )?;
            } else {
                writeln!(
                    f,
                    "Fallbacks for names that are not imported: {}",
                    fallbacks.join(", ")
                )?;
            }
        }

        writeln!(f)?;
        if self.definitions.is_empty() {
            writeln!(f, "No loaded module defines {}.", self.symbol)?;
        } else {
            writeln!(f, "Definitions:")?;
            for definition in &self.definitions {
                let marker = if definition.selected { "*" } else { " " };
                write!(
                    f,
                    "  {marker} {}::{}  {}",
                    definition.module, self.symbol, definition.kind
                )?;
                if let Some(oid) = &definition.oid {
                    write!(f, "  {oid}")?;
                }
                if !definition.applicable {
                    write!(f, "  (not a {what})")?;
                }
                if let Some(file) = &definition.file {
                    write!(f, "  {}", file.display())?;
                }
                writeln!(f)?;
            }
        }

        // For a lookup from one module, its own failed references matter;
        // the others are only counted.
        let (shown, others): (Vec<&UnresolvedUse>, Vec<&UnresolvedUse>) =
            self.unresolved.iter().partition(|reference| {
                self.scope
                    .as_ref()
                    .is_none_or(|scope| *scope == reference.module)
            });
        if !shown.is_empty() {
            writeln!(f)?;
            writeln!(f, "Unresolved references to {}:", self.symbol)?;
            for reference in shown {
                write!(f, "    {} ({}", reference.module, reference.kind)?;
                if reference.count > 1 {
                    write!(f, ", {} times", reference.count)?;
                }
                writeln!(f, "): {}", reference.reason)?;
            }
        }
        if !others.is_empty() {
            let modules: std::collections::BTreeSet<&str> = others
                .iter()
                .map(|reference| reference.module.as_str())
                .collect();
            writeln!(f)?;
            let count: usize = others.iter().map(|reference| reference.count).sum();
            writeln!(
                f,
                "{count} more unresolved {} to {} in {} other {}.",
                if count == 1 {
                    "reference"
                } else {
                    "references"
                },
                self.symbol,
                modules.len(),
                if modules.len() == 1 {
                    "module"
                } else {
                    "modules"
                }
            )?;
        }
        Ok(())
    }
}

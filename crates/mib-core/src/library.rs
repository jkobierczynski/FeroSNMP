//! Where MIB modules come from, and how strictly to read them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::convert;
use crate::error::LoadError;
use crate::set::MibSet;

/// How forgiving the loader is with modules that break the SMI rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Strictness {
    /// Load as much as possible. When an import cannot be found where the
    /// module says, look for the symbol in every loaded module. This is the
    /// default, so imperfect vendor MIBs still load and browse.
    #[default]
    Permissive,
    /// Allow only the common, well-understood fallbacks, such as known module
    /// renames and SMI types used without importing them.
    Normal,
    /// No guessing, and report style and informational diagnostics too.
    /// Meant for MIB authors checking their own modules (`ferosnmp lint`).
    Strict,
}

impl Strictness {
    fn resolver(self) -> mib_rs::ResolverStrictness {
        match self {
            Strictness::Permissive => mib_rs::ResolverStrictness::Permissive,
            Strictness::Normal => mib_rs::ResolverStrictness::Normal,
            Strictness::Strict => mib_rs::ResolverStrictness::Strict,
        }
    }

    fn diagnostic_config(self) -> mib_rs::DiagnosticConfig {
        let reporting = match self {
            Strictness::Permissive | Strictness::Normal => mib_rs::ReportingLevel::Default,
            Strictness::Strict => mib_rs::ReportingLevel::Verbose,
        };
        mib_rs::DiagnosticConfig {
            reporting,
            // Never give up on errors in the files: collect them as
            // diagnostics and return the set. Only a fatal problem stops a load.
            fail_at: mib_rs::Severity::Fatal,
            ..mib_rs::DiagnosticConfig::default()
        }
    }
}

/// Which kind of folder a module file was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// A folder added with [`MibLibrary::with_folder`].
    UserFolder,
    /// A folder net-snmp or libsmi is configured to use on this machine.
    SystemFolder,
}

/// A module found in a file, from [`MibLibrary::index`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFile {
    /// Module name, read from the `DEFINITIONS` line inside the file. It
    /// often differs from the file name in vendor MIBs.
    pub module: String,
    /// The file.
    pub file: PathBuf,
    /// The kind of folder the file is in.
    pub source: SourceKind,
    /// True if an earlier file defines the same module, so this one is not
    /// used.
    pub shadowed: bool,
}

/// The set of places MIB modules are loaded from, and how strictly.
///
/// Modules are looked up in three kinds of source, in this order; when a
/// module is in more than one, the first one wins:
///
/// 1. folders added with [`with_folder`](Self::with_folder), in the order added;
/// 2. the folders net-snmp and libsmi use on this machine (see
///    [`system_folders`](Self::system_folders)), unless turned off with
///    [`with_system_folders(false)`](Self::with_system_folders);
/// 3. the foundation modules built into this crate (`SNMPv2-SMI`,
///    `SNMPv2-TC`, `SNMPv2-CONF`, `RFC1155-SMI` and similar).
///
/// Folders are searched recursively. Files are matched on their content, not
/// their name: a file is used for a module if its `DEFINITIONS` line names
/// that module, whatever the file is called. Files without an extension or
/// ending in `.mib`, `.smi`, `.txt` or `.my` are considered.
///
/// ```no_run
/// use mib_core::MibLibrary;
///
/// let set = MibLibrary::new()
///     .with_folder(r"C:\MIBs\cisco")
///     .load()?;
/// println!("{} diagnostics", set.diagnostics().len());
/// # Ok::<(), mib_core::LoadError>(())
/// ```
#[derive(Debug, Clone)]
pub struct MibLibrary {
    folders: Vec<PathBuf>,
    use_system_folders: bool,
    strictness: Strictness,
}

impl Default for MibLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl MibLibrary {
    /// A library with no folders of its own, using the system folders,
    /// permissive.
    pub fn new() -> Self {
        MibLibrary {
            folders: Vec::new(),
            use_system_folders: true,
            strictness: Strictness::default(),
        }
    }

    /// Add a folder to search, after the folders added before it. A single
    /// MIB file may be given instead of a folder; it is used whatever its
    /// extension.
    pub fn with_folder(mut self, folder: impl Into<PathBuf>) -> Self {
        self.folders.push(folder.into());
        self
    }

    /// Turn the net-snmp and libsmi system folders on or off.
    pub fn with_system_folders(mut self, enabled: bool) -> Self {
        self.use_system_folders = enabled;
        self
    }

    /// Set how strictly modules are read.
    pub fn with_strictness(mut self, strictness: Strictness) -> Self {
        self.strictness = strictness;
        self
    }

    /// The folders added with [`with_folder`](Self::with_folder).
    pub fn folders(&self) -> &[PathBuf] {
        &self.folders
    }

    /// The strictness modules are read with.
    pub fn strictness(&self) -> Strictness {
        self.strictness
    }

    /// The folders net-snmp and libsmi are configured to use on this machine
    /// that exist: their default folders, `mibdirs` in `snmp.conf`, `path`
    /// in `smi.conf`, and the `MIBDIRS` and `SMIPATH` environment variables.
    pub fn system_folders() -> Vec<PathBuf> {
        mib_rs::searchpath::discover_system_paths()
            .into_iter()
            .map(PathBuf::from)
            .collect()
    }

    /// Every folder searched, in search order (the built-in modules come
    /// after these).
    pub fn search_folders(&self) -> Vec<(PathBuf, SourceKind)> {
        let mut folders: Vec<(PathBuf, SourceKind)> = self
            .folders
            .iter()
            .map(|folder| (folder.clone(), SourceKind::UserFolder))
            .collect();
        if self.use_system_folders {
            folders.extend(
                Self::system_folders()
                    .into_iter()
                    .map(|folder| (folder, SourceKind::SystemFolder)),
            );
        }
        folders
    }

    /// List the modules in every folder, by the name inside each file.
    ///
    /// Files are listed in search order; within a folder, by module name. A
    /// file that defines several modules is listed once per module.
    pub fn index(&self) -> Result<Vec<ModuleFile>, LoadError> {
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        for (folder, kind) in self.search_folders() {
            let Some(source) = open_folder(&folder, kind)? else {
                continue;
            };
            for module in source.list_modules().map_err(LoadError::Io)? {
                for candidate in source.find_candidates(&module) {
                    let candidate = candidate.map_err(LoadError::Io)?;
                    let mib_rs::SourceOrigin::File { path } = candidate.origin() else {
                        continue;
                    };
                    files.push(ModuleFile {
                        shadowed: !seen.insert(module.clone()),
                        module: module.clone(),
                        file: path.clone(),
                        source: kind,
                    });
                }
            }
        }
        Ok(files)
    }

    /// Load every module in the library.
    pub fn load(&self) -> Result<MibSet, LoadError> {
        self.load_with(None)
    }

    /// Load only the named modules and the modules they import.
    pub fn load_modules(&self, modules: &[&str]) -> Result<MibSet, LoadError> {
        self.load_with(Some(modules))
    }

    fn load_with(&self, modules: Option<&[&str]>) -> Result<MibSet, LoadError> {
        let mut loader = mib_rs::Loader::new()
            .resolver_strictness(self.strictness.resolver())
            .diagnostic_config(self.strictness.diagnostic_config());
        let mut has_sources = false;
        for (folder, kind) in self.search_folders() {
            if let Some(source) = open_folder(&folder, kind)? {
                loader = loader.source(source);
                has_sources = true;
            }
        }
        if !has_sources {
            // mib-rs refuses to load without any source at all; an empty one
            // still gives us the built-in foundation modules.
            loader = loader.source(mib_rs::source::memory_modules(
                Vec::<(String, Vec<u8>)>::new(),
            ));
        }
        if let Some(modules) = modules {
            loader = loader.modules(modules.iter().copied());
        }
        let mib = loader.load().map_err(load_error)?;
        let folders: Vec<PathBuf> = self
            .search_folders()
            .into_iter()
            .map(|(folder, _)| folder)
            .collect();
        Ok(MibSet::new(mib, self.strictness, &folders))
    }
}

/// Load a single module from text in memory, plus the built-in foundation
/// modules it may import. `module` must match the name on the `DEFINITIONS`
/// line of the text.
///
/// Meant for tests, fuzzing and checking a module before it is saved.
pub fn load_module_text(
    module: &str,
    text: impl Into<Vec<u8>>,
    strictness: Strictness,
) -> Result<MibSet, LoadError> {
    let mib = mib_rs::Loader::new()
        .source(mib_rs::source::memory(module, text))
        .resolver_strictness(strictness.resolver())
        .diagnostic_config(strictness.diagnostic_config())
        .load()
        .map_err(load_error)?;
    Ok(MibSet::new(mib, strictness, &[]))
}

/// Open a folder (or a single file) as an mib-rs source. A user folder that
/// cannot be read is an error; a system folder that cannot be read is skipped
/// (`Ok(None)`), as the user did not ask for it.
fn open_folder(
    folder: &Path,
    kind: SourceKind,
) -> Result<Option<Box<dyn mib_rs::Source>>, LoadError> {
    let source = if folder.is_file() {
        mib_rs::source::file(folder)
    } else {
        mib_rs::source::dir(folder)
    };
    match (source, kind) {
        (Ok(source), _) => Ok(Some(source)),
        (Err(_), SourceKind::SystemFolder) => Ok(None),
        (Err(source), SourceKind::UserFolder) => Err(LoadError::Folder {
            path: folder.to_path_buf(),
            source,
        }),
    }
}

fn load_error(error: mib_rs::LoadError) -> LoadError {
    match error {
        mib_rs::LoadError::MissingModules(modules) => LoadError::ModulesNotFound(modules),
        mib_rs::LoadError::DiagnosticThreshold { report } => {
            LoadError::Fatal(convert::diagnostics(&report))
        }
        mib_rs::LoadError::Io(error) => LoadError::Io(error),
        mib_rs::LoadError::Source(error) => LoadError::Io(std::io::Error::other(error)),
        // We always pass at least one source, so this cannot happen.
        mib_rs::LoadError::NoSources => {
            LoadError::Io(std::io::Error::other("no MIB sources configured"))
        }
    }
}

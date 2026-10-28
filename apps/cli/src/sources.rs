//! Where the `mib` and `lint` commands load MIB files from.

use std::env;
use std::error::Error;
use std::path::PathBuf;

use clap::Args;
use mib_core::{LoadError, MibLibrary, MibSet};

use crate::CliError;

/// Environment variable with extra MIB folders, separated like `PATH`
/// (`;` on Windows, `:` elsewhere). Searched after the `-M` folders.
pub const MIBDIRS_VAR: &str = "FEROSNMP_MIBDIRS";

/// Command-line options that choose the MIB folders.
#[derive(Debug, Clone, Args)]
pub struct MibSources {
    /// Folder with MIB files to load; repeat for more folders. Searched before
    /// the folders in FEROSNMP_MIBDIRS and the net-snmp and libsmi folders.
    #[arg(short = 'M', long = "mib-dir", value_name = "DIR", global = true)]
    pub mib_dirs: Vec<PathBuf>,

    /// Do not use the net-snmp and libsmi MIB folders on this machine, nor
    /// FEROSNMP_MIBDIRS.
    #[arg(long, global = true)]
    pub no_system_mibs: bool,
}

impl MibSources {
    /// A library with `extra` folders first, then the `-M` folders, then the
    /// environment and system folders (unless `--no-system-mibs`).
    pub fn library(&self, extra: &[PathBuf]) -> MibLibrary {
        let mut library = MibLibrary::new().with_system_folders(!self.no_system_mibs);
        for folder in extra.iter().chain(&self.mib_dirs) {
            library = library.with_folder(folder);
        }
        if !self.no_system_mibs
            && let Some(value) = env::var_os(MIBDIRS_VAR)
        {
            for folder in env::split_paths(&value).filter(|f| !f.as_os_str().is_empty()) {
                // A folder from the environment that does not exist is
                // skipped rather than failing every command.
                if folder.is_dir() {
                    library = library.with_folder(folder);
                }
            }
        }
        library
    }

    /// Load every module in the chosen folders.
    pub fn load(&self) -> Result<MibSet, CliError> {
        self.library(&[]).load().map_err(load_error)
    }
}

/// Turn a [`LoadError`] into a message that includes its cause, such as
/// "cannot read MIB folder C:\MIBs: The system cannot find the file specified."
pub fn load_error(error: LoadError) -> CliError {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    if let LoadError::Fatal(diagnostics) = &error {
        for diagnostic in diagnostics {
            message.push_str("\n  ");
            message.push_str(&diagnostic.to_string());
        }
    }
    CliError::Message(message)
}

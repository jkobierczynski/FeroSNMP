//! Errors that stop a MIB library from loading.

use std::io;
use std::path::PathBuf;

use crate::diagnostic::Diagnostic;

/// Why [`MibLibrary::load`](crate::MibLibrary::load) returned no
/// [`MibSet`](crate::MibSet).
///
/// Problems inside MIB files are normally not errors: they become
/// [`Diagnostic`]s on the loaded set. Only these cases stop a load.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// A folder added with [`MibLibrary::with_folder`](crate::MibLibrary::with_folder)
    /// does not exist or cannot be read.
    #[error("cannot read MIB folder {}", path.display())]
    Folder {
        /// The folder.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: io::Error,
    },

    /// Modules asked for by name were not found in any source.
    #[error("modules not found: {}", .0.join(", "))]
    ModulesNotFound(Vec<String>),

    /// A problem so serious that no usable set could be built. Every
    /// diagnostic collected up to that point is included.
    #[error("loading failed with a fatal error")]
    Fatal(Vec<Diagnostic>),

    /// Reading a MIB file failed.
    #[error("cannot read MIB file")]
    Io(#[source] io::Error),
}

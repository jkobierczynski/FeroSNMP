//! The GUI's settings, kept in a TOML file.
//!
//! On Windows the file is `%APPDATA%\FeroSNMP\settings.toml`, for example
//! `C:\Users\Jurgen\AppData\Roaming\FeroSNMP\settings.toml`. It looks like:
//!
//! ```toml
//! mib-folders = ['C:\MIBs\cisco', 'C:\MIBs\juniper']
//! use-system-folders = true
//! strictness = "permissive"
//! agent = "lab switch"
//!
//! [[targets]]
//! name = "lab switch"
//! host = "192.0.2.10"
//! ```
//!
//! See [`crate::targets`] for the fields of a saved agent.
//!
//! Settings the file does not mention keep their defaults, so older files keep
//! working when settings are added.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use mib_core::{MibLibrary, Strictness};
use serde::{Deserialize, Serialize};

use crate::targets::SavedTarget;

/// Everything the user can set in the Settings dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Settings {
    /// Folders with MIB files, searched in this order.
    pub mib_folders: Vec<PathBuf>,
    /// Also search the folders net-snmp and libsmi use on this computer.
    pub use_system_folders: bool,
    /// How strictly MIB files are read.
    pub strictness: StrictnessSetting,
    /// What was in the Agent box last time: a saved agent's name or an
    /// address.
    pub agent: String,
    /// The saved agents, in the order the Agent box lists them. Written
    /// last, because TOML tables must follow the plain values.
    pub targets: Vec<SavedTarget>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            mib_folders: Vec::new(),
            use_system_folders: true,
            strictness: StrictnessSetting::Permissive,
            agent: String::new(),
            targets: Vec::new(),
        }
    }
}

/// [`Strictness`] as written in the settings file.
///
/// mib-core does not depend on serde, so the GUI has its own copy of the enum
/// with the names used in the file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrictnessSetting {
    /// See [`Strictness::Permissive`].
    #[default]
    Permissive,
    /// See [`Strictness::Normal`].
    Normal,
    /// See [`Strictness::Strict`].
    Strict,
}

impl StrictnessSetting {
    /// Every value, in the order shown in the Settings dialog.
    pub const ALL: [StrictnessSetting; 3] = [
        StrictnessSetting::Permissive,
        StrictnessSetting::Normal,
        StrictnessSetting::Strict,
    ];

    /// The name shown in the Settings dialog.
    pub fn label(self) -> &'static str {
        match self {
            StrictnessSetting::Permissive => "Permissive",
            StrictnessSetting::Normal => "Normal",
            StrictnessSetting::Strict => "Strict",
        }
    }

    /// One sentence on what the setting does, for the Settings dialog.
    pub fn explanation(self) -> &'static str {
        match self {
            StrictnessSetting::Permissive => {
                "Load as much as possible, and look for missing imports in every \
                 loaded module. Best for browsing vendor MIBs."
            }
            StrictnessSetting::Normal => {
                "Allow only common, well-understood fixes, such as known module renames."
            }
            StrictnessSetting::Strict => {
                "No guessing, and report style problems too. For checking MIBs you write."
            }
        }
    }
}

impl From<StrictnessSetting> for Strictness {
    fn from(setting: StrictnessSetting) -> Self {
        match setting {
            StrictnessSetting::Permissive => Strictness::Permissive,
            StrictnessSetting::Normal => Strictness::Normal,
            StrictnessSetting::Strict => Strictness::Strict,
        }
    }
}

/// Why the settings file could not be read or written.
#[derive(Debug)]
pub enum SettingsError {
    /// The file or its folder could not be read or written.
    Io(PathBuf, io::Error),
    /// The file is not valid TOML, or has a value of the wrong type.
    Parse(PathBuf, toml::de::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Io(path, error) => write!(f, "{}: {error}", path.display()),
            SettingsError::Parse(path, error) => write!(f, "{}: {error}", path.display()),
        }
    }
}

impl std::error::Error for SettingsError {}

impl Settings {
    /// Where the settings file lives: `%APPDATA%\FeroSNMP\settings.toml`, or
    /// `None` if `APPDATA` is not set.
    pub fn default_path() -> Option<PathBuf> {
        let app_data = std::env::var_os("APPDATA")?;
        Some(
            PathBuf::from(app_data)
                .join("FeroSNMP")
                .join("settings.toml"),
        )
    }

    /// Read settings from `path`. A file that does not exist yet gives the
    /// defaults.
    pub fn load(path: &Path) -> Result<Settings, SettingsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|error| SettingsError::Parse(path.into(), error))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(error) => Err(SettingsError::Io(path.into(), error)),
        }
    }

    /// Write settings to `path`, creating its folder if needed.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let io_error = |error| SettingsError::Io(path.into(), error);
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(io_error)?;
        }
        // Serializing this struct cannot fail: every field is a plain value.
        let text = toml::to_string_pretty(self).expect("settings are always valid TOML");
        // Write to a temporary file and rename it over the old one, so a crash
        // half-way never leaves a truncated settings file.
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, text).map_err(io_error)?;
        std::fs::rename(&temporary, path).map_err(io_error)
    }

    /// The MIB library these settings describe, with `extra` folders or
    /// files searched first: the ones dropped on the window, which are kept
    /// for this session only.
    pub fn library_with(&self, extra: &[PathBuf]) -> MibLibrary {
        extra.iter().chain(&self.mib_folders).fold(
            MibLibrary::new()
                .with_system_folders(self.use_system_folders)
                .with_strictness(self.strictness.into()),
            |library, folder| library.with_folder(folder),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ferosnmp-gui-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = temp_dir("missing");
        let settings = Settings::load(&dir.join("settings.toml")).unwrap();
        assert_eq!(settings, Settings::default());
        assert!(settings.use_system_folders);
    }

    #[test]
    fn saves_and_loads_again() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("FeroSNMP").join("settings.toml");
        let settings = Settings {
            mib_folders: vec![
                PathBuf::from(r"C:\MIBs\cisco"),
                PathBuf::from("/usr/share/snmp/mibs"),
            ],
            use_system_folders: false,
            strictness: StrictnessSetting::Strict,
            agent: "lab".into(),
            targets: vec![SavedTarget {
                name: "lab".into(),
                host: "192.0.2.10".into(),
                ..SavedTarget::default()
            }],
        };
        settings.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("strictness = \"strict\""), "{text}");
        assert_eq!(Settings::load(&path).unwrap(), settings);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_and_missing_keys_are_fine() {
        let settings: Settings =
            toml::from_str("strictness = \"normal\"\nfuture-setting = 3\n").unwrap();
        assert_eq!(settings.strictness, StrictnessSetting::Normal);
        assert!(settings.mib_folders.is_empty());
    }

    #[test]
    fn bad_values_are_reported_with_the_file() {
        let dir = temp_dir("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.toml");
        std::fs::write(&path, "strictness = \"very\"\n").unwrap();
        let error = Settings::load(&path).unwrap_err().to_string();
        assert!(error.contains("settings.toml"), "{error}");
        assert!(error.contains("very"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

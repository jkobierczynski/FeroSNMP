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
//! See [`crate::targets`] for the fields of a saved agent, and
//! [`crate::trap_users`] for the `[[trap-users]]`. The trap receiver and the
//! look of the window have settings of their own:
//!
//! ```toml
//! trap-port = 162
//! trap-address = "0.0.0.0"
//! listen-at-start = false
//! theme = "system"
//!
//! [layout]
//! x = 120
//! y = 80
//! width = 1100
//! height = 720
//! maximized = false
//! tree-width = 380
//! details-height = 280
//! bottom-height = 250
//! tab = "traps"
//! ```
//!
//! The `[layout]` is written when the window closes.
//!
//! Settings the file does not mention keep their defaults, so older files keep
//! working when settings are added.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use mib_core::{MibLibrary, Strictness};
use serde::{Deserialize, Serialize};

use crate::targets::SavedTarget;
use crate::trap_users::SavedTrapUser;

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
    /// The UDP port the Traps tab listens on.
    pub trap_port: u16,
    /// The local address the Traps tab listens on: `0.0.0.0` for every
    /// IPv4 address, `::` for every IPv6 address.
    pub trap_address: String,
    /// Start listening for traps when the window opens.
    pub listen_at_start: bool,
    /// Light or dark tree and lists.
    pub theme: ThemeSetting,
    /// Where the window and its panes were when it was last closed. TOML
    /// tables must follow the plain values, so this and the lists below
    /// come last.
    pub layout: Option<Layout>,
    /// The SNMPv3 users traps and INFORMs are accepted from.
    pub trap_users: Vec<SavedTrapUser>,
    /// The saved agents, in the order the Agent box lists them.
    pub targets: Vec<SavedTarget>,
}

/// Light or dark, as written in the settings file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeSetting {
    /// Follow "Choose your app mode" in the Windows settings.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl ThemeSetting {
    /// Every value, in the order shown in the Settings dialog.
    pub const ALL: [ThemeSetting; 3] = [
        ThemeSetting::System,
        ThemeSetting::Light,
        ThemeSetting::Dark,
    ];

    /// The name shown in the Settings dialog.
    pub fn label(self) -> &'static str {
        match self {
            ThemeSetting::System => "Same as Windows",
            ThemeSetting::Light => "Light",
            ThemeSetting::Dark => "Dark",
        }
    }

    /// Whether the tree and lists are dark, given whether Windows is set
    /// to dark mode.
    pub fn is_dark(self, windows_dark: bool) -> bool {
        match self {
            ThemeSetting::System => windows_dark,
            ThemeSetting::Light => false,
            ThemeSetting::Dark => true,
        }
    }
}

/// The window's position and size and the panes' sizes. The window is in
/// screen pixels (as `GetWindowPlacement` gives it); the panes are at 96
/// DPI, so they keep their size on a monitor with another scaling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Layout {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    /// Width of the MIB tree.
    pub tree_width: i32,
    /// Height of the properties list above the description.
    pub details_height: i32,
    /// Height of the tabs along the bottom.
    pub bottom_height: i32,
    /// The tab shown along the bottom: `results`, `table`, `traps`, `log`
    /// or `diagnostics`.
    pub tab: String,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            maximized: false,
            tree_width: 380,
            details_height: 280,
            bottom_height: 250,
            tab: "results".to_string(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            mib_folders: Vec::new(),
            use_system_folders: true,
            strictness: StrictnessSetting::Permissive,
            agent: String::new(),
            trap_port: trap_rx::TRAP_PORT,
            trap_address: "0.0.0.0".to_string(),
            listen_at_start: false,
            theme: ThemeSetting::System,
            layout: None,
            trap_users: Vec::new(),
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

    /// The address and port the Traps tab listens on.
    pub fn trap_listen_address(&self) -> Result<std::net::SocketAddr, String> {
        let address = self.trap_address.trim();
        let address = address
            .strip_prefix('[')
            .and_then(|a| a.strip_suffix(']'))
            .unwrap_or(address);
        let ip: std::net::IpAddr = if address.is_empty() {
            std::net::Ipv4Addr::UNSPECIFIED.into()
        } else {
            address.parse().map_err(|_| {
                format!(
                    "The trap address \u{201c}{address}\u{201d} is not an IP address. Use \
                     0.0.0.0 to listen on every IPv4 address, or one of this computer's \
                     addresses."
                )
            })?
        };
        if self.trap_port == 0 {
            return Err("The trap port must be a number from 1 to 65535.".to_string());
        }
        Ok(std::net::SocketAddr::new(ip, self.trap_port))
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
            trap_port: 1162,
            trap_address: "127.0.0.1".into(),
            listen_at_start: true,
            theme: ThemeSetting::Dark,
            layout: Some(Layout {
                x: -1200,
                maximized: true,
                tab: "traps".into(),
                ..Layout::default()
            }),
            trap_users: vec![SavedTrapUser {
                name: "trapper".into(),
                ..SavedTrapUser::default()
            }],
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
    fn trap_addresses() {
        let settings = |address: &str, port: u16| Settings {
            trap_address: address.into(),
            trap_port: port,
            ..Settings::default()
        };
        assert_eq!(
            Settings::default().trap_listen_address().unwrap(),
            "0.0.0.0:162".parse().unwrap()
        );
        assert_eq!(
            settings("", 1162).trap_listen_address().unwrap(),
            "0.0.0.0:1162".parse().unwrap()
        );
        assert_eq!(
            settings("[::]", 162).trap_listen_address().unwrap(),
            "[::]:162".parse().unwrap()
        );
        assert!(settings("trap.example", 162).trap_listen_address().is_err());
        assert!(settings("0.0.0.0", 0).trap_listen_address().is_err());
    }

    #[test]
    fn themes() {
        assert!(ThemeSetting::System.is_dark(true));
        assert!(!ThemeSetting::System.is_dark(false));
        assert!(ThemeSetting::Dark.is_dark(false));
        assert!(!ThemeSetting::Light.is_dark(true));
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

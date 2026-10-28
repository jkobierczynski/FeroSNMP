//! Noticing MIB files that were changed by another program.
//!
//! When the window becomes active again, FeroSNMP checks whether any loaded
//! file changed since it was loaded (for example, after fixing an import in
//! an editor) and reloads the MIBs. Checking a few hundred file times takes
//! a millisecond or two, so it is done on the UI thread.

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use mib_core::MibSet;

/// The modification time of every file a set was loaded from.
#[derive(Debug, Clone, Default)]
pub struct FileStamps {
    files: Vec<(PathBuf, Option<SystemTime>)>,
}

impl FileStamps {
    /// Record the files of a set as they are now.
    pub fn of(set: &MibSet) -> Self {
        let files = set
            .modules()
            .into_iter()
            .filter(|module| !module.embedded)
            .filter_map(|module| module.file)
            .map(|file| {
                let time = modified(&file);
                (file, time)
            })
            .collect();
        FileStamps { files }
    }

    /// The files whose modification time changed, or that were removed.
    pub fn changed(&self) -> Vec<PathBuf> {
        self.files
            .iter()
            .filter(|(file, time)| modified(file) != *time)
            .map(|(file, _)| file.clone())
            .collect()
    }
}

fn modified(file: &PathBuf) -> Option<SystemTime> {
    fs::metadata(file).and_then(|meta| meta.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mib_core::MibLibrary;

    #[test]
    fn notices_a_changed_file() {
        let dir = std::env::temp_dir().join(format!("ferosnmp-stamps-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("STAMP-TEST-MIB");
        let text = "STAMP-TEST-MIB DEFINITIONS ::= BEGIN\nIMPORTS enterprises FROM SNMPv2-SMI;\n\
                    stampTest OBJECT IDENTIFIER ::= { enterprises 99998 }\nEND\n";
        fs::write(&file, text).unwrap();
        let set = MibLibrary::new()
            .with_system_folders(false)
            .with_folder(&dir)
            .load()
            .unwrap();
        let stamps = FileStamps::of(&set);
        assert!(stamps.changed().is_empty());

        // Make sure the time differs even on file systems with coarse times.
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(stamps.changed(), std::slice::from_ref(&file));

        fs::remove_file(&file).unwrap();
        assert_eq!(stamps.changed(), [file]);
        let _ = fs::remove_dir_all(&dir);
    }
}

//! Loading MIBs on a background thread.
//!
//! Loading a large MIB library takes seconds. Doing it on the UI thread would
//! freeze the window ("Not responding"), so it runs on a thread of its own and
//! hands the finished [`MibSet`] back when done. The window code turns that
//! hand-over into a posted window message.

use std::thread;
use std::time::{Duration, Instant};

use mib_core::{LoadError, MibLibrary, MibSet};

/// What a background load produced.
#[derive(Debug)]
pub struct Loaded {
    /// Which load this was. When the user starts a new load before the last
    /// one finished, the window ignores results with an older number.
    pub generation: u64,
    /// The loaded set, or why loading failed.
    pub result: Result<MibSet, LoadError>,
    /// How long loading took.
    pub elapsed: Duration,
}

/// Load `library` on a new thread and call `done` there with the result.
pub fn spawn(
    library: MibLibrary,
    generation: u64,
    done: impl FnOnce(Loaded) + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name(format!("mib-load-{generation}"))
        .spawn(move || {
            let started = Instant::now();
            let result = library.load();
            done(Loaded {
                generation,
                result,
                elapsed: started.elapsed(),
            });
        })
        .expect("could not start the MIB loading thread")
}

/// A `LoadError` with all of its causes, such as
/// "cannot read MIB folder C:\MIBs: The system cannot find the path specified."
pub fn error_text(error: &LoadError) -> String {
    use std::error::Error;
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    if let LoadError::Fatal(diagnostics) = error {
        for diagnostic in diagnostics {
            text.push('\n');
            text.push_str(&diagnostic.to_string());
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn loads_in_the_background() {
        let (sender, receiver) = mpsc::channel();
        let library = MibLibrary::new().with_system_folders(false);
        spawn(library, 7, move |loaded| sender.send(loaded).unwrap());
        let loaded = receiver.recv().unwrap();
        assert_eq!(loaded.generation, 7);
        assert!(loaded.result.unwrap().find("enterprises").is_some());
    }

    #[test]
    fn missing_folder_is_explained() {
        let (sender, receiver) = mpsc::channel();
        let library = MibLibrary::new()
            .with_system_folders(false)
            .with_folder("/no/such/folder/for/ferosnmp");
        spawn(library, 1, move |loaded| sender.send(loaded).unwrap());
        let error = receiver.recv().unwrap().result.unwrap_err();
        let text = error_text(&error);
        assert!(text.contains("/no/such/folder/for/ferosnmp"), "{text}");
    }
}

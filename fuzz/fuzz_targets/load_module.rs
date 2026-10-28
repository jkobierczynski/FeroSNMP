//! Feed arbitrary bytes to the MIB loader as if they were a MIB file.
//!
//! Whatever the input, loading must return a set or an error: never panic,
//! hang or run out of memory. Run with:
//!
//! ```sh
//! cargo +nightly fuzz run load_module
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use mib_core::Strictness;

fuzz_target!(|data: &[u8]| {
    let name = module_name(data);
    let _ = mib_core::load_module_text(&name, data, Strictness::Permissive);
});

/// The word before the first `DEFINITIONS`, so the loader resolves the module
/// instead of skipping it for having the wrong name.
fn module_name(data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data);
    let mut previous = None;
    for word in text.split_whitespace() {
        if word == "DEFINITIONS" {
            if let Some(name) = previous {
                return name;
            }
        }
        previous = Some(word.to_string());
    }
    "FUZZ-MIB".to_string()
}

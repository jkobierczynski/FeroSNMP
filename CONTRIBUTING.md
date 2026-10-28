# Contributing to FeroSNMP

## Layout

| Path | What it is |
|------|------------|
| `crates/mib-core` | Loads MIB libraries and answers "what is this OID/name?". Wraps [mib-rs](https://docs.rs/mib-rs/0.10.0). |
| `crates/snmp-core` | Async SNMP engine (from milestone M5). |
| `crates/trap-rx` | Trap and INFORM receiver (from milestone M9). |
| `apps/cli` | The `ferosnmp` command-line tool. See [apps/cli/README.md](apps/cli/README.md). |
| `crates/fero-win32` | Small Win32 helpers shared by the Fero tools: window classes, DPI, fonts, splitters, dialogs. |
| `apps/gui` | The Win32 GUI (Windows only). See [apps/gui/README.md](apps/gui/README.md). |
| `corpus/` | Real-world MIBs the loader is tested against. See [corpus/README.md](corpus/README.md). |
| `fuzz/` | Fuzz targets, run with `cargo fuzz`. |

## Building and testing

Install Rust with [rustup](https://rustup.rs). The compiler version is pinned in
`rust-toolchain.toml`; rustup downloads it automatically the first time you
build.

```sh
cargo build --workspace          # build everything
cargo test --workspace           # run all tests
cargo fmt --all                  # format the code
cargo clippy --workspace --all-targets -- -D warnings   # lint, as CI does
```

The GUI only builds into a real program on Windows. On Linux and macOS the
`ferosnmp-gui` binary is a stub, so `cargo build --workspace` works everywhere,
and the GUI's settings, search and details code is still tested there. To
check that the Windows-only code compiles from Linux:

```sh
rustup target add x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

## The MIB corpus

The IETF modules in `corpus/ietf/` are committed and tested on every build.
Vendor modules and older RFC modules cannot be redistributed, so they are
downloaded on demand:

```sh
corpus/fetch.sh            # on Windows, run it from Git Bash
cargo test -p mib-core --test corpus
```

Without the downloaded files, the full-corpus test is skipped.

### Snapshot tests

`crates/mib-core/tests/corpus.rs` stores what the loader made of the corpus
(modules loaded and every diagnostic) in `crates/mib-core/tests/snapshots/`. If
you change how MIBs are loaded, these tests fail and show the difference. When
the new output is what you want, accept it:

```sh
cargo install cargo-insta          # once
cargo insta test --accept -p mib-core
```

Then review the changed `.snap` files in your diff like any other change.

The command-line tool has snapshot tests too (`apps/cli/tests/cli.rs`): the
output of `ferosnmp lint corpus/ietf` and of the `mib` commands. Accept
intended changes with `cargo insta test --accept -p ferosnmp-cli`.

## Dependencies and licences

FeroSNMP is licensed GPL-3.0-or-later. `deny.toml` lists the licences a
dependency may use; `cargo deny check` (run in CI) fails on anything else.
`mib-rs` is pinned to an exact version because it is pre-1.0; upgrade it on
purpose and run the corpus tests.

```sh
cargo install cargo-deny cargo-audit   # once
cargo deny check
cargo audit
```

## Fuzzing

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cargo +nightly fuzz run load_module
```

CI runs this target for 60 seconds on every pull request. If it finds a
crash, the failing input is attached to the run as the `fuzz-artifacts`
download; reproduce it with `cargo +nightly fuzz run load_module <file>`.

## Continuous integration

Every pull request must pass these checks:

- **Build and test (Windows)**: format check, clippy and all tests on Windows.
- **cargo deny**: licences, banned crates and crate sources.
- **cargo audit**: known security advisories.
- **Fuzz smoke run**: 60 seconds of fuzzing the MIB loader.

Two more jobs run but are not required: **Build and test (Linux)** (also
builds the docs) and **Full MIB corpus** (downloads the vendor MIBs, so it
depends on vendor sites being reachable).

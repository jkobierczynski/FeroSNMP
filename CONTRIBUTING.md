# Contributing to FeroSNMP

## Layout

| Path | What it is |
|------|------------|
| `crates/mib-core` | Loads MIB libraries and answers "what is this OID/name?". Wraps [mib-rs](https://docs.rs/mib-rs/0.10.0). |
| `crates/snmp-core` | Async SNMP engine on tokio: GET, GETNEXT, GETBULK, WALK. See [crates/snmp-core/README.md](crates/snmp-core/README.md). |
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

### Query tests against net-snmp

`apps/cli/tests/agent.rs` runs `ferosnmp get|next|bulk|walk` against a real
net-snmp agent that serves fixed values, and compares the output with stored
snapshots. It needs `snmpd` and `python3`, so it runs on Linux (or WSL):

```sh
sudo apt install snmpd                     # once (Debian, Ubuntu, WSL)
crates/snmp-core/tests/agent/run-agent.sh  # starts snmpd on 127.0.0.1:11161
FEROSNMP_TEST_AGENT=127.0.0.1:11161 cargo test -p ferosnmp-cli --test agent
crates/snmp-core/tests/agent/run-agent.sh stop
```

Without `FEROSNMP_TEST_AGENT` these tests are skipped. The engine's own
tests (`cargo test -p snmp-core`) use a small fake agent and run everywhere.

For big walks there is a tiny agent with a made-up table of any size, which
runs on Windows too:

```sh
cargo run --release -p snmp-core --example big_agent -- 100000 127.0.0.1:11162
```

Walking `1.3.6.1.4.1.99999` then returns 300,000 values (three columns).

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

Three more jobs run but are not required: **Build and test (Linux)** (also
builds the docs), **Full MIB corpus** (downloads the vendor MIBs, so it
depends on vendor sites being reachable) and **Query tests (net-snmp)**
(the golden-file query tests against net-snmp `snmpd`).

## Releases

Releases are built by `.github/workflows/release.yml` when a tag such as
`v0.1.0` is pushed. It checks that the tag matches `version` in the root
`Cargo.toml`, runs the tests on Windows and Linux, and publishes a GitHub
release with:

- `ferosnmp-vX.Y.Z-windows-x64.zip`: `ferosnmp.exe` and `ferosnmp-gui.exe`
  (the C runtime is linked in, so nothing else needs installing);
- `ferosnmp-vX.Y.Z-linux-x64.tar.gz`: the `ferosnmp` command-line tool;
- `SHA256SUMS`: the SHA-256 checksum of each archive.

The release notes are the version's section of `CHANGELOG.md`. To make a
release:

1. Set the version in `Cargo.toml` and add its section to `CHANGELOG.md`, in
   a pull request.
2. After it is merged, tag the merge commit and push the tag:

   ```sh
   git checkout main && git pull
   git tag -a v0.1.0 -m "FeroSNMP 0.1.0"
   git push origin v0.1.0
   ```

To check a download on Windows, compare the output of
`Get-FileHash ferosnmp-v0.1.0-windows-x64.zip` with its line in `SHA256SUMS`
(`sha256sum -c SHA256SUMS` does all of them on Linux).

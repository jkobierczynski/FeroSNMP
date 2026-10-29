# Developing FeroSNMP

How FeroSNMP is put together, and how to add to it. For the commands to
build, test, fuzz and release, see [CONTRIBUTING.md](CONTRIBUTING.md); this
file is about the design behind them.

## The big picture

One Cargo workspace: two programs on top of three core crates, so every
feature can be tested without a window and reused by other tools.

```
 apps/gui (ferosnmp-gui)          apps/cli (ferosnmp)
   Win32 window, Windows only       command line, every platform
        |        \                    /        |
        |      crates/fero-win32     /         |
        |      (Win32 helpers)      /          |
        v                          v           v
 crates/snmp-core  ------------>  crates/mib-core      crates/trap-rx
   async SNMP engine on tokio      MIB loading,          trap and INFORM
   own v1/v2c/v3 codec and USM     lookup, diagnostics   receiver (M9)
        |                                |
        v                                v
   snmp2 (BER reader only)          mib-rs =0.10.0 (pinned, wrapped)
```

| Crate | Owns | Read first |
|-------|------|------------|
| `mib-core` | Loading a MIB library, the OID tree, node details, diagnostics with fix hints, "why this symbol?" traces, source views | `src/lib.rs` |
| `snmp-core` | The SNMP engine: requests, walks, timeouts, retries, SNMPv3, value formatting through the MIBs | [README](crates/snmp-core/README.md) |
| `trap-rx` | Trap and INFORM receiver; empty until M9 | `src/lib.rs` |
| `fero-win32` | Win32 plumbing that is not about SNMP: window classes, DPI, fonts, splitters, in-memory dialogs; meant to be shared with FeroSSH | `src/lib.rs` |
| `ferosnmp-cli` | The `ferosnmp` command | [README](apps/cli/README.md) |
| `ferosnmp-gui` | The window | [README](apps/gui/README.md), "How the code is organised" |

The dependencies only point downwards: `mib-core` knows nothing about SNMP,
`snmp-core` knows nothing about windows, and only `fero-win32` and the
GUI know about Win32.

## Design decisions worth knowing

**mib-rs is wrapped completely.** mib-rs does the parsing and resolution,
but it is pre-1.0 and may break its API in any minor release. So it is pinned
to an exact version in the root `Cargo.toml`, only `mib-core` depends on it,
and none of its types appear in `mib-core`'s public API. An upgrade touches
only the private conversion code in `mib-core`; the corpus snapshot tests
then show exactly what changed in the results.

**MIB sets are immutable and shared.** `MibLibrary::load` returns a
`MibSet`, a cheap-to-clone handle (an `Arc` inside) that never changes. The
window, the engine and the formatter all read the same set without locks.
Reloading builds a new set on a background thread and swaps it in.

**Problems are collected, not thrown.** A vendor MIB with mistakes should
still load and browse. Loading collects `Diagnostic`s (severity, file, line,
column, message, fix hint) and carries on; strictness decides which ones are
reported. Use `Result` for failures of the tool itself (a folder that cannot
be read), and `Diagnostic` for problems in the MIBs.

**The SNMP engine has its own codec.** snmp2's sessions silently drop the
rest of a variable list when they meet a value they cannot decode, so a walk
can end early without an error, which is the worst kind of wrong answer for
this tool. `snmp-core` therefore encodes requests itself and uses snmp2 only
for its low-level BER reader; unusual values are kept as `Value::Other`. The
same goes for SNMPv3: the USM (key localization, HMAC, DES and AES) is our
own code on the RustCrypto crates, tested against the RFC test vectors, so
there is no OpenSSL to build or ship. The full findings are in the
[snmp-core README](crates/snmp-core/README.md).

**The window never waits.** The UI thread only runs the Win32 message loop.
MIB loading runs on a background thread; queries run on one thread that owns
a tokio runtime and the engine. Both hand results to the window with
`PostMessage`, and the window collects what is waiting at most every 50 ms,
so a fast walk cannot starve painting. Big result lists are virtual ListViews
(`LVS_OWNERDATA`) that only ask for the rows on screen.

**Secrets stay out of files.** Settings live in
`%APPDATA%\FeroSNMP\settings.toml`; SNMPv3 passwords go to the Windows
Credential Manager. The CLI takes them from options or environment
variables.

## Adding a feature

The milestones in [ROADMAP.md](ROADMAP.md) all follow the same order, and it
works well for new features too:

1. **Core crate first.** Put the logic in `mib-core`, `snmp-core` or
   `trap-rx`, with unit tests. No printing, no Win32, no global state.
2. **Then the CLI.** Expose it as a `ferosnmp` command and add a snapshot
   test in `apps/cli/tests/`. This is the quickest way to see the feature
   working, and the snapshot keeps it working.
3. **Then the window.** Keep the Win32 code thin: anything that can be plain
   Rust (what a list row shows, what a search matches, how settings are
   saved) goes in a module outside `apps/gui/src/ui/`, where it is tested on
   every platform.
4. **Then the docs.** Update the app's README and add a line to
   `CHANGELOG.md` under `[Unreleased]`.

Each milestone has a tracking issue with one sub-issue per scope item; refer
to the sub-issue in the pull request (`Closes #NN`) so the roadmap stays
current by itself.

## Tests, from fast to slow

| Layer | Where | Runs |
|-------|-------|------|
| Unit tests | next to the code, `#[cfg(test)]` | everywhere, every build |
| MIB corpus snapshots | `crates/mib-core/tests/corpus.rs` | committed IETF MIBs every build; the vendor MIBs once fetched |
| CLI snapshots | `apps/cli/tests/cli.rs` | everywhere |
| Engine against a fake agent | `crates/snmp-core/src/tests*.rs` | everywhere |
| Golden files against net-snmp | `apps/cli/tests/agent.rs` | Linux or WSL with `snmpd`; the SNMPv3 matrix too |
| Fuzzing | `fuzz/` | 60 seconds on every pull request |

Snapshot tests use [insta](https://insta.rs): when output changes on purpose,
accept it with `cargo insta test --accept` and review the `.snap` diff like
code. A snapshot that changes for no reason is a bug.

## Rust notes for C and C++ programmers

- **Ownership replaces manual lifetime rules.** Where C++ would pass a raw
  pointer and a comment, Rust passes a borrow (`&T`) or moves the value. If
  the borrow checker refuses, the usual fix is to clone something cheap (an
  `Arc`, a small `String`) or to end a borrow sooner, not to fight it.
- **`Arc` is `shared_ptr`, `Box` is `unique_ptr`.** `Rc`/`Cell`/`RefCell`
  are the single-thread versions; the GUI uses them for window state.
- **Errors are values.** Functions return `Result<T, E>` and `?` passes an
  error up. Error types are enums made with `thiserror`. `unwrap()` is for
  tests and for things that truly cannot fail, with a comment saying why.
- **`unsafe` marks what the compiler cannot check.** Every Win32 call is
  `unsafe`; keep each block small and say in a comment why it is fine. The
  one trap: Win32 calls back into the window while a call such as
  `TVM_EXPAND` is still running, so never hold a `RefCell` borrow across a
  Win32 call (see `fero_win32::window`).
- **Async is only in `snmp-core` and `trap-rx`.** Everything else is plain
  synchronous code. An `async fn` does nothing until it is awaited on the
  tokio runtime; the GUI talks to the engine through a handle and never
  awaits itself.
- **Let the tools teach you.** `cargo clippy` explains most of its warnings
  with a link; `cargo doc --open` shows the documentation of this workspace
  and every dependency; `cargo test -p <crate> <name>` runs one test.

## Debugging

- **Command line first.** Most GUI problems reproduce with `ferosnmp`, which
  is easier to run under a debugger and to script. `--log` prints every SNMP
  request with its timing; `--raw` shows OIDs, tags and bytes.
- **The window.** Build with `cargo build -p ferosnmp-gui` (debug) and
  attach Visual Studio or WinDbg to `ferosnmp-gui.exe`; the `.pdb` sits next
  to it in `target\debug`. The Log tab shows what the engine did.
- **Big walks.** `cargo run --release -p snmp-core --example big_agent --
  100000 127.0.0.1:11162` serves a table of any size on every platform.
- **SNMPv3.** `docs/snmpv3.md` sets up a matching user on net-snmp's
  `snmpd`; the test agent in `crates/snmp-core/tests/agent/` has a user for
  every protocol combination.

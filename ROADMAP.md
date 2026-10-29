# FeroSNMP roadmap

The road to 1.0 in four phases and eleven milestones. Each phase ends with a
usable, tagged release. Within a phase the core crate comes first, then the
command-line tool, then the window (see [DEVELOPMENT.md](DEVELOPMENT.md)).

Every milestone has a tracking issue on GitHub with one sub-issue per scope
item; the issue lists the full scope. This file is the overview.

| Milestone | Status | Release |
|-----------|--------|---------|
| [M0: Workspace and CI skeleton](https://github.com/jkobierczynski/FeroSNMP/issues/10) | Done | |
| **Phase 1: read-only MIB browser** | | |
| [M1: mib-core, load and query MIBs](https://github.com/jkobierczynski/FeroSNMP/issues/11) | Done | |
| [M2: CLI, browse and lint](https://github.com/jkobierczynski/FeroSNMP/issues/12) | Done (smilint comparison still open, optional) | |
| [M3: Win32 main window and MIB tree](https://github.com/jkobierczynski/FeroSNMP/issues/13) | Done | |
| [M4: Diagnostics panel](https://github.com/jkobierczynski/FeroSNMP/issues/14) | Done | v0.1.0 |
| **Phase 2: SNMP queries** | | |
| [M5: snmp-core engine (v1/v2c) and CLI queries](https://github.com/jkobierczynski/FeroSNMP/issues/15) | Done | |
| [M6: GUI queries](https://github.com/jkobierczynski/FeroSNMP/issues/16) | Done | v0.2.0 |
| **Phase 3: SNMPv3, SET and tables** | | |
| [M7: SNMPv3 (USM)](https://github.com/jkobierczynski/FeroSNMP/issues/17) | Merged; a test on a real vendor device is still open | |
| [M8: Table view and SET](https://github.com/jkobierczynski/FeroSNMP/issues/18) | Next | v0.3.0 |
| **Phase 4: traps and 1.0** | | |
| [M9: trap-rx receiver](https://github.com/jkobierczynski/FeroSNMP/issues/19) | Planned | |
| [M10: Trap UI](https://github.com/jkobierczynski/FeroSNMP/issues/20) | Planned | v1.0.0 |

Status as of 2026-10-29. The issues are the live source; update this table
when a milestone closes.

## Phase 1: read-only MIB browser

No network code and no async Rust: the gentlest way into the language, and
already useful on its own.

- **M0 Workspace and CI skeleton.** Cargo workspace, toolchain and lint
  config, `deny.toml`, and the four required CI checks: *Build and test
  (Windows)*, *cargo deny*, *cargo audit*, *Fuzz smoke run*.
  *Done when* all four pass and are required in branch protection.
- **M1 mib-core.** A stable API over mib-rs: MIB library with ordered
  sources, module names indexed from file contents, immutable shared
  `MibSet`, node details, diagnostics, and corpus snapshot tests.
  *Done when* the corpus test runs in CI and no mib-rs type is public.
- **M2 CLI.** `ferosnmp mib show|tree|search` and `ferosnmp lint` with
  strict mode and JSON output. *Done when* `ferosnmp lint corpus/` is
  snapshot-tested in CI.
- **M3 Win32 window.** Main window with splitters, DPI awareness and the
  modern controls manifest; lazy MIB tree with search; details pane;
  background loading; settings for MIB folders and strictness.
  *Done when* the full corpus browses without freezing on Windows 10 and 11.
- **M4 Diagnostics.** Diagnostics tab, source view with the error
  highlighted, fix hints, "Why this symbol?", export, drag and drop, reload,
  and the release workflow. *Done when* `v0.1.0` is released.

## Phase 2: SNMP queries

- **M5 snmp-core engine.** Async engine on tokio with GET, GETNEXT, GETBULK
  and WALK over v1 and v2c; safe walk termination; formatting through the
  MIBs; `ferosnmp get|next|bulk|walk`; golden-file tests against net-snmp.
  *Done when* those tests pass in CI.
- **M6 GUI queries.** Engine on a worker thread, saved agents, query
  toolbar, virtual results list with Cancel, raw view and Log tab.
  *Done when* a 100,000-row walk streams smoothly and `v0.2.0` is released.

## Phase 3: SNMPv3, SET and tables

- **M7 SNMPv3.** USM with every authentication and privacy protocol,
  pure-Rust crypto, a clear error per failure class, passwords in the
  Credential Manager, and a CI matrix against net-snmp. *Done when* the
  matrix is green and a real vendor device has been tested by hand.
- **M8 Table view and SET.** Split table instances into index columns
  (multi-part, AUGMENTS, IMPLIED); SET with enum, range and size checks.
  *Done when* the table and SET tests pass and `v0.3.0` is released.

## Phase 4: traps and 1.0

- **M9 trap-rx.** UDP receiver for v1 (translated per RFC 3584), v2c and v3
  traps and INFORMs, decoded against the MIBs; port 162 handling on Windows
  and Linux; `ferosnmp traps listen`. *Done when* every notification type
  decodes correctly in CI.
- **M10 Trap UI and 1.0.** Traps tab with filters, a bounded buffer and
  export; trap settings; dark mode for the tree and lists; layout
  persistence. *Done when* every v1 scope item works and `v1.0.0` is
  released.

## Out of scope for 1.0

Agent simulation, long-term polling and graphing, alert routing, NMS
features, and editing MIBs beyond showing diagnostics.

## Open decisions

| Decision | Needed by | State |
|----------|-----------|-------|
| Pure-Rust crypto or OpenSSL | M7 | Decided: pure Rust (RustCrypto) |
| Windows 10 and 11, or 11 only | M3 | Built for Windows 10 and 11 (manifest and docs) |
| Commit older RFC MIBs or fetch them | M1 | Fetched by `corpus/fetch.sh` for now |
| Publish `mib-core` and `snmp-core` on crates.io | M10 | Open; `publish = false` for now |

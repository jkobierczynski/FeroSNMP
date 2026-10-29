# Changelog

All notable changes to FeroSNMP. Versions follow [Semantic Versioning](https://semver.org/);
before 1.0, a minor version may change behaviour.

## [Unreleased]

SNMPv3 with the User-based Security Model.

### SNMPv3

- Security levels `noAuthNoPriv`, `authNoPriv` and `authPriv`;
  authentication with MD5, SHA-1, SHA-224, SHA-256, SHA-384 and SHA-512;
  privacy with DES, AES-128, AES-192 and AES-256, with both AES-192/256 key
  extensions (most agents, and Cisco's as `AES-192C`/`AES-256C`).
- Pure-Rust crypto (RustCrypto); no OpenSSL.
- Engine discovery, remembered per agent with the derived keys; a clock
  that is out of step is synchronized, and an agent whose engine ID changed
  is discovered again.
- A clear error for each failure: unknown user, wrong authentication
  password or protocol, wrong privacy password or protocol (also when the
  agent silently drops what it cannot decrypt, as net-snmp does), security
  level the user does not have, time window, unknown context. An
  `authorizationError` comes with a hint about the agent's access rules.
- Command line: `-v 3` (implied by `-u`) with net-snmp's options `-u`, `-l`,
  `-a`, `-A`, `-x`, `-X`, plus `--context`, `--context-engine-id`, and the
  environment variables `FEROSNMP_AUTH_PASSWORD` and
  `FEROSNMP_PRIV_PASSWORD`. New `ferosnmp discover AGENT` shows an agent's
  engine ID, boots and time.
- Window: SNMPv3 settings in File > Agents; the passwords are kept in the
  Windows Credential Manager, not in `settings.toml`. The Log tab shows the
  engine ID, boots and uptime.
- Tests: the net-snmp CI job walks with every authentication and privacy
  protocol (30 users), and checks each error.
- How to add an SNMPv3 user to net-snmp's snmpd: `docs/snmpv3.md`.

## [0.2.0]

SNMP queries: GET, GETNEXT, GETBULK and WALK over SNMPv1 and SNMPv2c, in the
window and on the command line.

### SNMP queries in the window (`ferosnmp-gui`)

- A second toolbar row with an **Agent** box, an **Object** box and the
  **Get**, **Next**, **Bulk**, **Walk** and **Cancel** buttons. Selecting a
  node in the tree fills in the Object box (`sysDescr.0` for a scalar).
- **Saved agents** (File > Agents): address, port, version, community,
  timeout, retries and max-repetitions, kept in `settings.toml`. An address
  typed in the Agent box works too, with the default settings.
- Queries run on a separate thread, so the window never freezes: results
  stream into the **Results** tab, a virtual list that handles walks of
  hundreds of thousands of rows, with the row count in the tab title and in
  the status bar. **Cancel** (or **Esc**) stops a query at once.
- **F5** repeats the last query; **Enter** in the Agent or Object box runs
  the last kind of query again. Next puts the instance it returned in the
  Object box, so pressing Next again steps through the agent.
- **Ctrl+C** copies the selected rows (tab-separated, for spreadsheets);
  the right-click menu copies a single column or the numeric OIDs, shows
  the object in the MIB tree, and exports every row as CSV (also
  **Ctrl+S**). **Ctrl+A** selects every row.
- **Ctrl+F** searches the results when the Results list was used last, and
  the MIB tree otherwise.
- **Raw** (View > Raw values, **Ctrl+Shift+R**) shows numeric OIDs, ASN.1
  tags and the bytes in hex.
- The **Log** tab lists every request with its request-id, retries and
  timing, and how each query ended.
- Changed: **Reload MIBs** moved from F5 to **Ctrl+R**.

### SNMP queries (command-line tool)

- `ferosnmp get`, `next`, `bulk` and `walk` query an agent over SNMPv1 or
  SNMPv2c, with community, timeout and retries options.
- Walks use GETBULK with SNMPv2c and GETNEXT with SNMPv1 (or `--no-bulk`),
  fall back to GETNEXT when an agent fails on GETBULK, ask for fewer
  values when a response is too big, and stop safely at the end of the
  subtree, at `endOfMibView`, or when a broken agent returns OIDs that do
  not increase.
- Values are shown with the loaded MIBs: names, enum labels, BITS names,
  DISPLAY-HINTs (MAC addresses, DateAndTime, decimals) and units. `--raw`
  shows numeric OIDs, ASN.1 tags and the bytes in hex; `-n` numeric OIDs.
- `--log` prints every request with its request-id, retries and timing.
- Values of any type are kept, also ones that are malformed or unknown, so
  a walk never ends early without saying so.

### For developers

- `snmp-core` gained `resolve_object` (names such as `IF-MIB::ifDescr.3` to
  OIDs, shared by the CLI and the window) and `Formatter::value_text` (a
  value split into type and text, for table columns).
- `cargo run --release -p snmp-core --example big_agent` serves a made-up
  table of 100,000 rows (or any number), for trying big walks.

## [0.1.0]

The first release: a read-only MIB browser and linter for Windows 10 and 11,
and a command-line tool for Windows and Linux.

### MIB loading

- Loads SMIv1 and SMIv2 MIB modules from folders, the net-snmp and libsmi
  folders on the machine, and built-in copies of the foundation modules.
- Problems are reported with file, line, column and a code, at three
  strictness levels.
- Fix hints for the common problems: a missing import (naming the modules
  that define the symbol), an import from the wrong module, an imported
  module that is in no MIB folder (or is in a file with another name),
  SMIv1 constructs in an SMIv2 module, invalid identifiers, and a module
  whose file name does not match.
- "Why this symbol?": an explanation of how a name is resolved, or why it
  is not.

### Command-line tool (`ferosnmp`)

- `mib show`, `mib tree`, `mib search`: look up names, OIDs and patterns.
- `mib why`: the resolution trace of a name.
- `lint`: check MIB files, as text or JSON, with hints and CI-friendly exit
  codes.

### Windows program (`ferosnmp-gui`)

- MIB tree with lazy loading, details pane, search by name, OID or pattern.
- Diagnostics tab with every problem and its fix hint; the MIB source opens
  read-only with the problem highlighted.
- Why this symbol? (Ctrl+W), export of the diagnostics as text or JSON
  (Ctrl+E), drag and drop of files and folders, and reloading when a MIB is
  edited in another program.
- Settings for MIB folders and strictness; per-monitor DPI awareness.

# FeroSNMP

A native Windows SNMP MIB browser and client, written in Rust.

FeroSNMP's strongest feature is loading messy vendor MIBs correctly and
explaining clearly when they fail: every problem comes with the file, line and
column, and most with a concrete fix hint. The window is built from plain Win32
controls, so it starts fast, looks like Windows and needs no runtime. The same
MIB and SNMP code is available as a command-line tool, `ferosnmp`, that also
runs on Linux and macOS.

## What it does

- **MIB browser**: load a library of MIB folders (SMIv1 and SMIv2), with
  imports resolved by the module name inside each file, not the file name.
  Browse the OID tree, search by name, OID or wildcard pattern, and see
  everything about a node: type chain, access, status, units, enums, ranges
  and INDEX.
- **Diagnostics and lint**: every problem found while loading, with the MIB
  source highlighted at the error and a fix hint for the common vendor
  mistakes (missing imports, misnamed files, SMIv1 constructs in SMIv2,
  invalid identifiers). "Why this symbol?" shows how a name resolves, or why
  it does not. `ferosnmp lint` does the same from the command line, as text
  or JSON, with CI-friendly exit codes.
- **SNMP queries**: GET, GETNEXT, GETBULK and WALK over SNMPv1, SNMPv2c and
  SNMPv3 (USM with MD5 up to SHA-512, DES and AES-128/192/256). Values are
  shown through the MIBs: names, enum labels, DISPLAY-HINTs and units. Walks
  of hundreds of thousands of rows stream into a list that never freezes the
  window, and stop safely on agents that return OIDs out of order.
- **Tables and SET**: a table view that splits each row's instance by the
  table's INDEX into real columns, and SET with the value built from the
  MIB type and checked against its enums, ranges and sizes.
- **Trap and INFORM receiver**: `ferosnmp traps listen` receives SNMPv1,
  SNMPv2c and SNMPv3 traps and INFORMs, acknowledges INFORMs, and decodes
  each notification against the loaded MIBs, with a hint to load the
  vendor's MIB for anything unknown. Text or JSON Lines output, with
  filters. See [docs/traps.md](docs/traps.md).
- **Planned for 1.0**: a Traps tab in the window. See
  [ROADMAP.md](ROADMAP.md).

The window runs on Windows 10 and 11. The command-line tool builds on Windows,
Linux and macOS.

## Download

Releases are on the [GitHub releases page](https://github.com/jkobierczynski/FeroSNMP/releases).
Each one has a Windows zip with `ferosnmp-gui.exe` and `ferosnmp.exe`, a Linux
archive with `ferosnmp`, and a `SHA256SUMS` file to check them against. Nothing
else needs installing.

## Quick start

In the window, add your MIB folders under **File > Settings**, then type an
agent address in the **Agent** box, click a node in the tree and press
**Walk**. The [GUI guide](apps/gui/README.md) explains every key and menu.

From the command line:

```sh
ferosnmp mib -M mibs/ show IF-MIB::ifDescr     # what is this object?
ferosnmp mib -M mibs/ search 'if*Octets'       # find objects by pattern
ferosnmp lint mibs/VENDOR-MIB.my               # what is wrong with this MIB?
ferosnmp walk -c public 192.0.2.1 system       # walk an agent
ferosnmp walk -u admin -l authPriv -a SHA-256 -x AES 192.0.2.1 ifTable
ferosnmp traps listen --port 1162 -M mibs/      # print incoming traps
```

See the [command-line guide](apps/cli/README.md) for all commands and options,
and [docs/snmpv3.md](docs/snmpv3.md) for SNMPv3.

## Documentation

| Document | For |
|----------|-----|
| [apps/gui/README.md](apps/gui/README.md) | Using the window |
| [apps/cli/README.md](apps/cli/README.md) | Using the command-line tool |
| [docs/snmpv3.md](docs/snmpv3.md) | SNMPv3 setup, also on net-snmp's `snmpd` |
| [docs/traps.md](docs/traps.md) | Receiving traps: Windows port 162 and firewall, test traps from Debian |
| [CHANGELOG.md](CHANGELOG.md) | What changed in each version |
| [ROADMAP.md](ROADMAP.md) | Milestones and what comes next |
| [DEVELOPMENT.md](DEVELOPMENT.md) | How the code is designed, for developers |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Building, testing, CI and releases |

## Licence

FeroSNMP is free software under the [GNU General Public License v3.0 or later](LICENSE).
The MIB modules in `corpus/` keep their own licences; see
[corpus/README.md](corpus/README.md).

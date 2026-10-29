# Changelog

All notable changes to FeroSNMP. Versions follow [Semantic Versioning](https://semver.org/);
before 1.0, a minor version may change behaviour.

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

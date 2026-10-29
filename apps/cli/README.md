# ferosnmp command-line tool

Browse SNMP MIBs and check MIB files for problems.

```sh
cargo build --release -p ferosnmp-cli     # builds target/release/ferosnmp(.exe)
```

## Where MIBs come from

Every command loads MIB modules from, in this order:

1. folders given with `-M DIR` (repeat `-M` for more folders);
2. folders in the `FEROSNMP_MIBDIRS` environment variable, separated like
   `PATH` (`;` on Windows, `:` elsewhere);
3. the folders net-snmp and libsmi use on this machine, if they are installed;
4. built-in copies of the foundation modules (`SNMPv2-SMI`, `SNMPv2-TC`, ...).

Folders are searched recursively, and files are matched on the module name
inside them, not on the file name. `--no-system-mibs` leaves out 2 and 3, so
only `-M` folders and the built-in modules are used.

## Browsing

```sh
ferosnmp mib -M corpus show sysDescr
ferosnmp mib -M corpus show IF-MIB::ifInOctets ifDescr.3 1.3.6.1.2.1.1.5.0
ferosnmp mib -M corpus tree system
ferosnmp mib -M corpus tree --depth 2            # the top of the whole tree
ferosnmp mib -M corpus search 'if*Octets'        # * any text, ? one character
ferosnmp mib -M corpus search 'CISCO-ENVMON-MIB::*'
```

`show` accepts a name, `MODULE::name`, a numeric OID or an instance such as
`ifDescr.3`. When the name is defined in more than one module, the other
definitions are listed under "Also defined". Search ignores case; a pattern
without `*` or `?` matches any name that contains it.

## Linting

```sh
ferosnmp lint MY-VENDOR-MIB.my                   # one file
ferosnmp lint mibs/ -M more-mibs/                # a folder; imports also from more-mibs/
ferosnmp lint --strict MY-VENDOR-MIB.my          # also style problems
ferosnmp lint --format json mibs/ > lint.json
```

Each problem is printed as `file:line:column: severity: MODULE: message [code]`.
Imports are looked up in the linted folders, the folder of each linted file,
and the usual MIB folders above, but only problems in the linted files are
reported.

`lint` reads modules more strictly than the browser: it does not go looking
for an import in other modules when the module named in `IMPORTS` lacks it.
`--strict` also reports style and informational problems.

Most problems come with a fix hint on the line below, for example which
module to import a missing symbol from, or which file defines the module an
import names:

```text
mibs/ACME-MIB:18:17: error: ACME-MIB: unresolved type: "acmeLevel" references unknown type "AcmeLevel" [type-unknown]
    hint: AcmeLevel is defined in ACME-TC-MIB: add `AcmeLevel FROM ACME-TC-MIB` to the IMPORTS.
```

In JSON output the hint is the `hint` field.

## Why does a name resolve (or not)?

```sh
ferosnmp mib -M corpus why ifInOctets
ferosnmp mib -M mibs why ACME-MIB::acmeLevel     # as seen from ACME-MIB's IMPORTS
ferosnmp mib -M mibs why --lint ACME-MIB::acmeLevel
```

`why` explains how a name is looked up: which modules define it, what the
`IMPORTS` of the module say, which step found it (a direct import, a
well-known SMI name, a search of other modules, ...) and, when it fails,
every place that refers to it and why that did not resolve. `--lint` and
`--strict` look the name up as `ferosnmp lint` does.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success; `lint` found nothing at or above `--fail-on` |
| 1 | `mib show` or `mib search` found nothing, `mib why` could not resolve the name, or `lint` found problems at or above `--fail-on` (default `error`) |
| 2 | The command could not run: bad arguments, or a file or folder that cannot be read |

For a CI job that should also fail on warnings, use
`ferosnmp lint --fail-on warning mibs/`.

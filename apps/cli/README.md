# ferosnmp command-line tool

Browse SNMP MIBs, check MIB files for problems, and query SNMP agents.

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

## Querying an agent

```sh
ferosnmp get  192.0.2.1 sysDescr.0 sysUpTime.0
ferosnmp next 192.0.2.1 ifDescr
ferosnmp bulk 192.0.2.1 ifDescr ifOperStatus --max-repetitions 5
ferosnmp walk 192.0.2.1 system
ferosnmp walk 192.0.2.1 ifTable -c private -v 1
ferosnmp walk 192.0.2.1:1161 1.3.6.1.4.1.9 --log    # every request with its timing
ferosnmp walk 192.0.2.1 system -u monitor -a SHA-256 -A "auth pass" -x AES -X "priv pass"
ferosnmp discover 192.0.2.1                         # SNMPv3 engine ID, boots and time
```

SNMPv3, including how to add a user to net-snmp's snmpd and what each error
means, is described in [docs/snmpv3.md](../../docs/snmpv3.md).

The agent is a host name or address, optionally with `:port` (default 161;
IPv6 with a port as `[2001:db8::1]:161`). Objects are names (`sysDescr`),
`MODULE::name`, numeric OIDs, each optionally with an instance
(`ifDescr.3`). Without a root, `walk` walks all of mib-2.

| Option | Default | Meaning |
|--------|---------|---------|
| `-v 1\|2c\|3` | `2c`, or `3` with `-u` | SNMP version |
| `-c TEXT` | `public` | Community (SNMPv1, SNMPv2c) |
| `-u NAME` | | SNMPv3 user |
| `-l LEVEL` | from the passwords | `noAuthNoPriv`, `authNoPriv` or `authPriv` |
| `-a PROTOCOL` | `SHA` | Authentication: `MD5`, `SHA`, `SHA-224`, `SHA-256`, `SHA-384`, `SHA-512` |
| `-A TEXT` | | Authentication password; or set `FEROSNMP_AUTH_PASSWORD` |
| `-x PROTOCOL` | `AES` | Privacy: `DES`, `AES`, `AES-192`, `AES-256`, `AES-192C`, `AES-256C` |
| `-X TEXT` | the `-A` password | Privacy password; or set `FEROSNMP_PRIV_PASSWORD` |
| `--context NAME` | empty | SNMPv3 context name |
| `--context-engine-id HEX` | the agent's | SNMPv3 context engine ID, for proxies |
| `-t SECONDS` | `2` | Wait this long for each response |
| `-r N` | `2` | Send a request again this many times when no response arrives |
| `-n` | | Numeric OIDs instead of names |
| `--raw` | | Numeric OIDs, the ASN.1 tag and type, and the value's bytes in hex |
| `--log` | | Print each request, its request-id, retries and timing to standard error |
| `--max-repetitions N` | `10` | Values per GETBULK request (`bulk`, `walk`, `table`) |
| `--non-repeaters N` | `0` | `bulk`: how many of the first objects get one value only |
| `--no-bulk` | | `walk`, `table`: use GETNEXT also with SNMPv2c |

Output looks like net-snmp's, with the loaded MIBs used for names, enum
labels, BITS, display hints and units:

```text
IF-MIB::ifOperStatus.1 = INTEGER: up(1)
IF-MIB::ifPhysAddress.2 = STRING: "00:1b:21:0a:ff:09"
HOST-RESOURCES-MIB::hrSystemDate.0 = STRING: "2026-10-29,13:45:30.5,+1:0"
```

`walk` uses GETBULK with SNMPv2c and GETNEXT with SNMPv1. When an agent
answers GETBULK with an error it switches to GETNEXT, and when a response
would be too big it asks for fewer values at a time; both are reported on
standard error. A walk stops at the end of the subtree, at `endOfMibView`,
or, with an error, when the agent returns an OID that is not greater than
the previous one (a broken agent would otherwise loop forever). Walking a
single instance (`walk host sysDescr.0`) fetches it with GET.

## Tables

`table` walks a table and prints it as rows and columns. The instance part
of each OID is split into the table's INDEX values (RFC 2578, 7.7), also
when the INDEX has several parts, uses `IMPLIED`, or comes from another
row through `AUGMENTS`:

```sh
ferosnmp table 192.0.2.1 ifTable
ferosnmp table 192.0.2.1 ifXTable                   # AUGMENTS ifEntry: indexed by ifIndex
ferosnmp table 192.0.2.1 ipNetToMediaTable --format csv > arp.csv
```

```text
ipNetToMediaIfIndex  ipNetToMediaNetAddress  ipNetToMediaPhysAddress  ipNetToMediaType
2                    192.0.2.1               02:fc:00:00:00:05        dynamic(3)
2                    192.0.2.254             00:1b:21:0a:ff:09        static(4)
```

The index columns come first, then every column the agent returns values
for. The table, its row or any of its columns can be named. A row whose
instance does not fit the INDEX (a broken agent, or a MIB that does not
match the agent) is shown with its raw instance (`?.1.192.0.2.300`) and a
note that says which index part is wrong, and the exit code is 1.

## Writing values

`set` writes one or more values in one SET request, so the agent sets all
of them or none. Each value is read with the object's type from the MIB,
written the way the tool shows values:

```sh
ferosnmp set 192.0.2.1 -c private sysContact.0 "Jurgen" sysLocation.0 "Lab 2"
ferosnmp set 192.0.2.1 -c private ifAdminStatus.3 down
ferosnmp set 192.0.2.1 -u admin -a SHA-256 -A "auth pass" -X "priv pass" ifAlias.3 "uplink"
```

| MIB type | Write it as |
|----------|-------------|
| enumeration | the label (`down`), `down(2)`, or the number |
| INTEGER with a DISPLAY-HINT such as `d-1` | as shown: `-12.5` |
| OCTET STRING | text; with a DISPLAY-HINT as shown: `00:1b:21:0a:ff:09`, `2026-10-29,13:45:30.5` |
| BITS | bit names or numbers: `alpha gamma` |
| OBJECT IDENTIFIER | a name (`sysDescr`) or a numeric OID |
| IpAddress | `192.0.2.1` |

Before anything is sent, the value is checked against the MIB: the
object must be an instance (`sysContact.0`, not `sysContact`) that is
read-write or read-create, an enumeration value must be one of the labels,
and numbers and string lengths must be in the ranges and sizes the MIB
gives. A value that fails a check is not sent, and the message says what is
allowed. `--force` sends it anyway, to see how an agent reacts.

`--type` reads every value as another type, with net-snmp's letters: `i`
(INTEGER), `u` (Unsigned32), `c` (Counter32), `C` (Counter64), `t`
(TimeTicks), `a` (IpAddress), `o` (OBJECT IDENTIFIER), `s` (text), `x`
(hex bytes) or `b` (BITS). Use `--type x` to give a string's bytes in hex,
and `--type` for objects no loaded MIB defines.

The agent's answer is printed like `get`'s output. When the agent refuses,
the message says why in plain words (for example: a read-only community
gets `noAccess`, because net-snmp needs `rwcommunity` to write).

## Receiving traps

```sh
ferosnmp traps listen                              # port 162, every version
ferosnmp traps listen --port 1162 -M mibs/         # an unprivileged port
ferosnmp traps listen -u trapuser -a SHA -A "auth pass" -x AES -X "priv pass"
ferosnmp traps listen --users snmptrapd.conf       # SNMPv3 users as createUser lines
ferosnmp traps listen --format json > traps.jsonl  # JSON Lines, to save or process
ferosnmp traps listen --notification 'link*' --from 192.0.2.1 --count 1
```

Each notification is printed with the loaded MIBs as it arrives; INFORMs
are acknowledged. SNMPv1 traps are translated to SNMPv2 form (RFC 3584), so
an SMIv1 `TRAP-TYPE` is shown by its name. Problems (not a notification, an
unknown SNMPv3 user, a wrong password) go to standard error with a hint.

```text
2026-10-29T21:05:00.123Z 192.0.2.1:41003 SNMPv3 trap, user trapuser authPriv, engine 8000000001020304
  IF-MIB::linkDown, uptime 0:42:00.00
  IF-MIB::ifIndex.2 = INTEGER: 2
  IF-MIB::ifOperStatus.2 = INTEGER: down(2)
```

Port 162 needs `setcap` on Linux, and on Windows the SNMP Trap service must
be stopped; the error message says so. Setting up Windows (port, firewall)
and sending test traps from Debian, also SNMPv3, is described in
[docs/traps.md](../../docs/traps.md), with every option.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success; `lint` found nothing at or above `--fail-on` |
| 1 | `mib show` or `mib search` found nothing, `mib why` could not resolve the name, `lint` found problems at or above `--fail-on` (default `error`), the agent answered with an error (such as `noSuchName` or `notWritable`), a "no such object/instance" value, or an SNMPv3 security error (unknown user, wrong password, ...), or `table` found no rows or a row that does not fit the INDEX |
| 2 | The command could not run: bad arguments, a file or folder that cannot be read, an unknown object name, a `set` value the MIB does not allow (without `--force`), no response from the agent, or a port `traps listen` cannot open |

For a CI job that should also fail on warnings, use
`ferosnmp lint --fail-on warning mibs/`.

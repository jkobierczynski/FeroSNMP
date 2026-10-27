# MIB test corpus

Real-world MIB modules used to test the loader and linter, and to compare
results against libsmi's `smilint`.

Only files whose licence allows redistribution are committed. Everything else
is listed in [`manifest.tsv`](manifest.tsv) with a pinned download URL and a
SHA-256 checksum, and is fetched into `fetched/` (git-ignored) on demand:

```sh
corpus/fetch.sh            # download what is missing, verify everything
corpus/fetch.sh --verify   # verify only
```

On Windows, run it from Git Bash. Most vendor modules import the core IETF
modules (`SNMPv2-SMI`, `IF-MIB`, ...), which also live under `fetched/`, so run
the script before using the corpus.

## Layout

| Path | Contents | Committed |
|------|----------|-----------|
| `ietf/` | IETF modules whose text carries the IETF Trust BSD licence grant | yes |
| `fetched/ietf/`, `fetched/iana/`, `fetched/ieee/` | Standard modules without a redistribution licence (pre-2008 RFCs, IANA, IEEE) | no |
| `fetched/<vendor>/` | Cisco, Juniper, Fortinet, Palo Alto Networks, Siemens, Hirschmann, Moxa | no |

## Manifest columns

`manifest.tsv` has one row per file, tab separated:

- `path`: location relative to `corpus/`
- `modules`: module names defined in the file (some vendor files hold several)
- `group`: `ietf`, `iana`, `ieee` or the vendor
- `redistributable`: `yes` if the file is committed here
- `licence`: the licence basis for that decision
- `origin`: the RFC, or the vendor's official download page
- `source_url`: pinned URL that `fetch.sh` downloads from
- `sha256`: checksum of the exact bytes

Files are kept byte for byte as upstream ships them, including CRLF line
endings and odd file extensions, because those are part of what the loader has
to handle. `.gitattributes` turns off line-ending conversion for that reason.

## Licence policy

- **IETF, committed.** Modules from RFCs published after 10 November 2008 state
  in their own text that redistribution is permitted under the Simplified or
  Revised BSD licence (IETF Trust Legal Provisions, section 4.c). That licence
  is compatible with GPL-3.0.
- **IETF, fetched.** Older RFCs carry the Internet Society notice, which allows
  copying the RFC but not modified or derived works, and the IETF Trust can only
  grant the BSD licence where contributors gave it those rights. Debian ships these through
  `snmp-mibs-downloader` in non-free for the same reason.
- **IANA and IEEE, fetched.** No licence is stated in the modules.
- **Vendors, fetched.** Every vendor module is either marked "All rights
  reserved" (Cisco, Juniper, Moxa, Siemens) or carries no licence at all
  (Fortinet, Palo Alto Networks, Hirschmann). None grants redistribution, and
  the vendors' download pages do not either.

## Sources

Downloads are pinned to a commit so the checksums stay valid:

- [`cisco/cisco-mibs`](https://github.com/cisco/cisco-mibs) at `c9ab98c`:
  Cisco's own public MIB repository, and many standard modules.
- [`net-snmp/net-snmp`](https://github.com/net-snmp/net-snmp) at `009177f`:
  the core IETF and IANA modules.
- [`librenms/librenms`](https://github.com/librenms/librenms) at `aae46f0`:
  a mirror of Juniper, Fortinet, Palo Alto Networks, Siemens, Hirschmann and
  Moxa modules. These vendors publish MIBs only as release archives or behind
  support-portal logins, so the `origin` column links the official page for
  anyone who wants to fetch newer versions by hand.

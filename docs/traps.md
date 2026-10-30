# Receiving traps with FeroSNMP

`ferosnmp traps listen` receives SNMP notifications and prints each one
decoded with the loaded MIBs:

| | Supported |
|---|---|
| SNMPv1 | Trap-PDU, translated to its SNMPv2 notification (RFC 3584): a generic trap becomes `coldStart`, `linkDown` ...; an enterprise-specific one becomes `enterprise.0.specific`, which is how an SMIv1 `TRAP-TYPE` is named |
| SNMPv2c | Traps and INFORMs; INFORMs are acknowledged |
| SNMPv3 | Traps and INFORMs with every authentication and privacy protocol of [docs/snmpv3.md](snmpv3.md); INFORMs are acknowledged |
| Output | Text, or JSON Lines (`--format json`, one object per notification) |
| Filters | `--from ADDRESS`, `--notification PATTERN` |

```text
2026-10-29T21:05:00.123Z 192.168.1.20:41003 SNMPv2c trap, community "public"
  IF-MIB::linkDown, uptime 3 days, 4:05:06.07
  IF-MIB::ifIndex.2 = INTEGER: 2
  IF-MIB::ifAdminStatus.2 = INTEGER: up(1)
  IF-MIB::ifOperStatus.2 = INTEGER: down(2)
```

The time is UTC. A notification or object that no loaded MIB defines is
shown with its numeric OID and a `hint:` line that says which vendor's MIB
to add (for OIDs under `1.3.6.1.4.1`, the enterprise number). INFORMs are
marked `(acknowledged)`. Datagrams that are not notifications, or SNMPv3
messages that fail a check (unknown user, wrong password), are reported on
standard error with a hint; `-q` hides them.

The window has the same receiver in its **Traps** tab, with filters and
CSV or JSON export; see [the GUI guide](../apps/gui/README.md#traps). Port
162, the firewall and the test traps below are the same for both: in the
window, set the port and SNMPv3 users in **Traps > Trap settings** instead
of on the command line, and press **Start listening** (**Ctrl+L**).

The rest of this page sets up the receiver on Windows and sends test traps
from a Debian machine, first by hand with `snmptrap`, then from snmpd
itself.

## 1. Start the receiver on Windows

In PowerShell, with `ferosnmp.exe` from the release zip:

```powershell
.\ferosnmp.exe traps listen
```

It says `listening for traps and INFORMs on UDP 0.0.0.0:162` and prints
each notification as it arrives; **Ctrl+C** stops it. Load MIBs as for the
other commands (`-M C:\MIBs`, or `FEROSNMP_MIBDIRS`), so the names come out.

### Port 162 is in use

If it says `cannot listen on UDP 0.0.0.0:162` and names the **SNMP Trap
service**: Windows has its own trap receiver, the optional *SNMP Trap*
service (`SNMPTRAP`), and while it runs it holds port 162. Check and stop it
in a PowerShell **run as administrator**:

```powershell
Get-Service SNMPTRAP
Stop-Service SNMPTRAP
Set-Service SNMPTRAP -StartupType Disabled   # keep it stopped after a restart
```

Or leave it alone and use another port, such as 1162, for testing; then
send the traps to that port (`WINDOWS:1162` below):

```powershell
.\ferosnmp.exe traps listen --port 1162
```

### Windows Firewall

Windows Firewall blocks incoming UDP by default. The first time
`ferosnmp.exe` listens, Windows may ask whether to allow it; allow it on
**private networks**. Without that prompt, or to be precise about it, add a
rule in a PowerShell run as administrator. This one lets only the Debian
machine (`192.168.1.20`; use its address) send to port 162:

```powershell
New-NetFirewallRule -DisplayName "FeroSNMP traps" -Direction Inbound `
  -Protocol UDP -LocalPort 162 -RemoteAddress 192.168.1.20 -Action Allow
```

Use `-LocalPort 162,1162` for both ports. Check it with
`Get-NetFirewallRule -DisplayName "FeroSNMP traps"`, and remove it again
with `Remove-NetFirewallRule -DisplayName "FeroSNMP traps"`.

If traps still do not arrive, check that the network is not set to
*Public* (`Get-NetConnectionProfile`): a prompt answered for private
networks does not cover public ones. The rule above has no `-Profile`, so it
applies to all of them.

The traps go to the Windows machine's address, which `ipconfig` shows
(the *IPv4 Address* of the network the Debian machine is on). Below it is
written `WINDOWS`.

## 2. Send test traps from Debian with snmptrap

On the Debian machine, install net-snmp's command-line tools:

```sh
sudo apt install snmp
```

Debian ships these without MIBs, so the commands below use numeric OIDs.
`''` as the uptime means "my own uptime". Each command sends one
notification; the receiver prints it at once.

```sh
# SNMPv2c trap: coldStart
snmptrap -v2c -c public WINDOWS '' 1.3.6.1.6.3.1.1.5.1

# SNMPv2c trap: linkDown for interface 2, with ifIndex, ifAdminStatus, ifOperStatus
snmptrap -v2c -c public WINDOWS '' 1.3.6.1.6.3.1.1.5.3 \
  1.3.6.1.2.1.2.2.1.1.2 i 2  1.3.6.1.2.1.2.2.1.7.2 i 1  1.3.6.1.2.1.2.2.1.8.2 i 2

# SNMPv1 trap, enterprise-specific trap 3 of enterprise 1.3.6.1.4.1.8072.9999.3
# ('' as agent address: the sender's own)
snmptrap -v1 -c public WINDOWS 1.3.6.1.4.1.8072.9999.3 '' 6 3 '' \
  1.3.6.1.4.1.8072.9999.3.1 s "disk full"

# SNMPv2c INFORM: snmpinform waits for the acknowledgement and says
# "Timeout" if none comes
snmpinform -v2c -c public WINDOWS '' 1.3.6.1.6.3.1.1.5.1
```

Append `:1162` to `WINDOWS` if the receiver listens on port 1162.

The test MIBs in the repository name the `1.3.6.1.4.1.8072.9999` OIDs:
start the receiver with `-M crates\trap-rx\tests\mibs` in a checkout to see
`FEROSNMP-V1TRAP-TEST-MIB::ftvAlarm` instead of numbers.

### SNMPv3

SNMPv3 needs the same user, protocols and passwords on both sides. Start
the receiver with the user (or put it in a users file, see below):

```powershell
.\ferosnmp.exe traps listen -u trapuser -a SHA -A "my-auth-password" -x AES -X "my-priv-password"
```

Then from Debian:

```sh
# SNMPv3 trap. For a trap the sender's engine is the authoritative one; -e
# chooses its engine ID (any 5 to 32 bytes, starting with 80 by convention).
snmptrap -v3 -e 0x8000000001020304 -u trapuser -l authPriv \
  -a SHA -A "my-auth-password" -x AES -X "my-priv-password" \
  WINDOWS '' 1.3.6.1.6.3.1.1.5.1

# SNMPv3 INFORM. For an INFORM the receiver is the authoritative engine:
# snmpinform first discovers the receiver's engine ID (the one the receiver
# prints when it starts), then sends the INFORM. No -e needed.
snmpinform -v3 -u trapuser -l authPriv \
  -a SHA -A "my-auth-password" -x AES -X "my-priv-password" \
  WINDOWS '' 1.3.6.1.6.3.1.1.5.1
```

A user given without `-e` is accepted from any sender that knows the
passwords. net-snmp's own snmptrapd requires the sender's engine ID; to
bind the user to one sender the same way, start the receiver with
`-e 0x8000000001020304` as well. A sender that uses another engine ID is
then refused with `the trap user with this name is bound to another engine
ID`.

With a wrong password, `snmptrap` gets no answer (traps are never
answered), so look at the receiver: it prints `wrong authentication code`.
`snmpinform` gets a Report back and says `Authentication failure`.

## 3. Let snmpd send its own traps

snmpd sends notifications when something happens: `coldStart` when it
starts, `authenticationFailure` for a wrong community (with `authtrapenable
1`), and more with its monitoring features. Add the receiver to
`/etc/snmp/snmpd.conf`:

```text
# SNMPv1 trap, SNMPv2c trap and SNMPv2c INFORM, community "public"
trapsink   WINDOWS public
trap2sink  WINDOWS public
informsink WINDOWS public
# Send authenticationFailure for requests with a wrong community
authtrapenable 1
```

(`WINDOWS:1162` for the other port.) Restart snmpd; the receiver gets a
`coldStart` of each kind:

```sh
sudo systemctl restart snmpd
snmpget -v2c -c wrong-community localhost sysDescr.0    # sends authenticationFailure
```

### SNMPv3 from snmpd

`trapsess` sends SNMPv3 notifications; `-Ci` makes them INFORMs:

```text
trapsess -v3 -u trapuser -l authPriv -a SHA -A "my-auth-password" -x AES -X "my-priv-password" WINDOWS
trapsess -Ci -v3 -u trapuser -l authPriv -a SHA -A "my-auth-password" -x AES -X "my-priv-password" WINDOWS
```

For the **traps** (without `-Ci`), snmpd is the authoritative engine and
takes the user from its own user table, so `trapuser` must also exist in
snmpd itself, or snmpd logs `send_trap: USM unknown security name (no such
user exists)`. Add it as in [docs/snmpv3.md](snmpv3.md), step 2 (with
snmpd stopped, a `createUser` line in `/var/lib/snmp/snmpd.conf`):

```sh
sudo systemctl stop snmpd
echo 'createUser trapuser SHA "my-auth-password" AES "my-priv-password"' \
  | sudo tee -a /var/lib/snmp/snmpd.conf
sudo systemctl start snmpd
```

The **INFORMs** need no such user: snmpd discovers the receiver's engine ID
and localizes the keys for it.

The traps carry snmpd's engine ID, which `ferosnmp discover debian` shows;
use it with `-e` (or `createUser -e` in a users file) to accept the user
only from this snmpd.

## Trap users in a file

For more than one SNMPv3 user, write them as `createUser` lines, as in
net-snmp's `snmptrapd.conf`, and pass the file with `--users` (repeat it for
more files):

```text
# traps-users.conf
createUser -e 0x8000000001020304 trapuser SHA "my-auth-password" AES "my-priv-password"
createUser switches SHA-256 "another-password" AES-256 "and-another-one"
createUser monitor
```

```powershell
.\ferosnmp.exe traps listen --users traps-users.conf
```

`createUser [-e ENGINEID] NAME [AUTH PASSWORD [PRIV [PASSWORD]]]`: without
a privacy password, the authentication password is used, as in net-snmp.
Every other line (`authUser`, `disableAuthorization`, ...) is skipped, so an
existing `snmptrapd.conf` can be given as it is. Keep the file private: it
holds passwords.

A user with an authentication password accepts only authenticated
notifications (`authNoPriv`, or `authPriv` if it has privacy); a user
without one accepts only `noAuthNoPriv`.

## Options

| Option | Default | Meaning |
|--------|---------|---------|
| `-p`, `--port N` | `162` | UDP port |
| `--address ADDRESS` | `0.0.0.0` | Address to listen on; `::` for IPv6 |
| `--format text\|json` | `text` | JSON Lines: one JSON object per notification, for scripts and export |
| `--count N` | | Stop after N notifications |
| `--from ADDRESS` | | Only notifications from this sender |
| `--notification PATTERN` | | Only notifications whose name matches (`link*`, `IF-MIB::*`), or whose OID is under a numeric OID |
| `-n` | | Numeric OIDs |
| `-q` | | Do not report invalid or refused datagrams |
| `--users FILE` | | SNMPv3 trap users as `createUser` lines |
| `-u`, `-a`, `-A`, `-x`, `-X` | | One SNMPv3 trap user, as in net-snmp's tools |
| `-e HEX` | any sender | The engine ID that user's traps must come from |
| `--local-engine-id HEX` | new each start | The receiver's own engine ID, which SNMPv3 INFORMs are sent to |

To save notifications to a file, redirect the output:
`.\ferosnmp.exe traps listen --format json > traps.jsonl`.

## On Linux

Ports below 1024 need extra rights. Either allow ferosnmp to use them once
(again after each update of the program):

```sh
sudo setcap cap_net_bind_service=+ep "$(command -v ferosnmp)"
```

or listen on port 1162. If port 162 is in use, it is usually snmptrapd:
`sudo systemctl stop snmptrapd`.

## When it does not work

| What you see | Cause | Fix |
|---|---|---|
| `cannot listen on UDP 0.0.0.0:162`, names the SNMP Trap service | Windows' SNMPTRAP service holds the port | Stop it (step 1), or use `--port 1162` |
| `cannot listen ...: permission denied` (Linux) | Port below 1024 | `setcap`, or `--port 1162` |
| Nothing arrives | Firewall, wrong address, or the wrong port | The firewall rule (step 1); `ipconfig` for the address; same port on both sides |
| `snmpinform` says `Timeout` | The INFORM did not arrive, or the answer did not get back | As above; the receiver acknowledges every INFORM it accepts |
| `no trap user has this name` | The SNMPv3 user is not configured on the receiver | `-u ...`, or a `createUser` line in a users file |
| `wrong authentication code` | Authentication password or protocol differ | Both must match; `SHA` is SHA-1 |
| `cannot decrypt` | Privacy password or protocol differ | Both must match |
| `bound to another engine ID` | The user was given with `-e`, and the sender uses another engine ID | Check the sender's engine ID (`ferosnmp discover`), or leave out `-e` |
| `the security level does not match` | For example an `authPriv` trap for a user without a privacy password, or `noAuthNoPriv` for a user with a password | Make the levels match |
| `send_trap: USM unknown security name` in snmpd's log | snmpd has no such user for its SNMPv3 traps | Add the user to snmpd too (step 3) |
| A name like `SNMPv2-SMI::enterprises.9.9.41.2.0.1` and a `hint:` | No loaded MIB defines it | Add the vendor's MIB to the MIB folders (`-M`) |

The SNMPv3 security checks follow RFC 3414, with one difference: for traps
the sender's engine time is not checked (FeroSNMP does not track the time of
every sender), so a recorded trap sent again is accepted again. INFORMs are
checked against the receiver's own time.

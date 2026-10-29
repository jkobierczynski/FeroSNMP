# SNMPv3 with FeroSNMP

FeroSNMP speaks SNMPv3 with the User-based Security Model (USM, RFC 3414):

| | Supported |
|---|---|
| Security levels | `noAuthNoPriv`, `authNoPriv`, `authPriv` |
| Authentication | MD5, SHA (SHA-1), SHA-224, SHA-256, SHA-384, SHA-512 (RFC 3414, RFC 7860) |
| Privacy | DES, AES (AES-128, RFC 3826), AES-192, AES-256, and AES-192C / AES-256C (Cisco key extension) |
| Context | context name; context engine ID (for proxies) |

All crypto is pure Rust (the RustCrypto crates); there is no OpenSSL to
install.

## Try it against a Debian snmpd

These steps add one SNMPv3 user to net-snmp's `snmpd` on Debian or Ubuntu
and query it from Windows. Replace the passwords with your own; each must be
at least 8 characters.

### 1. Make snmpd listen on the network

Debian's `/etc/snmp/snmpd.conf` listens on localhost only. If you already
query this snmpd with SNMPv2c from Windows, this is done. Otherwise change
the `agentaddress` line to:

```text
agentaddress udp:161,udp6:[::]:161
```

### 2. Add the user

Stop snmpd first: it rewrites its user file when it stops.

```sh
sudo systemctl stop snmpd
```

Add the user's password line to the **persistent** file
`/var/lib/snmp/snmpd.conf` (create it if it does not exist):

```sh
echo 'createUser ferosnmp SHA-256 "my-auth-password" AES "my-priv-password"' \
  | sudo tee -a /var/lib/snmp/snmpd.conf
```

Let the user read everything, at `authPriv` only, by adding this line to
`/etc/snmp/snmpd.conf`:

```text
rouser ferosnmp priv
```

(`rouser NAME auth` would allow `authNoPriv` too; `rouser NAME noauth`
anything.) Then start snmpd again:

```sh
sudo systemctl start snmpd
```

When snmpd starts, it replaces the `createUser` line with a `usmUser` line
that holds keys instead of the passwords. To change a password later, stop
snmpd, delete that user's `usmUser` line, and add a new `createUser` line.

`net-snmp-create-v3-user` (in the `libsnmp-dev` package) does the same
two steps in one go:

```sh
sudo systemctl stop snmpd
sudo net-snmp-create-v3-user -ro -a SHA-256 -A "my-auth-password" -x AES -X "my-priv-password" ferosnmp
sudo systemctl start snmpd
```

Other protocols: `MD5`, `SHA`, `SHA-224`, `SHA-384`, `SHA-512`, and `DES`,
`AES-192`, `AES-256` for privacy. If `journalctl -u snmpd` says `unknown
privProtocol`, that snmpd was built without it; use `AES`.

Check on the Debian machine itself with net-snmp's own tool (package
`snmp`):

```sh
snmpget -v3 -l authPriv -u ferosnmp -a SHA-256 -A "my-auth-password" \
  -x AES -X "my-priv-password" localhost sysDescr.0
```

### 3. Query it from Windows (command line)

In PowerShell, with `ferosnmp.exe` from the release zip and `debian` being
the host name or address of the Debian machine:

```powershell
.\ferosnmp.exe discover debian
.\ferosnmp.exe get debian -u ferosnmp -a SHA-256 -A "my-auth-password" -x AES -X "my-priv-password" sysDescr.0
.\ferosnmp.exe walk debian -u ferosnmp -a SHA-256 -A "my-auth-password" -x AES -X "my-priv-password" system --log
```

`discover` needs no user: it shows the agent's engine ID, how often it
restarted (boots) and how long it has been up (time). With `-u`, the version
is SNMPv3 without `-v 3`. The options are named as in net-snmp's tools;
`ferosnmp get --help` lists them.

To keep the passwords out of the PowerShell history, set them as
environment variables instead of `-A` and `-X`:

```powershell
$env:FEROSNMP_AUTH_PASSWORD = "my-auth-password"
$env:FEROSNMP_PRIV_PASSWORD = "my-priv-password"
.\ferosnmp.exe get debian -u ferosnmp -a SHA-256 -x AES sysDescr.0
```

The security level follows from the passwords given (both: `authPriv`;
only `-A`: `authNoPriv`; none: `noAuthNoPriv`); `-l` sets it explicitly.
An empty privacy password means the authentication password, as in
net-snmp.

### 4. Query it from the window

1. **File > Agents**, **New**. Name it, enter the host, and set
   **Version** to **SNMPv3**.
2. On the right, fill in **User** (`ferosnmp`), **Security level**
   (`authPriv`), **Auth protocol** (`SHA-256`), **Auth password**,
   **Privacy protocol** (`AES-128`) and **Privacy password**. **OK**.
3. Pick the agent in the **Agent** box and press **Walk**. The **Log** tab
   shows the discovery and the engine ID, boots and uptime.

The passwords are stored in the **Windows Credential Manager** (Control
Panel > Credential Manager > Windows Credentials, entries named
`FeroSNMP/<agent>/auth` and `FeroSNMP/<agent>/priv`), never in
`settings.toml`.

## When it does not work

Each failure has its own message. The usual causes:

| Message says | Cause | Fix |
|---|---|---|
| does not know SNMPv3 user (usmStatsUnknownUserNames) | No such user on the agent | Check the spelling (case matters); add the user (step 2) |
| authentication failed (usmStatsWrongDigests) | Wrong auth password, or wrong auth protocol | Both must match; `SHA` is SHA-1, not SHA-256 |
| the privacy password or protocol differs | Wrong privacy password or protocol | net-snmp drops what it cannot decrypt, so FeroSNMP asks again without privacy to find this out |
| not at security level authPriv (usmStatsUnsupportedSecLevels) | The user has no privacy password on the agent | Use `authNoPriv`, or add privacy to the user |
| authorizationError | The access rules do not allow this user at this level | `rouser NAME priv` allows `authPriv` only |
| outside its time window (usmStatsNotInTimeWindows) | The agent's clock data is broken (FeroSNMP resynchronizes once by itself) | Restart the agent; check that no two devices share an engine ID |
| no response | Wrong address or port, firewall, or snmpd not listening on the network | Step 1; `discover` tells whether the agent answers at all |

### AES-192 and AES-256 on Cisco devices

AES-192 and AES-256 need longer keys than MD5 and SHA-1 produce, and there
are two ways to extend them. Most agents (net-snmp, and FeroSNMP's
`AES-192`/`AES-256`) use the one from draft-blumenthal-aes-usm-04; Cisco IOS
uses the one from draft-reeder-snmpv3-usm-3desede, which FeroSNMP offers as
`AES-192C`/`AES-256C` (in the window: *AES-192 (Cisco)*). With SHA-256 or
longer hashes the two give the same key. If a Cisco device rejects AES-256
with a privacy error, try `AES-256C`.

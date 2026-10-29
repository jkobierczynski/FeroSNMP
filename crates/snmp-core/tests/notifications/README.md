# Recorded notifications

Datagrams sent by net-snmp 5.9.4 (`snmptrap` and `snmpinform`), one per
file in hex, for the unit tests in `src/tests_notify.rs`. They were recorded
with a UDP socket on 127.0.0.1:11162 and these commands (`MIBS=` so that
net-snmp loads no MIBs):

```sh
H=127.0.0.1:11162
E=0x80001f88040102030405     # engine ID of the SNMPv3 sender
snmptrap -v1 -c public $H 1.3.6.1.4.1.8072.9999.3 192.0.2.10 6 3 12345 1.3.6.1.4.1.8072.9999.3.1 s "disk full"    # v1-enterprise-specific
snmptrap -v1 -c public $H 1.3.6.1.4.1.8072.9999.3 192.0.2.10 2 0 500 1.3.6.1.2.1.2.2.1.1.2 i 2                    # v1-link-down
snmptrap -v2c -c public $H 4242 1.3.6.1.4.1.8072.9999.2.0.1 1.3.6.1.4.1.8072.9999.2.1.1 s eth0 1.3.6.1.4.1.8072.9999.2.1.2 i 2   # v2c-port-change
snmptrap -v2c -c s3cret $H 100 1.3.6.1.4.1.9.9.41.2.0.1 1.3.6.1.4.1.9.9.41.1.2.3.1.2.7 s LINK-3-UPDOWN \
    1.3.6.1.4.1.8072.9999.2.1.4 C 18446744073709551615 1.3.6.1.4.1.8072.9999.2.1.5 a 192.0.2.77                    # v2c-vendor
snmpinform -t 1 -r 0 -v2c -c public $H 4242 1.3.6.1.4.1.8072.9999.2.0.2 1.3.6.1.4.1.8072.9999.2.1.3 i 85            # v2c-inform
# The SNMPv3 traps all carry 1.3.6.1.4.1.8072.9999.2.0.2 with ...2.1.3 i 85,
# passwords ferosnmp-auth and ferosnmp-priv:
snmptrap -v3 -e $E -u trap-sha-aes -l authPriv -a SHA -A ferosnmp-auth -x AES -X ferosnmp-priv $H 4242 ...        # v3-sha-aes
snmptrap -v3 -e $E -u trap-md5-des -l authPriv -a MD5 -A ferosnmp-auth -x DES -X ferosnmp-priv $H 4242 ...        # v3-md5-des
snmptrap -v3 -e $E -u trap-sha256 -l authNoPriv -a SHA-256 -A ferosnmp-auth $H 4242 ...                            # v3-sha256-none
snmptrap -v3 -e $E -u trap-noauth -l noAuthNoPriv $H 4242 ...                                                     # v3-noauth
snmptrap -v3 -e $E -u trap-sha512-aes256 -l authPriv -a SHA-512 -A ferosnmp-auth -x AES-256 -X ferosnmp-priv $H 4242 ...   # v3-sha512-aes256
# The first message of an SNMPv3 INFORM: engine discovery.
snmpinform -t 1 -r 0 -v3 -u trap-sha-aes -l authPriv -a SHA -A ferosnmp-auth -x AES -X ferosnmp-priv $H 4242 ...   # v3-inform-discovery
```

The whole SNMPv3 INFORM exchange is tested against net-snmp itself in
`apps/cli/tests/traps.rs`.

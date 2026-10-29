#!/usr/bin/env python3
"""Serve FEROSNMP-TEST-MIB to net-snmp snmpd through `pass_persist`.

snmpd starts this script once and sends it requests on stdin:
"PING", or "get"/"getnext"/"set" followed by an OID on the next line. Every
value is fixed, so walks of the test agent give the same output every time.
See snmpd.conf(5), section "pass_persist".
"""

import sys

BASE = (1, 3, 6, 1, 4, 1, 8072, 9999, 1)


def oid(*arcs):
    return BASE + arcs


def build():
    objects = {
        oid(1, 1, 0): ("string", "FeroSNMP test agent"),
        oid(1, 2, 0): ("octet", "00 01 7f 80 ff"),
        oid(1, 3, 0): ("integer", "3"),
        oid(1, 4, 0): ("integer", "-125"),
        oid(1, 5, 0): ("counter64", "18446744073709551615"),
        oid(1, 6, 0): ("timeticks", "123456789"),
        oid(1, 7, 0): ("ipaddress", "192.0.2.17"),
        oid(1, 8, 0): ("objectid", ".1.3.6.1.4.1.8072.9999.1.2"),
        oid(1, 9, 0): ("octet", "00 1b 21 0a ff 09"),
        # 2026-10-29,13:45:30.5,+1:0
        oid(1, 10, 0): ("octet", "07 ea 0a 1d 0d 2d 1e 05 2b 01 00"),
        oid(1, 11, 0): ("gauge", "4294967295"),
        oid(1, 12, 0): ("counter", "42"),
        # alpha(0), gamma(2) and delta(9)
        oid(1, 13, 0): ("octet", "a0 40"),
        oid(1, 14, 0): ("unsigned", "7"),
    }
    # Enough rows that a walk needs many GETBULK and GETNEXT requests.
    for row in range(1, 151):
        objects[oid(2, 1, 2, row)] = ("string", f"row {row}")
        objects[oid(2, 1, 3, row)] = ("counter", str(row * 1000))
    return objects


OBJECTS = build()
ORDER = sorted(OBJECTS)


def parse(text):
    try:
        return tuple(int(arc) for arc in text.strip().strip(".").split("."))
    except ValueError:
        return None


def answer(key):
    kind, value = OBJECTS[key]
    return "." + ".".join(map(str, key)) + "\n" + kind + "\n" + value + "\n"


def main():
    while True:
        command = sys.stdin.readline()
        if not command:
            return
        command = command.strip()
        if command == "PING":
            reply = "PONG\n"
        elif command in ("get", "getnext"):
            key = parse(sys.stdin.readline())
            reply = "NONE\n"
            if key is not None:
                if command == "get":
                    if key in OBJECTS:
                        reply = answer(key)
                else:
                    later = next((k for k in ORDER if k > key), None)
                    if later is not None:
                        reply = answer(later)
        elif command == "set":
            sys.stdin.readline()
            sys.stdin.readline()
            reply = "not-writable\n"
        else:
            reply = "NONE\n"
        sys.stdout.write(reply)
        sys.stdout.flush()


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Serve FEROSNMP-TEST-MIB to net-snmp snmpd through `pass_persist`.

snmpd starts this script once and sends it requests on stdin:
"PING", or "get"/"getnext"/"set" followed by an OID on the next line ("set" also
by a line with the type and the new value). Every value outside fstSettings
is fixed, so walks of the test agent give the same output every time;
fstSettings can be written, with checks like a real agent's, and starts
with the same values each time snmpd starts. See snmpd.conf(5), section
"pass_persist".
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
    objects.update(SETTINGS)
    # fstPortTable and fstPortXTable: INDEX { slot, address, name }.
    ports = [
        (1, "192.0.2.1", "eth0", "00 1b 21 0a ff 01", 1000, 1, "uplink", 0),
        (1, "192.0.2.1", "eth1", "00 1b 21 0a ff 02", 100, 2, "", 17),
        (2, "198.51.100.7", "mgmt", "00 1b 21 0a ff 03", 10, 1, "management", 3),
        (16, "10.0.0.1", "", "02 00 00 00 00 10", 0, 2, "no name", 0),
    ]
    for slot, address, name, mac, speed, status, alias, errors in ports:
        index = (slot, *map(int, address.split(".")), *string_index(name))
        objects[oid(4, 1, 4, *index)] = ("octet", mac)
        objects[oid(4, 1, 5, *index)] = ("gauge", str(speed))
        objects[oid(4, 1, 6, *index)] = ("integer", str(status))
        objects[oid(5, 1, 1, *index)] = ("string", alias)
        objects[oid(5, 1, 2, *index)] = ("counter", str(errors))
    # fstUserTable: INDEX { IMPLIED name }, so no length before the bytes.
    for name, group, logins in [("admin", "wheel", 12), ("guest", "users", 0), ("operator", "ops", 7)]:
        index = tuple(name.encode())
        objects[oid(6, 1, 2, *index)] = ("string", group)
        objects[oid(6, 1, 3, *index)] = ("counter", str(logins))
    # fstPolicyTable: INDEX { family, IMPLIED id }.
    for family, policy, hits in [(1, (1, 3, 6, 1, 4, 1, 8072, 9999, 1, 7), 5), (2, (0, 0), 2**40)]:
        objects[oid(7, 1, 3, family, *policy)] = ("counter64", str(hits))
    return objects


def string_index(text):
    """A string in an INDEX that is not IMPLIED: its length, then its bytes."""
    data = text.encode()
    return (len(data), *data)


# Writable objects: OID -> (type, value, check). The check gets the value
# as snmpd passes it and returns the value to store, or an error name.
def check_contact(value):
    text = unquote(value)
    return ("wrong-length", None) if len(text) > 32 else (None, text)


def check_range(low, high):
    def check(value):
        try:
            number = int(value)
        except ValueError:
            return ("wrong-type", None)
        return ("wrong-value", None) if not low <= number <= high else (None, str(number))
    return check


def check_mac(value):
    data = unquote(value)
    return ("wrong-length", None) if len(data.split()) != 6 else (None, data)


def check_any(value):
    return (None, unquote(value))


def unquote(value):
    if len(value) >= 2 and value[0] == value[-1] == '"':
        return value[1:-1]
    return value


SETTINGS = {
    oid(3, 1, 0): ("string", "nobody"),
    oid(3, 2, 0): ("integer", "1"),
    oid(3, 3, 0): ("integer", "50"),
    oid(3, 4, 0): ("ipaddress", "192.0.2.254"),
    oid(3, 5, 0): ("octet", "00 1b 21 0a ff 09"),
    oid(3, 6, 0): ("octet", "00"),
    oid(3, 7, 0): ("objectid", ".1.3.6.1.4.1.8072.9999.1"),
    oid(3, 8, 0): ("gauge", "10"),
}
CHECKS = {
    oid(3, 1, 0): ("string", check_contact),
    oid(3, 2, 0): ("integer", check_range(1, 3)),
    oid(3, 3, 0): ("integer", check_range(0, 100)),
    oid(3, 4, 0): ("ipaddress", check_any),
    oid(3, 5, 0): ("octet", check_mac),
    oid(3, 6, 0): ("octet", check_any),
    oid(3, 7, 0): ("objectid", check_any),
    oid(3, 8, 0): ("gauge", check_range(1, 1000)),
}


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


def set_value(key, line):
    """Write a setting; answer DONE or the SNMP error name."""
    if key not in CHECKS:
        return "not-writable\n"
    kind, value = (line.split(" ", 1) + [""])[:2]
    expected, check = CHECKS[key]
    # snmpd says "string" or "octet" for an OCTET STRING, depending on
    # whether the bytes are printable.
    if kind != expected and not {kind, expected} <= {"string", "octet"}:
        return "wrong-type\n"
    error, stored = check(value)
    if error:
        return error + "\n"
    OBJECTS[key] = (kind, stored)
    return "DONE\n"


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
            key = parse(sys.stdin.readline())
            reply = set_value(key, sys.stdin.readline().rstrip("\n"))
        else:
            reply = "NONE\n"
        sys.stdout.write(reply)
        sys.stdout.flush()


if __name__ == "__main__":
    main()

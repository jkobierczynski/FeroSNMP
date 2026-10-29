#!/bin/sh
# Start net-snmp snmpd with the test configuration on 127.0.0.1:11161.
#
#   crates/snmp-core/tests/agent/run-agent.sh          # start in the background
#   crates/snmp-core/tests/agent/run-agent.sh stop     # stop it again
#
# Needs snmpd (Debian and Ubuntu: `sudo apt install snmpd`) and python3. Runs
# as a normal user; state goes to a temporary folder.
set -eu

dir=$(cd "$(dirname "$0")" && pwd)
state=${TMPDIR:-/tmp}/ferosnmp-test-agent
pidfile=$state/snmpd.pid

if [ -f "$pidfile" ]; then
    kill "$(cat "$pidfile")" 2>/dev/null || true
    rm -f "$pidfile"
fi
if [ "${1:-}" = stop ]; then
    exit 0
fi

mkdir -p "$state"
sed "s|@AGENT_DIR@|$dir|" "$dir/snmpd.conf" > "$state/snmpd.conf"
# -C: read only our file; -Lf: log to a file; -p: write the process id.
MIBS= SNMP_PERSISTENT_DIR=$state snmpd -C -c "$state/snmpd.conf" -Lf "$state/snmpd.log" -p "$pidfile"

# Wait until it answers.
for _ in 1 2 3 4 5 6 7 8 9 10; do
    if [ -f "$pidfile" ] && grep -q "NET-SNMP version" "$state/snmpd.log" 2>/dev/null; then
        echo "test agent listening on 127.0.0.1:11161 (log: $state/snmpd.log)"
        exit 0
    fi
    sleep 0.5
done
echo "snmpd did not start; see $state/snmpd.log" >&2
exit 1

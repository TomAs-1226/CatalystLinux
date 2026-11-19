#!/bin/sh
# Every address this handheld has, where its traffic goes, and who answers
# its name lookups.

echo "addresses"
ip -4 -o addr show | awk '$2 != "lo" { printf "  %-6s %s\n", $2, $4 }'
echo "routes"
ip route | sed 's/^/  /'
echo "name servers"
grep '^nameserver' /etc/resolv.conf 2>/dev/null | sed 's/^nameserver/ /' || echo "  none"

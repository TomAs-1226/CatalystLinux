#!/bin/sh
# Is the robot there? Its radio and its controller, three pings each, and how
# long they take to answer. The team comes from the handheld's robot setting.

input=$(cat /data/catalyst/robot 2>/dev/null)
case "$input" in
	'' )
		echo "No robot set: Settings › Robot, or setup."
		exit 0
		;;
	*[!0-9]* )
		hosts=$input
		;;
	* )
		te=$((input / 100))
		am=$((input % 100))
		hosts="10.$te.$am.1 10.$te.$am.2"
		echo "Team $input"
		;;
esac

for host in $hosts; do
	case "$host" in
		*.1) what="radio" ;;
		*) what="robot" ;;
	esac
	if out=$(ping -c 3 -W 1 "$host" 2>&1); then
		# "round-trip min/avg/max = 0.4/0.6/0.9 ms": the average.
		avg=$(echo "$out" | tail -1 | sed 's|.*= [^/]*/\([^/]*\)/.*|\1|')
		printf '%-6s %-14s answers in %s ms\n' "$what" "$host" "$avg"
	else
		printf '%-6s %-14s no answer\n' "$what" "$host"
	fi
done

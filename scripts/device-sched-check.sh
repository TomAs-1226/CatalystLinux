#!/bin/sh
# device-sched-check.sh: the processor's pace, the battery and the card,
# measured on the handheld itself (BusyBox sh), for before and after a
# change to the shell's scheduling (sched.rs). Run as root, from a computer:
#
#     ssh root@<handheld> sh -s -- <check> [seconds] < scripts/device-sched-check.sh
#
# or copied to the handheld and run there. Each scenario asks for what to do
# (press keys, start a song), then measures for [seconds] (default 30):
#
#   info        the clock and its governor, idle, CMA, the card, TLS, interrupts
#   rest        nothing touched, the screen on (Screen off after: 5 min or never)
#   scroll      Down held in a long list (Settings › About)
#   page        Enter, then Esc, about once a second, in Settings
#   music-dark  a song playing, the screen gone dark (music standby after 30 s)
#   model       the on-device model answering a long question
#   boost-ab    scroll with the boost off, then on (/run/catalyst-noboost)
#   tls         the TLS suite curl agrees, and processor time a MB for each
#   irq         interrupts a second by source, over 10 s
#   io          the card's latency under each I/O scheduler (catalyst-shell --sched-bench io)
#   bench       the clock and the battery (catalyst-shell --sched-bench clock, power):
#               stops the shell while it runs, on battery, about four minutes
#
# Each scenario prints the draw from the fuel gauge (mA and mW, read each
# second), the time at each clock (cpufreq stats), clock changes, how busy
# the processor was, context switches and interrupts a second (mmc1, the
# Wi-Fi's SDIO, on its own), the shell's processor time, the GPU's free
# contiguous memory, and the motions the shell drew (/run/catalyst-pacing).

check=${1:-info}
seconds=${2:-30}
POLICY=/sys/devices/system/cpu/cpufreq/policy0
PACING=/run/catalyst-pacing

gauge() {
	for d in /sys/class/power_supply/*; do
		[ -e "$d/capacity" ] && { echo "$d"; return; }
	done
}

say() { printf '%s\n' "$*"; }

snapshot() {
	# One line of numbers: busy and total jiffies, context switches,
	# interrupts, mmc1's interrupts, clock changes, the shell's ticks, CmaFree.
	awk '/^cpu / { busy = $2 + $3 + $4 + $7 + $8; total = busy + $5 + $6 } /^ctxt/ { c = $2 } /^intr/ { i = $2 }
		END { printf "%d %d %d %d", busy, total, c, i }' /proc/stat
	printf ' %d' "$(awk '/mmc1/ { for (k = 2; k <= NF; k++) if ($k ~ /^[0-9]+$/) s += $k; else break } END { print s + 0 }' /proc/interrupts)"
	printf ' %d' "$(cat $POLICY/stats/total_trans 2>/dev/null || echo 0)"
	p=$(pidof catalyst-shell | cut -d' ' -f1)
	printf ' %d' "$( [ -n "$p" ] && awk '{ print $14 + $15 }' /proc/$p/stat || echo 0)"
	printf ' %d\n' "$(awk '/CmaFree/ { print $2 }' /proc/meminfo)"
}

states() { cat $POLICY/stats/time_in_state 2>/dev/null; }

measure() {
	label=$1
	g=$(gauge)
	: >$PACING
	before=$(snapshot)
	states >/tmp/sched-check-states
	samples=""
	i=0
	while [ $i -lt "$seconds" ]; do
		sleep 1
		if [ -n "$g" ]; then
			samples="$samples $(cat $g/current_now 2>/dev/null || echo 0):$(cat $g/voltage_now 2>/dev/null || echo 0):$(cat $g/status 2>/dev/null | tr ' ' _)"
		fi
		i=$((i + 1))
	done
	after=$(snapshot)
	say "== $label · $seconds s"
	echo "$samples" | tr ' ' '\n' | awk -F: 'NF == 3 { a = $1 < 0 ? -$1 : $1; ma += a / 1000; mw += a / 1000 * $2 / 1e6; n++; s = $3 }
		END { if (n) printf "battery      %.0f mA · %.0f mW (%s, %d readings)\n", ma / n, mw / n, s, n; else print "battery      no gauge" }'
	echo "$before $after $seconds" | awk '{ s = $17; db = $9 - $1; dt = $10 - $2
		printf "processor    %.1f%% busy · %.0f switches/s · %.0f interrupts/s (mmc1 %.0f/s) · %.1f clock changes/s\n", dt ? 100 * db / dt : 0, ($11 - $3) / s, ($12 - $4) / s, ($13 - $5) / s, ($14 - $6) / s
		printf "the shell    %.1f ms of processor a second\n", ($15 - $7) * 10 / s
		printf "cma free     %d kB (was %d)\n", $16, $8 }'
	states | awk 'NR == FNR { was[$1] = $2; next } { d[$1] = $2 - was[$1]; t += d[$1] } END { if (t) { printf "clock       "; for (f in d) printf " %d MHz %.0f%%", f / 1000, 100 * d[f] / t; print "" } }' /tmp/sched-check-states -
	if [ -s $PACING ]; then
		awk '{ for (k = 2; k <= NF; k++) { split($k, kv, "="); v[kv[1]] = kv[2] }
			n++; f += v["frames"]; late += v["late"]; if (v["boost"] == "on") on++
			if (v["key"] != "-") { sub("ms", "", v["key"]); key[++nk] = v["key"] + 0 }
			sub("ms", "", v["typical"]); sub("ms", "", v["worst"]); if (v["worst"] + 0 > worst) worst = v["worst"] + 0
			if (v["typical"] != "-") typ[++nt] = v["typical"] + 0; mhz[v["mhz"]]++ }
			function median(a, m,   i, j, t) { for (i = 1; i <= m; i++) for (j = i + 1; j <= m; j++) if (a[j] < a[i]) { t = a[i]; a[i] = a[j]; a[j] = t } return m ? a[int((m + 1) / 2)] : 0 }
			END { printf "motions      %d (%d frames, %d late, boost on in %d) · key to frame median %.1f ms · typical gap %.1f ms · worst %.1f ms\n", n, f, late, on, median(key, nk), median(typ, nt), worst
				printf "clock as drawn (first>last):"; for (m in mhz) printf " %s ×%d", m, mhz[m]; print "" }' $PACING
	else
		say "motions      none drawn"
	fi
	rm -f $PACING
}

ask() {
	say "$1"
	say "(measuring starts in 5 s)"
	sleep 5
}

case "$check" in
info)
	say "kernel       $(uname -r)"
	say "governor     $(cat $POLICY/scaling_governor) of: $(cat $POLICY/scaling_available_governors)"
	say "clock        $(cat $POLICY/scaling_cur_freq) kHz now, floor $(cat $POLICY/scaling_min_freq), ceiling $(cat $POLICY/scaling_max_freq), steps: $(cat $POLICY/scaling_available_frequencies)"
	say "latency      $(cat $POLICY/cpuinfo_transition_latency) ns · schedutil rate_limit_us $(cat $POLICY/schedutil/rate_limit_us 2>/dev/null || cat /sys/devices/system/cpu/cpufreq/schedutil/rate_limit_us 2>/dev/null || echo -)"
	say "conservative ignore_nice_load $(cat /sys/devices/system/cpu/cpufreq/conservative/ignore_nice_load 2>/dev/null || echo '- (not in use)')"
	say "boost        $([ -e /run/catalyst-noboost ] && echo 'switched off (/run/catalyst-noboost)' || echo on) · floor raised now: $([ "$(cat $POLICY/scaling_min_freq)" = "$(cat $POLICY/cpuinfo_min_freq)" ] && echo no || echo yes)"
	say "idle         $(cat /sys/devices/system/cpu/cpuidle/current_driver 2>/dev/null || echo 'no cpuidle driver: WFI')"
	say "cores        online $(cat /sys/devices/system/cpu/online); cpu3 can go offline: $([ -w /sys/devices/system/cpu/cpu3/online ] && echo yes || echo no)"
	say "time at each clock since boot (10 ms units):"; states | sed 's/^/             /'
	grep -E 'CmaTotal|CmaFree' /proc/meminfo | sed 's/^/             /'
	dmesg | grep -iE 'cma|linux,cma' | head -3 | sed 's/^/  dmesg      /'
	say "card         scheduler: $(cat /sys/block/mmcblk0/queue/scheduler)"
	for t in low_latency slice_idle; do [ -e /sys/block/mmcblk0/queue/iosched/$t ] && say "             bfq $t = $(cat /sys/block/mmcblk0/queue/iosched/$t)"; done
	say "             vm.dirty_background_bytes $(cat /proc/sys/vm/dirty_background_bytes) · vm.dirty_bytes $(cat /proc/sys/vm/dirty_bytes)"
	say "shell        threads and their policy (0 normal, 3 batch, 5 idle) and nice:"
	p=$(pidof catalyst-shell | cut -d' ' -f1)
	# stat's fields after the name in brackets (names may have spaces):
	# nice is the 17th of them, the policy the 39th.
	for t in /proc/$p/task/*; do
		sed 's/.*) //' $t/stat | awk -v n="$(cat $t/comm)" '{ printf "             %-18s nice %3d policy %d\n", n, $17, $39 }'
	done
	say "others       (pid name nice policy) for the background:"
	for q in /proc/[0-9]*; do
		n=$(cat $q/comm 2>/dev/null)
		case "$n" in llama-server | curl | yt-dlp | python3 | catalyst-update | catalyst-play | tar | qjs)
			sed 's/.*) //' $q/stat | awk -v n="$n" -v p="${q#/proc/}" '{ printf "             %6s %-16s nice %3d policy %d\n", p, n, $17, $39 }' ;;
		esac
	done
	say "wi-fi        brcmfmac parameters: $(cd /sys/module/brcmfmac/parameters 2>/dev/null && for f in *; do printf '%s=%s ' "$f" "$(cat $f 2>/dev/null)"; done)"
	# brcmfmac says it as it sets it, at each connection (CONFIG_CFG80211_DEFAULT_PS=y asks for it).
	say "             $(dmesg | grep -i 'power save' | tail -1)"
	say "tls          $(curl -sv -o /dev/null https://www.youtube.com 2>&1 | grep -i 'SSL connection' | sed 's/^\* //')"
	;;
rest)
	ask "Leave it be, the screen on, nothing moving."
	measure rest
	;;
scroll)
	ask "Open Settings › About and hold Down (then Up) for the whole time."
	measure scroll
	;;
page)
	ask "In Settings: Enter on a row, then Esc, about once a second, for the whole time."
	measure "page turns"
	;;
music-dark)
	ask "Play a song in Music, then leave it: the screen goes off, and music standby comes 30 s later. Starting in 5 s, measuring for $seconds s (give it 120)."
	measure "music, screen dark"
	say "(xruns: what catalyst-play said, from the shell's log)"
	grep -c 'xrun' /tmp/catalyst-shell.log 2>/dev/null | sed 's/^/xrun lines   /'
	;;
model)
	ask "Ask the on-device model something with a long answer, and leave it answering."
	measure "model answering"
	;;
boost-ab)
	touch /run/catalyst-noboost
	ask "Boost OFF: hold Down in Settings › About for the whole time."
	measure "scroll, boost off"
	rm -f /run/catalyst-noboost
	ask "Boost ON: the same again."
	measure "scroll, boost on"
	;;
tls)
	say "agreed now:  $(curl -sv -o /dev/null https://www.youtube.com 2>&1 | grep -i 'SSL connection' | sed 's/^\* //')"
	url='https://speed.cloudflare.com/__down?bytes=30000000'
	for suite in TLS_AES_256_GCM_SHA384 TLS_AES_128_GCM_SHA256 TLS_CHACHA20_POLY1305_SHA256; do
		b=$(snapshot)
		r=$(curl -s -o /dev/null --tls13-ciphers $suite -w '%{size_download} %{time_total}' "$url")
		a=$(snapshot)
		echo "$b $a $r" | awk -v s=$suite '{ mb = $17 / 1e6; busy = ($9 - $1) / 100
			printf "%-30s %.1f MB in %.1f s · %.2f processor-seconds a MB (all four cores, interrupts included)\n", s, mb, $18, mb ? busy / mb : 0 }'
	done
	;;
irq)
	cp /proc/interrupts /tmp/sched-check-irq
	n=/sys/class/net/wlan0/statistics
	w0="$(cat $n/rx_packets $n/tx_packets $n/multicast 2>/dev/null | tr '\n' ' ')"
	sleep 10
	w1="$(cat $n/rx_packets $n/tx_packets $n/multicast 2>/dev/null | tr '\n' ' ')"
	echo "$w0 $w1" | awk 'NF == 6 { printf "wlan0        %.1f packets/s in (%.1f multicast), %.1f out\n", ($4 - $1) / 10, ($6 - $3) / 10, ($5 - $2) / 10 }'
	awk 'NR == FNR { if (FNR > 1) { s = 0; for (k = 2; k <= NF && $k ~ /^[0-9]+$/; k++) s += $k; was[$1] = s }; next }
		FNR > 1 { s = 0; for (k = 2; k <= NF && $k ~ /^[0-9]+$/; k++) s += $k; d = (s - was[$1]) / 10; if (d > 0) { $1 = $1; name = ""; for (j = k; j <= NF; j++) name = name " " $j; printf "%8.1f/s  %s%s\n", d, $1, name } }' /tmp/sched-check-irq /proc/interrupts | sort -rn | head -12
	# Can mmc1 be kept to one core? Tried, then put back as it was.
	for i in $(awk '/mmc1/ { sub(":", "", $1); print $1 }' /proc/interrupts); do
		was=$(cat /proc/irq/$i/smp_affinity_list)
		if echo 1 >/proc/irq/$i/smp_affinity_list 2>/dev/null; then
			say "irq $i (mmc1): affinity $was, and it took cpu1 alone: $(cat /proc/irq/$i/effective_affinity_list 2>/dev/null)"
			echo "$was" >/proc/irq/$i/smp_affinity_list
		else
			say "irq $i (mmc1): affinity $was, and it cannot be moved (the BCM2835's interrupt controller has no affinity; the kernel turns its routing to the next core on every interrupt)"
		fi
	done
	say "unbound workqueues (brcmf_wq, writeback) may run on: $(cat /sys/devices/virtual/workqueue/cpumask 2>/dev/null)"
	;;
io)
	was=$(sed 's/.*\[\(.*\)\].*/\1/' /sys/block/mmcblk0/queue/scheduler)
	for s in mq-deadline bfq; do
		echo $s >/sys/block/mmcblk0/queue/scheduler || continue
		if [ $s = bfq ]; then
			for ll in 1 0; do for si in 8 0; do
				echo $ll >/sys/block/mmcblk0/queue/iosched/low_latency
				echo $si >/sys/block/mmcblk0/queue/iosched/slice_idle
				say "== bfq low_latency=$ll slice_idle=$si"
				catalyst-shell --sched-bench io | grep -E '^(quiet|behind)|settings save|uncached'
			done; done
			echo 1 >/sys/block/mmcblk0/queue/iosched/low_latency
			echo 8 >/sys/block/mmcblk0/queue/iosched/slice_idle
		else
			say "== $s"
			catalyst-shell --sched-bench io | grep -E '^(quiet|behind)|settings save|uncached'
		fi
	done
	echo "$was" >/sys/block/mmcblk0/queue/scheduler
	;;
bench)
	say "Stopping the shell for the bench (it comes back at the end). Unplug the charger."
	rc-service catalyst-shell stop
	catalyst-shell --sched-bench clock
	catalyst-shell --sched-bench power
	rc-service catalyst-shell start
	;;
*)
	say "checks: info rest scroll page music-dark model boost-ab tls irq io bench"
	exit 2
	;;
esac

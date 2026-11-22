#!/usr/bin/env python3
"""Count wakeups in the emulator: how often the machine and the shell wake
while the handheld sits idle, with the screen on, with it off, and (with
--deep) in deep standby.

For each spell it reads, before and after: every interrupt the kernel took
(/proc/interrupts, the timer's among them), every context switch
(/proc/stat), the shell's own context switches thread by thread and the
processor time it used (/proc/<pid>/task/*/status, /proc/<pid>/stat), and
every other process's context switches, to name what else wakes.

    python3 scripts/vm-wakeups.py                      # the image's own shell
    python3 scripts/vm-wakeups.py --shell PATH         # a shell built elsewhere, swapped in
    python3 scripts/vm-wakeups.py --deep               # also wait out deep standby (10 min)

Run inside WSL, as vm.py is (CATALYST_BUILD may point at a private copy of
the build's images, so a build elsewhere cannot change them underneath).
Emulated time is not device time: compare one shell with another here, and
read the device's own figures off the device.
"""
import argparse
import http.server
import importlib.util
import os
import shutil
import socketserver
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("vm", os.path.join(HERE, "vm.py"))
vm_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vm_module)

# The counts, as the guest prints them (fetched into it as /tmp/snap.sh).
SNAPSHOT = r"""p=$(pidof catalyst-shell)
echo "irq $(awk 'NR > 1 { for (i = 2; i <= NF; i++) if ($i ~ /^[0-9]+$/) s += $i; else break } END { print s }' /proc/interrupts)"
echo "ctxt $(awk '/^ctxt/ { print $2 }' /proc/stat)"
echo "cpu $(awk '{ print $14 + $15 }' /proc/$p/stat)"
for t in /proc/$p/task/*; do
    echo "thread $(tr ' ' '_' <$t/comm) $(awk '/ctxt_switches/ { s += $2 } END { print s + 0 }' $t/status)"
done
for q in /proc/[0-9]*; do
    [ "$q" = /proc/$p ] && continue
    n=$(tr ' ' '_' <$q/comm 2>/dev/null)
    c=$(awk '/ctxt_switches/ { s += $2 } END { print s + 0 }' $q/status 2>/dev/null)
    [ -n "$n" ] && echo "proc ${q#/proc/}:$n $c"
done
"""


def snapshot(vm):
    out = {"thread": {}, "proc": {}}
    for line in vm.run("sh /tmp/snap.sh", 60).splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[0] in ("irq", "ctxt", "cpu"):
            out[parts[0]] = int(parts[1])
        elif len(parts) == 3 and parts[0] in ("thread", "proc"):
            # Threads of one name are added together (a pool of workers).
            out[parts[0]][parts[1]] = out[parts[0]].get(parts[1], 0) + int(parts[2] or 0)
    return out


def spell(vm, name, seconds):
    """Wakeups per second over `seconds` of the machine left alone."""
    before = snapshot(vm)
    time.sleep(seconds)
    after = snapshot(vm)
    rate = lambda a, b: (a - b) / seconds
    print(f"== {name} ({seconds} s)")
    print(f"   interrupts      {rate(after['irq'], before['irq']):7.1f} /s")
    print(f"   context switches {rate(after['ctxt'], before['ctxt']):6.1f} /s (the whole machine)")
    # Clock ticks of processor time: USER_HZ is 100 on this kernel.
    print(f"   shell processor  {rate(after['cpu'], before['cpu']) * 10:6.2f} ms/s")
    shell = sum(after["thread"].values()) - sum(v for k, v in before["thread"].items() if k in after["thread"])
    print(f"   shell switches   {shell / seconds:6.1f} /s, by thread:")
    for t, n in sorted(after["thread"].items(), key=lambda kv: -(kv[1] - before["thread"].get(kv[0], kv[1]))):
        d = (n - before["thread"].get(t, n)) / seconds
        if d >= 0.05:
            print(f"      {t:<20} {d:6.2f} /s")
    print("   other processes waking most:")
    others = [(k, (v - before["proc"].get(k, v)) / seconds) for k, v in after["proc"].items()]
    for k, d in sorted(others, key=lambda kv: -kv[1])[:8]:
        if d >= 0.05:
            print(f"      {k.split(':', 1)[1]:<20} {d:6.2f} /s")
    return after


def serve(folder):
    """A folder offered to the guest at 10.0.2.2 (QEMU's user network)."""

    class Quiet(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *a, **k):
            super().__init__(*a, directory=folder, **k)

        def log_message(self, *a):
            pass

    httpd = socketserver.TCPServer(("0.0.0.0", 0), Quiet)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return f"http://10.0.2.2:{httpd.server_address[1]}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--shell", help="a catalyst-shell built for the handheld, run in place of the image's")
    ap.add_argument("--seconds", type=int, default=60, help="how long each spell is counted")
    ap.add_argument("--deep", action="store_true", help="also wait for deep standby (ten minutes) and count it")
    ap.add_argument("--run", action="append", default=[], help="a command to run in the machine first")
    opts = ap.parse_args()
    vm = vm_module.Machine(fresh=True)
    try:
        vm.login()
        # Set up already, the screen off after two minutes, deep standby after the least wait.
        # A team's robot set but not there, as on a handheld away from the field:
        # its link, logs and schedule keep trying.
        vm.run("rc-service catalyst-shell stop >/dev/null 2>&1; mkdir -p /data/catalyst; touch /data/catalyst/setup-done; "
               "printf 'screen_off=120\\ndeep_after=10\\ngreeting=false\\n' >> /data/catalyst/settings; "
               "echo 10.58.5.2 > /data/catalyst/robot")
        offered = tempfile.mkdtemp()
        with open(os.path.join(offered, "snap.sh"), "w") as f:
            f.write(SNAPSHOT)
        if opts.shell:
            shutil.copy(opts.shell, os.path.join(offered, "catalyst-shell"))
        url = serve(offered)
        print(vm.run(f"wget -q -O /tmp/snap.sh {url}/snap.sh && echo the counter is in", 120))
        if opts.shell:
            print(vm.run(f"wget -q -O /data/catalyst-shell.new {url}/catalyst-shell && chmod +x /data/catalyst-shell.new && "
                         "mount --bind /data/catalyst-shell.new /usr/bin/catalyst-shell && echo swapped in", 300))
        for command in opts.run:
            print(vm.run(command, 120))
        vm.run("rc-service catalyst-shell start >/dev/null 2>&1")
        time.sleep(5)
        # Past the start animation (the last key), then left alone.
        vm.key("esc")
        t0 = time.time()
        time.sleep(15)
        print(vm.run("ps -o pid,args | grep -v grep | grep -E 'catalyst-shell|ifplugd|bluetoothd|wpa_supplicant|crond|udhcpc' | cut -c1-110"))
        seconds = min(opts.seconds, 80)
        spell(vm, "screen on, Home, nothing moving", seconds)
        # It dims at 112 s and is dark by 121 s.
        time.sleep(max(0, t0 + 130 - time.time()))
        spell(vm, "screen off (standby)", opts.seconds)
        if opts.deep:
            # Ten minutes from the last key, and its ten-second look.
            time.sleep(max(0, t0 + 625 - time.time()))
            print(vm.run("cat /run/catalyst-deep 2>/dev/null; echo checked"))
            spell(vm, "deep standby", max(opts.seconds, 180))
        print(vm.run("tail -n 5 /tmp/catalyst-shell.log"))
    finally:
        vm.stop()


if __name__ == "__main__":
    main()

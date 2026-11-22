#!/usr/bin/env python3
"""music-xrun-harness: count ALSA-style underruns (xruns) in the Music player's audio
pipeline, old ("before") vs new ("after"), with CPU load and a slow, stalling network.

Python 3 stdlib only. Runs on a PC and on the handheld. It never opens a sound card:
the ALSA sink is emulated by a pipe whose capacity is the ALSA buffer, drained by a
separate "hardware" process one period at a time on a fixed clock. Whenever the sink
needs a period and less than a full period has arrived, that period is an xrun; the
sink takes what is there and carries on.

Pipelines
  before  ffmpeg reads the song's http URL itself (-reconnect 1 -reconnect_streamed 1,
          -af aresample=48000 -ac 2), at the same nice as the load (10).
          Sink: buffer 4096 frames, period 1024 frames (85 ms, the old asound.conf).
          -flush_packets 1 makes ffmpeg write every decoded packet at once, the way the
          old player's "-f alsa" output called snd_pcm_writei per packet.
  after   a downloader fetches the song in 1 MiB Range chunks with curl (nice 10) into a
          cache file; a reader thread follows the growing file (waits for more data, and
          only treats the end as EOF once the download is complete) and writes it into
          ffmpeg's stdin. Playback starts once ~5 s of audio (or the whole file) is on
          disk. ffmpeg -threads 1 at nice 0, no aresample for 48 kHz input.
          Sink: buffer 16384 frames, period 2048 frames (341 ms).

Scenarios: idle, cpu (N busy-spinning processes at nice 10), net (throttled to a multiple
of the song's bitrate, plus a stall every N seconds), cpu+net. By default a stall keeps the
TCP connection open, so whatever the kernel socket buffers already hold still plays; --drop
also resets the connection at each stall, so the client must reconnect.

CPU contention: the decoder, the feeder/downloader and the load are pinned to one CPU
(--cpu); the sink ("hardware") to another (--sink-cpu) and the HTTP server to a third
(--server-cpu), so a many-core PC still sees a contended core. Pinning uses
os.sched_setaffinity and niceness os.setpriority, so no taskset or nice binary is needed.

Example
  python3 music-xrun-harness.py --ffmpeg /usr/bin/ffmpeg --curl /usr/bin/curl --song song.webm
"""
import argparse
import fcntl
import json
import os
import re
import select
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

F_SETPIPE_SZ = getattr(fcntl, "F_SETPIPE_SZ", 1031)
F_GETPIPE_SZ = getattr(fcntl, "F_GETPIPE_SZ", 1032)

RATE = 48000
FRAME_BYTES = 4  # S16LE stereo
CHUNK = 1 << 20  # downloader Range chunk
START_SECONDS = 5.0  # audio on disk before "after" starts playback

MODES = {
    "before": {"buffer_frames": 4096, "period_frames": 1024},
    "after": {"buffer_frames": 16384, "period_frames": 2048},
}
SCENARIOS = {  # name: (cpu load, network throttle + stalls)
    "idle": (False, False),
    "cpu": (True, False),
    "net": (False, True),
    "cpu+net": (True, True),
}
CTYPES = {".webm": "audio/webm", ".m4a": "audio/mp4", ".mp4": "audio/mp4", ".opus": "audio/ogg",
          ".ogg": "audio/ogg", ".mp3": "audio/mpeg", ".flac": "audio/flac", ".wav": "audio/wav"}


def pin(pid, cpu):
    if cpu is None or cpu < 0:
        return
    try:
        os.sched_setaffinity(pid, {cpu})
    except (OSError, AttributeError) as e:
        print("warning: cannot pin pid %d to CPU %d: %s" % (pid, cpu, e), file=sys.stderr)


def renice(pid, value):
    try:
        os.setpriority(os.PRIO_PROCESS, pid, value)
    except OSError as e:
        print("warning: cannot set nice %d on pid %d: %s" % (value, pid, e), file=sys.stderr)


def probe_song(ffmpeg, path):
    """Duration (s) and sample rate (Hz) from ffmpeg's own banner (the handheld has no ffprobe)."""
    p = subprocess.run([ffmpeg, "-nostdin", "-hide_banner", "-i", path],
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    err = p.stderr.decode("utf-8", "replace")
    m = re.search(r"Duration: (\d+):(\d+):(\d+(?:\.\d+)?)", err)
    dur = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3)) if m else 0.0
    m = re.search(r"Audio: .*?(\d+) Hz", err)
    rate = int(m.group(1)) if m else 0
    if not dur or not rate:
        sys.exit("cannot probe %s with %s:\n%s" % (path, ffmpeg, err[-800:]))
    return dur, rate


# ---------------------------------------------------------------- HTTP server (the "internet")

class Link:
    """bps throttle plus periodic stalls. Without drop, a stall just sends nothing and the TCP
    connection survives (a short Wi-Fi blip). With drop, each stall also resets open
    connections (RST) so the client has to reconnect; new requests are held until the link is
    back. Note: on Linux the client still reads whatever was already in its receive queue
    before it sees the reset, so with a link faster than the bitrate the kernel socket buffer
    (~100+ KB, several seconds of a 142 kbit/s stream) keeps working as a prefetch buffer."""

    def __init__(self, bps, stall_s, stall_every, t0, drop=False):
        self.bps, self.stall_s, self.stall_every, self.t0 = bps, stall_s, stall_every, t0
        self.drop = drop

    def stall_left(self):
        """Seconds left in the current stall window (0 when the link is up).
        The last stall_s seconds of every stall_every window carry nothing."""
        if not (self.stall_s and self.stall_every):
            return 0.0
        ph = (time.monotonic() - self.t0) % self.stall_every
        start = self.stall_every - self.stall_s
        return self.stall_every - ph if ph >= start else 0.0


def make_handler(path, link):
    size = os.path.getsize(path)
    ctype = CTYPES.get(os.path.splitext(path)[1].lower(), "application/octet-stream")

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        disable_nagle_algorithm = True

        def log_message(self, *args):
            pass

        def _range(self):
            r = self.headers.get("Range")
            if not r:
                return 0, size - 1, False
            m = re.match(r"\s*bytes=(\d*)-(\d*)\s*$", r)
            if not m or (m.group(1) == "" and m.group(2) == ""):
                return 0, size - 1, False
            a, b = m.groups()
            if a == "":
                return max(0, size - int(b)), size - 1, True
            a = int(a)
            b = min(int(b), size - 1) if b else size - 1
            if a >= size or b < a:
                return None
            return a, b, True

        def _reset(self):
            """Abort the connection with an RST (SO_LINGER 0), like a link that went away."""
            try:
                self.connection.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
                self.connection.close()
            except OSError:
                pass
            self.close_connection = True

        def _reply(self, body):
            left = link.stall_left()
            if left > 0:  # no connectivity: the request gets nowhere until the link is back
                time.sleep(left)
            rg = self._range()
            if rg is None:
                self.send_response(416)
                self.send_header("Content-Range", "bytes */%d" % size)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            a, b, partial = rg
            n = b - a + 1
            self.send_response(206 if partial else 200)
            self.send_header("Content-Type", ctype)
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(n))
            if partial:
                self.send_header("Content-Range", "bytes %d-%d/%d" % (a, b, size))
            self.end_headers()
            if body:
                try:
                    self._body(a, n)
                except (BrokenPipeError, ConnectionResetError):
                    self.close_connection = True
                except ConnectionAbortedError:
                    self._reset()

        def do_HEAD(self):
            self._reply(False)

        def do_GET(self):
            self._reply(True)

        def _body(self, a, n):
            with open(path, "rb") as f:
                f.seek(a)
                if not link.bps and not link.stall_s:
                    while n > 0:
                        d = f.read(min(n, 65536))
                        if not d:
                            return
                        self.wfile.write(d)
                        n -= len(d)
                    return
                piece = max(512, min(16384, int(link.bps / 50))) if link.bps else 16384
                burst = 2 * piece
                tokens, last = 0.0, time.monotonic()
                while n > 0:
                    left = link.stall_left()
                    if left > 0:
                        if link.drop:
                            raise ConnectionAbortedError
                        time.sleep(left)
                        tokens, last = 0.0, time.monotonic()
                        continue
                    want = min(n, piece)
                    if link.bps:
                        now = time.monotonic()
                        tokens = min(burst, tokens + (now - last) * link.bps)
                        last = now
                        if tokens < want:
                            time.sleep(min((want - tokens) / link.bps, 0.05))
                            continue
                    d = f.read(want)
                    if not d:
                        return
                    self.wfile.write(d)
                    n -= len(d)
                    tokens -= len(d)

    return Handler


def server_main(path, link, cpu, port_w):
    pin(0, cpu)
    httpd = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(path, link))
    httpd.daemon_threads = True
    os.write(port_w, str(httpd.server_address[1]).encode())
    os.close(port_w)
    httpd.serve_forever()


# ---------------------------------------------------------------- the ALSA sink (the "hardware")

def sink_main(rfd, mode, duration, t0, cpu, res_w):
    pin(0, cpu)
    cfg = MODES[mode]
    pbytes = cfg["period_frames"] * FRAME_BYTES
    pdur = cfg["period_frames"] / float(RATE)
    fl = fcntl.fcntl(rfd, fcntl.F_GETFL)
    fcntl.fcntl(rfd, fcntl.F_SETFL, fl | os.O_NONBLOCK)

    def take(n):
        got = 0
        while got < n:
            try:
                b = os.read(rfd, n - got)
            except BlockingIOError:
                return got, False
            if not b:
                return got, True
            got += len(b)
        return got, False

    res = {"first_sound_s": None, "xrun_periods": 0, "xrun_events": 0, "short_s": 0.0,
           "played_s": 0.0, "ended_early": False, "max_sink_late_ms": 0.0}
    have, eof = 0, False
    while have < pbytes and not eof:  # playback starts with the first full period
        if time.monotonic() - t0 > 180:
            break
        if select.select([rfd], [], [], 0.5)[0]:
            g, eof = take(pbytes - have)
            have += g
    if have < pbytes:
        res["error"] = "no audio arrived"
        os.write(res_w, json.dumps(res).encode())
        return
    first = time.monotonic()
    res["first_sound_s"] = round(first - t0, 3)
    nper = int(duration / pdur) if duration > 0 else 1 << 62
    k, in_x, short, cur = 0, False, 0, None
    xat = []  # [playback second where an xrun event starts, short periods in it] (first 50)
    res["xrun_at"] = xat
    while k < nper:
        k += 1
        dl = first + k * pdur
        now = time.monotonic()
        if dl > now:
            time.sleep(dl - now)
        else:
            res["max_sink_late_ms"] = max(res["max_sink_late_ms"], (now - dl) * 1000.0)
        got, eof = take(pbytes)
        if eof:  # the stream ended: not an underrun
            k -= 1
            res["ended_early"] = duration > 0 and k * pdur < duration - 2.0
            break
        if got < pbytes:
            res["xrun_periods"] += 1
            short += pbytes - got
            if not in_x:
                res["xrun_events"] += 1
                in_x = True
                cur = [round(k * pdur, 2), 0] if len(xat) < 50 else None
                if cur is not None:
                    xat.append(cur)
            if cur is not None:
                cur[1] += 1
        else:
            in_x = False
    res["played_s"] = round(k * pdur, 2)
    res["short_s"] = round(short / float(FRAME_BYTES * RATE), 2)
    res["max_sink_late_ms"] = round(res["max_sink_late_ms"], 1)
    os.write(res_w, json.dumps(res).encode())


# ---------------------------------------------------------------- the new player ("after")

def player_after_main(a, url, song_dur, song_rate, out_w, cache_dir, log_path, clen):
    """Runs in its own process group, pinned to the contended CPU, at nice 0.
    clen is the size the player already knows (YouTube stream URLs carry clen=);
    the downloader still takes the real size from Content-Range."""
    os.setpgrp()
    pin(0, a.cpu)
    tstart = time.monotonic()
    # Named as the shell names one downloading (catalyst-play goes by the extension).
    cache = os.path.join(cache_dir, "song" + os.path.splitext(a.song)[1] + ".part")
    hdr = cache + ".hdr"
    open(cache, "wb").close()
    st = {"total": None, "err": None}
    done = threading.Event()
    log = open(log_path, "ab")

    def parse_total():
        try:
            with open(hdr, "rb") as f:
                h = f.read().decode("latin-1")
        except OSError:
            return None
        m = re.search(r"(?im)^content-range:\s*bytes\s+\d+-\d+/(\d+)", h)
        if m:
            return int(m.group(1))
        if re.search(r"(?m)^HTTP/\S+\s+200", h):
            m = re.search(r"(?im)^content-length:\s*(\d+)", h)
            if m:
                return int(m.group(1))
        return None

    def downloader():
        pos, fails = 0, 0
        with open(cache, "ab") as out:
            while st["total"] is None or pos < st["total"]:
                end = pos + CHUNK - 1
                if st["total"] is not None:
                    end = min(end, st["total"] - 1)
                p = subprocess.Popen([a.curl, "-sS", "-f", "-r", "%d-%d" % (pos, end), "-D", hdr, url],
                                     stdout=out, stderr=log, stdin=subprocess.DEVNULL)
                renice(p.pid, 10)
                rc = p.wait()
                if st["total"] is None:
                    st["total"] = parse_total()
                newpos = os.fstat(out.fileno()).st_size
                if rc != 0 or newpos == pos:
                    fails += 1
                    if fails > 30:
                        st["err"] = "curl failed (rc %d)" % rc
                        break
                    time.sleep(0.5)
                else:
                    fails = 0
                pos = newpos
        log.write(("harness: download finished, %d bytes, %.2f s after start\n"
                   % (pos, time.monotonic() - tstart)).encode())
        log.flush()
        done.set()

    threading.Thread(target=downloader, daemon=True).start()

    while True:  # wait until ~5 s of audio (or the whole file) is on disk
        size = os.path.getsize(cache)
        tot = st["total"] or clen
        if done.is_set():
            break
        if tot and size >= min(tot, int(START_SECONDS * tot / song_dur)):
            break
        time.sleep(0.02)

    if a.player:
        # The shell's player: it follows the growing file itself, given the size it will have.
        tot = st["total"] or clen
        cmd = [a.player, "--raw"] + (["--size", str(tot)] if tot else []) + [cache]
        pl = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=out_w, stderr=log)
        os.close(out_w)
        pl.wait()
        os._exit(0)

    cmd = [a.ffmpeg, "-nostdin", "-hide_banner", "-loglevel", "warning", "-threads", "1",
           "-i", "pipe:0", "-vn", "-ac", "2"]
    if song_rate != RATE:
        cmd += ["-af", "aresample=%d" % RATE]
    cmd += ["-f", "s16le", "pipe:1"]
    ff = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=out_w, stderr=log, bufsize=0)
    os.close(out_w)

    def reader():
        fd = os.open(cache, os.O_RDONLY)
        pos = 0
        try:
            while True:
                d = os.read(fd, 65536)
                if d:
                    mv = memoryview(d)
                    while mv:
                        n = ff.stdin.write(mv)
                        mv = mv[n:]
                    pos += len(d)
                    continue
                # at the current end of the cache file: only EOF if the download is complete
                if done.is_set() and pos >= os.path.getsize(cache):
                    break
                time.sleep(0.05)
        except (BrokenPipeError, OSError):
            pass
        finally:
            os.close(fd)
            try:
                ff.stdin.close()
            except OSError:
                pass

    t = threading.Thread(target=reader, daemon=True)
    t.start()
    ff.wait()
    os._exit(0)


# ---------------------------------------------------------------- one run

def run_one(a, mode, scenario, song_dur, song_rate, bitrate_bps, workdir):
    cpu_load, net = SCENARIOS[scenario]
    cfg = MODES[mode]
    duration = a.duration if a.duration > 0 else song_dur
    warm = 1.0 if cpu_load else 0.2
    t0 = time.monotonic() + warm
    link = Link(a.throttle * bitrate_bps if net else 0, a.stall if net else 0, a.stall_every if net else 0, t0,
                drop=a.drop and net)
    log_path = os.path.join(workdir, "%s-%s.log" % (mode, scenario.replace("+", "_")))
    open(log_path, "wb").close()
    kids = []

    port_r, port_w = os.pipe()
    spid = os.fork()
    if spid == 0:
        os.close(port_r)
        try:
            server_main(a.song, link, a.server_cpu, port_w)
        finally:
            os._exit(0)
    os.close(port_w)
    port = int(os.read(port_r, 16) or b"0")
    os.close(port_r)
    kids.append(("pid", spid))
    url = "http://127.0.0.1:%d/song%s" % (port, os.path.splitext(a.song)[1].lower())

    pr, pw = os.pipe()  # the ALSA ring buffer
    try:
        fcntl.fcntl(pr, F_SETPIPE_SZ, cfg["buffer_frames"] * FRAME_BYTES)
    except OSError as e:
        print("warning: F_SETPIPE_SZ failed: %s" % e, file=sys.stderr)
    pipe_bytes = fcntl.fcntl(pr, F_GETPIPE_SZ)
    res_r, res_w = os.pipe()
    kpid = os.fork()
    if kpid == 0:
        os.close(pw)
        os.close(res_r)
        try:
            sink_main(pr, mode, duration, t0, a.sink_cpu, res_w)
        finally:
            os._exit(0)
    os.close(pr)
    os.close(res_w)
    kids.append(("pid", kpid))

    if cpu_load:
        for _ in range(a.cpu_load):
            p = subprocess.Popen(["/bin/sh", "-c", "while :; do :; done"], stdin=subprocess.DEVNULL)
            renice(p.pid, 10)
            pin(p.pid, a.cpu)
            kids.append(("popen", p))

    now = time.monotonic()
    if t0 > now:
        time.sleep(t0 - now)
    if mode == "before":
        cmd = [a.ffmpeg, "-nostdin", "-hide_banner", "-loglevel", "warning",
               "-reconnect", "1", "-reconnect_streamed", "1"]
        if a.rcvbuf:
            cmd += ["-recv_buffer_size", str(a.rcvbuf)]
        cmd += ["-i", url,
               "-vn", "-af", "aresample=%d" % RATE, "-ac", "2", "-flush_packets", "1",
               "-f", "s16le", "pipe:1"]
        with open(log_path, "ab") as log:
            p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=pw, stderr=log)
        renice(p.pid, 10)  # same nice as the load
        pin(p.pid, a.cpu)
        kids.append(("popen", p))
    else:
        cache_dir = tempfile.mkdtemp(prefix="cache-", dir=workdir)
        ppid = os.fork()
        if ppid == 0:
            os.close(res_r)
            try:
                player_after_main(a, url, song_dur, song_rate, pw, cache_dir, log_path,
                                  os.path.getsize(a.song))
            finally:
                os._exit(0)
        kids.append(("pgrp", ppid))
    os.close(pw)

    data = b""
    deadline = time.monotonic() + duration + 240
    while time.monotonic() < deadline:
        r = select.select([res_r], [], [], 1.0)[0]
        if r:
            chunk = os.read(res_r, 65536)
            if not chunk:
                break
            data += chunk
    os.close(res_r)

    for kind, k in reversed(kids):
        try:
            if kind == "popen":
                k.kill()
                k.wait()
            elif kind == "pgrp":
                os.killpg(k, signal.SIGKILL)
                os.waitpid(k, 0)
            else:
                os.kill(k, signal.SIGKILL)
                os.waitpid(k, 0)
        except (OSError, ChildProcessError):
            pass
    try:
        res = json.loads(data.decode() or "{}")
    except ValueError:
        res = {"error": "bad sink result"}
    with open(log_path, "rb") as f:
        warn = [l for l in f.read().decode("utf-8", "replace").splitlines() if l.strip()]
    res.update({"mode": mode, "scenario": scenario, "pipe_bytes": pipe_bytes,
                "link_bps": int(link.bps), "log_lines": len(warn),
                "log_tail": [l[:160] for l in warn[-3:]]})
    return res


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ffmpeg", default="/usr/bin/ffmpeg")
    ap.add_argument("--curl", default="/usr/bin/curl")
    ap.add_argument("--player", default=None,
                    help="catalyst-play: the after mode's decoder, reading the growing cache file itself "
                         "(--raw --size), as the shell runs it; without it, ffmpeg fed by a reader thread")
    ap.add_argument("--song", required=True, help="song file (served over local HTTP)")
    ap.add_argument("--modes", default="before,after")
    ap.add_argument("--scenarios", default="idle,cpu,net,cpu+net")
    ap.add_argument("--duration", type=float, default=0, help="seconds of playback per run (0 = the whole song)")
    ap.add_argument("--cpu-load", type=int, default=4, help="busy-spinning processes for the cpu scenarios")
    ap.add_argument("--throttle", type=float, default=1.5, help="link speed as a multiple of the song's bitrate")
    ap.add_argument("--stall", type=float, default=3.0, help="seconds with nothing sent ...")
    ap.add_argument("--stall-every", type=float, default=20.0, help="... once in every this many seconds")
    ap.add_argument("--drop", action="store_true",
                    help="each stall also resets the TCP connection (RST), so the client must reconnect")
    ap.add_argument("--rcvbuf", type=int, default=0,
                    help="before mode: pass -recv_buffer_size to ffmpeg (fixed SO_RCVBUF). Over loopback the "
                         "kernel autotunes the receive buffer to hold tens of seconds of a slow stream, which "
                         "hides stalls; a Wi-Fi socket read at 142 kbit/s stays near the 128 KB default")
    ncpu = os.cpu_count() or 1
    ap.add_argument("--cpu", type=int, default=1 if ncpu > 1 else -1, help="CPU for decoder, feeder and load")
    ap.add_argument("--sink-cpu", type=int, default=0 if ncpu > 1 else -1, help="CPU for the emulated sound card")
    ap.add_argument("--server-cpu", type=int, default=2 if ncpu > 2 else (0 if ncpu > 1 else -1))
    ap.add_argument("--no-pin", action="store_true", help="do not pin anything to CPUs")
    ap.add_argument("--workdir", default=None, help="where the cache file and logs go (default: a temp dir)")
    ap.add_argument("--json", default=None, help="also write the results here")
    a = ap.parse_args()
    if a.no_pin:
        a.cpu = a.sink_cpu = a.server_cpu = -1
    a.song = os.path.abspath(a.song)
    for exe in (a.ffmpeg, a.curl):
        if not os.access(exe, os.X_OK):
            sys.exit("not executable: %s" % exe)
    modes = [m for m in a.modes.split(",") if m]
    scens = [s for s in a.scenarios.split(",") if s]
    for m in modes:
        if m not in MODES:
            sys.exit("unknown mode %s" % m)
    for s in scens:
        if s not in SCENARIOS:
            sys.exit("unknown scenario %s" % s)
    workdir = a.workdir or tempfile.mkdtemp(prefix="xrun-harness-")
    os.makedirs(workdir, exist_ok=True)

    song_dur, song_rate = probe_song(a.ffmpeg, a.song)
    size = os.path.getsize(a.song)
    bitrate = size / song_dur  # bytes per second
    print("song: %s  %.1f s  %d Hz  %.1f kbit/s  (%d bytes)" % (a.song, song_dur, song_rate, bitrate * 8 / 1000, size))
    print("net scenarios: link %.1f kbit/s (%.2fx), stall %.1f s every %.1f s%s; cpu scenarios: %d spinners at nice 10"
          % (a.throttle * bitrate * 8 / 1000, a.throttle, a.stall, a.stall_every,
             " with connection reset" if a.drop else "", a.cpu_load))
    print("pinning: decoder/feeder/load CPU %s, sink CPU %s, server CPU %s; each run %s"
          % (a.cpu, a.sink_cpu, a.server_cpu, ("%.0f s" % a.duration) if a.duration > 0 else "the whole song"))
    print("workdir: %s" % workdir)
    sys.stdout.flush()

    results = []
    for s in scens:
        for m in modes:
            r = run_one(a, m, s, song_dur, song_rate, bitrate, workdir)
            results.append(r)
            print("  %-8s %-7s xrun periods %5s  events %4s  silence %6ss  first sound %7ss  played %7ss%s"
                  % (s, m, r.get("xrun_periods"), r.get("xrun_events"), r.get("short_s"),
                     r.get("first_sound_s"), r.get("played_s"),
                     ("  ERROR " + r["error"]) if r.get("error") else (
                         "  (stream ended early)" if r.get("ended_early") else "")))
            sys.stdout.flush()

    print()
    print("| scenario | mode | xrun periods | xrun events | silence s | first sound s | played s |")
    print("|---|---|---|---|---|---|---|")
    for r in results:
        print("| %s | %s | %s | %s | %s | %s | %s |" % (r["scenario"], r["mode"], r.get("xrun_periods"),
              r.get("xrun_events"), r.get("short_s"), r.get("first_sound_s"), r.get("played_s")))
    if a.json:
        with open(a.json, "w") as f:
            json.dump({"song": a.song, "duration": song_dur, "rate": song_rate, "bitrate_Bps": bitrate,
                       "args": vars(a), "results": results}, f, indent=1)


if __name__ == "__main__":
    main()

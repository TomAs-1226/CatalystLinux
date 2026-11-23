"""Check fe.c (the K210's streaming front end) against features.py.

    python3 tools/unitv/speech/test_fe.py [clip.wav ...]

Builds fe.c with gcc (-std=c99 -Wall -Wextra -pedantic -Werror) into a
shared library in a temporary directory, checks it calls nothing from libm
but logf/log10f/cosf/sqrtf, then streams test audio through it in random
cut sizes (0..700 samples, through ctypes) and after every push compares:

  - every new frame's 40 log-mels (max abs error <= 2e-3, mean <= 1e-4)
    and energy (<= 1e-3 dB) with features.analyse;
  - fe_vad with features.vad_trace run on fe.c's energies (exact), and
    with features.py's own energies (reported; may differ only where an
    energy sits within float rounding of a threshold);
  - fe_best_window for assorted ranges (inside, short, reversed, past the
    end, reaching frames that have left the 512-frame ring) with
    features.best_window run on fe.c's powers (exact) and on its own
    powers (same start, or a window whose energy is within 1e-5);
  - fe_window (normalised, with silence rows past the end and for frames
    gone from the ring) with features.window;
  - fe_logmel/fe_energy for frames not kept (NULL / -100).

The audio: about 14 s of synthetic sound (digital silence, mic noise,
voiced harmonic "words" at -20..-50 dBFS with gaps shorter and longer than
the VAD's 30-frame hangover, a clipped chirp, noise bursts), long enough to
wrap the ring; then a stream under 48 frames; then any wav files given
(16 kHz mono int16), e.g. Speech Commands clips. Needs numpy and gcc.
"""

import ctypes
import os
import subprocess
import sys
import tempfile
import wave

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import features as F  # noqa: E402

LIBM_OK = {"logf", "log10f", "cosf", "sqrtf"}
LIBC_OK = {"memset", "memmove", "memcpy"}


def build(tmp):
    shim = os.path.join(tmp, "fe_shim.c")
    with open(shim, "w") as f:
        f.write('#include "%s"\nsize_t fe_sizeof(void) { return sizeof(fe_t); }\n'
                % os.path.join(HERE, "fe.c").replace("\\", "/"))
    flags = ["-std=c99", "-O2", "-Wall", "-Wextra", "-pedantic", "-Werror",
             "-Wshadow", "-Wdouble-promotion", "-fno-fast-math", "-fno-stack-protector"]
    obj = os.path.join(tmp, "fe.o")
    subprocess.check_call(["gcc", *flags, "-c", "-fPIC",
                           os.path.join(HERE, "fe.c"), "-o", obj])
    undefined = subprocess.check_output(["nm", "-u", obj], text=True).split()
    used = {u.split("@")[0] for u in undefined if u not in ("U", "w")}
    bad = used - LIBM_OK - LIBC_OK
    if bad:
        raise SystemExit("fe.c calls %s (only %s allowed)" % (sorted(bad), sorted(LIBM_OK | LIBC_OK)))
    so = os.path.join(tmp, "libfe.so")
    subprocess.check_call(["gcc", *flags, "-shared", "-fPIC", shim, "-o", so, "-lm"])
    lib = ctypes.CDLL(so)
    P = ctypes.c_void_p
    i, f = ctypes.c_int, ctypes.c_float
    fp = ctypes.POINTER(ctypes.c_float)
    ip = ctypes.POINTER(ctypes.c_int)
    lib.fe_sizeof.restype = ctypes.c_size_t
    lib.fe_init.argtypes = [P]
    lib.fe_push.argtypes = [P, ctypes.POINTER(ctypes.c_int16), i]
    lib.fe_push.restype = i
    lib.fe_count.argtypes = [P]
    lib.fe_count.restype = i
    lib.fe_logmel.argtypes = [P, i]
    lib.fe_logmel.restype = fp
    lib.fe_energy.argtypes = [P, i]
    lib.fe_energy.restype = f
    lib.fe_power.argtypes = [P, i]
    lib.fe_power.restype = f
    lib.fe_vad.argtypes = [P, ip, ip, ip, ip]
    lib.fe_best_window.argtypes = [P, i, i]
    lib.fe_best_window.restype = i
    lib.fe_window.argtypes = [P, i, fp, fp, fp]
    return lib, used


def synth(rng):
    sr = F.SAMPLE_RATE
    parts = []

    def noise(sec, dbfs):
        return rng.standard_normal(int(sec * sr)) * 10 ** (dbfs / 20)

    def word(sec, dbfs, f0):
        t = np.arange(int(sec * sr)) / sr
        f = f0 * (1 + 0.15 * np.sin(2 * np.pi * 3 * t))
        ph = 2 * np.pi * np.cumsum(f) / sr
        x = sum(np.sin(h * ph) / h for h in range(1, 25))
        x += 0.3 * rng.standard_normal(len(t))
        env = np.sin(np.pi * t / sec) ** 0.7
        x = x * env
        return x / np.sqrt(np.mean(x ** 2)) * 10 ** (dbfs / 20)

    parts.append(np.zeros(int(0.3 * sr)))
    parts.append(noise(0.8, -75))
    parts.append(word(0.55, -22, 130) + noise(0.55, -75))
    parts.append(noise(0.9, -75))                 # > 30 frames: ended
    parts.append(word(0.4, -45, 220) + noise(0.4, -75))
    parts.append(noise(0.3, -75))                 # < 30 frames: no end
    parts.append(word(0.7, -30, 180) + noise(0.7, -75))
    parts.append(noise(1.2, -62))                 # louder room noise
    parts.append(word(0.5, -50, 110) + noise(0.5, -62))
    parts.append(np.zeros(int(0.5 * sr)))
    t = np.arange(int(1.0 * sr)) / sr
    parts.append(1.4 * np.sin(2 * np.pi * (100 + 3500 * t) * t))  # clipped chirp
    parts.append(noise(0.6, -70))
    for _ in range(6):
        parts.append(noise(rng.uniform(0.05, 0.4), rng.uniform(-40, -15)))
        parts.append(noise(rng.uniform(0.1, 0.8), -80))
    while sum(len(p) for p in parts) < 14 * sr:
        sec = rng.uniform(0.3, 0.8)
        w = word(sec, rng.uniform(-40, -20), rng.uniform(90, 250))
        parts.append(w + noise(sec, -75)[:len(w)])
        parts.append(noise(rng.uniform(0.2, 1.0), -72))
    x = np.concatenate(parts)
    return np.clip(np.round(x * 32768), -32768, 32767).astype(np.int16)


def read_wav(path):
    with wave.open(path) as w:
        assert w.getframerate() == 16000 and w.getsampwidth() == 2 and w.getnchannels() == 1
        return np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.int16)


class Check:
    def __init__(self):
        self.fail = []
        self.n = {}
        self.states_seen = set()

    def count(self, key):
        self.n[key] = self.n.get(key, 0) + 1

    def bad(self, msg):
        if len(self.fail) < 20:
            print("FAIL:", msg)
        self.fail.append(msg)


def run(lib, size, pcm, rng, chk, name, mean, inv_std):
    st = ctypes.create_string_buffer(size)
    lib.fe_init(st)
    ref_l, ref_e, ref_p = F.analyse(pcm)
    nf = len(ref_e)
    ref_hist = []
    F.vad_trace(ref_e, ref_hist)
    c_e = np.zeros(nf, np.float32)
    c_p = np.zeros(nf, np.float64)
    c_l = np.zeros((nf, F.BANDS), np.float32)
    errs, eerrs = [], []
    vad_ref_diff = pushes = 0
    states = set()
    mean_c = mean.ctypes.data_as(ctypes.POINTER(ctypes.c_float))
    istd_c = inv_std.ctypes.data_as(ctypes.POINTER(ctypes.c_float))
    out = np.zeros(F.WINDOW_FRAMES * F.BANDS, np.float32)
    out_c = out.ctypes.data_as(ctypes.POINTER(ctypes.c_float))
    pos = 0
    while pos < len(pcm):
        n = int(min(rng.integers(0, 701), len(pcm) - pos))
        chunk = np.ascontiguousarray(pcm[pos:pos + n])
        before = lib.fe_count(st)
        added = lib.fe_push(st, chunk.ctypes.data_as(ctypes.POINTER(ctypes.c_int16)), n)
        pos += n
        count = lib.fe_count(st)
        if count != before + added or count != F.n_frames(pos):
            chk.bad("%s: count %d after %d samples, want %d" % (name, count, pos, F.n_frames(pos)))
            return
        for k in range(before, count):
            ptr = lib.fe_logmel(st, k)
            c_l[k] = np.ctypeslib.as_array(ptr, shape=(F.BANDS,))
            c_e[k] = lib.fe_energy(st, k)
            c_p[k] = lib.fe_power(st, k)
            errs.append(np.abs(c_l[k] - ref_l[k]))
            eerrs.append(abs(float(c_e[k]) - float(ref_e[k])))
        if count == 0:
            continue
        # VAD
        v = [ctypes.c_int() for _ in range(4)]
        lib.fe_vad(st, *[ctypes.byref(x) for x in v])
        got = tuple(x.value for x in v)
        want = F.vad_trace(c_e[:count])
        chk.count("vad")
        if got != want:
            chk.bad("%s: VAD at frame %d: C %s, features.py on C energies %s" % (name, count - 1, got, want))
        pushes += 1
        states.add(got[0])
        if got != ref_hist[count - 1]:
            vad_ref_diff += 1
        # windows
        oldest = max(0, count - F.RING)
        ranges = [(0, count - 1), (oldest, count - 1), (count - 1, count - 1),
                  (count - 30, count - 1), (count - 100, count - 60),
                  (count - 1, 0), (-50, count + 50), (count + 5, count + 9)]
        for _ in range(3):
            a, b = sorted(rng.integers(-60, count + 60, size=2))
            ranges.append((int(a), int(b)))
        if got[0] > 0:
            ranges.append((got[1], count - 1))
        for a, b in ranges:
            s_c = lib.fe_best_window(st, a, b)
            s_x = F.best_window(c_p, a, b, count)
            chk.count("window")
            if s_c != s_x:
                chk.bad("%s: best_window(%d, %d) count %d: C %d, features.py on C powers %d"
                        % (name, a, b, count, s_c, s_x))
            s_r = F.best_window(ref_p, a, b, count)
            if s_r != s_c:
                e_c = ref_p[max(s_c, 0):s_c + 48].sum()
                e_r = ref_p[max(s_r, 0):s_r + 48].sum()
                if abs(e_c - e_r) > 1e-5 * max(e_r, 1e-12):
                    chk.bad("%s: best_window(%d, %d): C %d, features.py %d (energy %g vs %g)"
                            % (name, a, b, s_c, s_r, e_c, e_r))
        for s in {lib.fe_best_window(st, 0, count - 1), count - 20, oldest - 10, count - 48}:
            lib.fe_window(st, s, mean_c, istd_c, out_c)
            want_w = F.window(ref_l, s, mean, inv_std, count)
            d = np.abs(out.reshape(F.WINDOW_FRAMES, F.BANDS) - want_w).max()
            chk.count("fe_window")
            if d > 2e-3 * float(inv_std.max()):
                chk.bad("%s: fe_window(%d) count %d: max diff %g" % (name, s, count, d))
        # frames not kept
        for k in (oldest - 1, count, -1):
            if lib.fe_logmel(st, k) or lib.fe_energy(st, k) != -100.0:
                chk.bad("%s: frame %d (not kept) still readable" % (name, k))
    chk.states_seen = states
    errs = np.concatenate(errs) if errs else np.zeros(1)
    print("%-22s %5d frames  log-mel max err %.1e mean %.1e  energy max err %.1e dB  "
          "VAD states %s, differs from features.py's own energies at %d of %d pushes"
          % (name, nf, errs.max(), errs.mean(), max(eerrs or [0]), sorted(states), vad_ref_diff, pushes))
    if errs.max() > 2e-3 or errs.mean() > 1e-4:
        chk.bad("%s: log-mel error %g (mean %g)" % (name, errs.max(), errs.mean()))
    if max(eerrs or [0]) > 1e-3:
        chk.bad("%s: energy error %g dB" % (name, max(eerrs)))
    if nf >= 100 and vad_ref_diff > 0.02 * max(pushes, 1):
        chk.bad("%s: VAD on features.py's energies differs too often" % name)


def main():
    rng = np.random.default_rng(5805)
    with tempfile.TemporaryDirectory() as tmp:
        lib, used = build(tmp)
        size = lib.fe_sizeof()
        print("fe.c built; sizeof(fe_t) = %d bytes; libc/libm calls: %s" % (size, " ".join(sorted(used))))
        chk = Check()
        mean = rng.uniform(-12, -2, F.BANDS).astype(np.float32)
        inv_std = rng.uniform(0.2, 0.5, F.BANDS).astype(np.float32)
        pcm = synth(rng)
        run(lib, size, pcm, rng, chk, "synthetic %.1f s" % (len(pcm) / 16000), mean, inv_std)
        if chk.states_seen != {0, 1, 2}:
            chk.bad("the synthetic audio did not take the VAD through states 0, 1 and 2")
        run(lib, size, pcm[:16000 * 3 // 4], rng, chk, "short (<48 frames)", mean, inv_std)
        for p in sys.argv[1:]:
            run(lib, size, read_wav(p), rng, chk, os.path.basename(p)[:22], mean, inv_std)
        print("checks: %s" % ", ".join("%s %d" % kv for kv in sorted(chk.n.items())))
        if chk.fail:
            print("%d FAILED" % len(chk.fail))
            return 1
        print("all passed")
        return 0


if __name__ == "__main__":
    sys.exit(main())

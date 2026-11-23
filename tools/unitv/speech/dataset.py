"""Speech Commands v0.02 for the Catalyst Coprocessor KWS model.

    python3 dataset.py        fetch, verify, extract, cache clips and
                              features (idempotent; a few minutes the
                              first time, plus the 2.4 GB download)

Dataset: Google Speech Commands v0.02 (Pete Warden, 2018), CC-BY-4.0,
https://arxiv.org/abs/1804.03209. The archives' sha256 are checked below.

Everything large lives in the work directory, $CATALYST_SPEECH_WORK or
~/catalyst-coprocessor-build/speech: dl/ (archives), data/ (extracted),
cache/ (int16 clips, features, augmented training sets).

Classes (model output order): the 20 commands, the 10 other trained words
(known words the handheld ignores), then _silence_. Five words are held
out of training altogether for the few-shot (user-word) test and as
never-seen words: backward, follow, forward, learn, visual. Splits follow
the dataset's validation_list.txt / testing_list.txt (by speaker).

Training examples are simulated push-to-talk streams, built by
make_stream(): the 1 s clip, shifted by up to +-100 ms (zero fill), gain
-10..+10 dB, with 0..350 ms of padding before and after, a mic noise
floor (-85..-65 dBFS white) everywhere and, 75 % of the time, a
background-noise segment at 3..25 dB SNR; quantised to int16. The model
input is then chosen exactly as on the K210 (features.kws_input: VAD
start .. last frame, best-energy 48-frame window), then moved by up to
+-2 frames half the time. _silence_ examples are 1.0..1.7 s streams of
background noise (-75..-25 dBFS) or a bare mic floor (-95..-60 dBFS).
Background noise comes from the dataset's _background_noise_ files: the
first 80 % of each for training, the last 20 % for validation/test
silence.
"""

import hashlib
import json
import os
import sys
import tarfile
import urllib.request
import wave
from multiprocessing import Pool

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import features as F  # noqa: E402

WORK = os.environ.get("CATALYST_SPEECH_WORK",
                      os.path.expanduser("~/catalyst-coprocessor-build/speech"))
CACHE = os.path.join(WORK, "cache")

ARCHIVES = {
    "train": ("http://download.tensorflow.org/data/speech_commands_v0.02.tar.gz",
              "af14739ee7dc311471de98f5f9d2c9191b18aedfe957f4a6ff791c709868ff58",
              "data/sc"),
    "official_test": ("http://download.tensorflow.org/data/speech_commands_test_set_v0.02.tar.gz",
                      "cc2a00c1147c2254e9be3fa0f779d8c17421dc349b86366567a8edfa9acd51df",
                      "data/sc_test"),
}

DATASET = "Google Speech Commands v0.02"
LICENSE = "CC-BY-4.0 (https://creativecommons.org/licenses/by/4.0/)"
ATTRIBUTION = ("Speech Commands v0.02 by Pete Warden, Google (2018), "
               "https://arxiv.org/abs/1804.03209, licensed under CC BY 4.0. "
               "The model was trained on it; the dataset is not redistributed.")

COMMANDS = ["yes", "no", "up", "down", "left", "right", "on", "off", "stop", "go",
            "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"]
OTHERS = ["bed", "bird", "cat", "dog", "happy", "house", "marvin", "sheila", "tree", "wow"]
HELD_OUT = ["backward", "follow", "forward", "learn", "visual"]
SILENCE = "_silence_"
LABELS = COMMANDS + OTHERS + [SILENCE]
LABEL_INDEX = {w: i for i, w in enumerate(LABELS)}

SR = 16000
CLIP = 16000
SILENCE_TRAIN = 3000      # _silence_ examples per training variant
SILENCE_EVAL = 600        # per split (val, test), from the held-back noise


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def fetch():
    """Download (if missing), verify and extract both archives."""
    for url, digest, dest in ARCHIVES.values():
        name = url.rsplit("/", 1)[1]
        path = os.path.join(WORK, "dl", name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        if not os.path.exists(path):
            print("downloading", url)
            urllib.request.urlretrieve(url, path + ".part")
            os.replace(path + ".part", path)
        got = sha256(path)
        if got != digest:
            raise SystemExit("%s: sha256 %s, want %s" % (path, got, digest))
        out = os.path.join(WORK, dest)
        if not os.path.exists(os.path.join(out, "LICENSE")):
            print("extracting", name)
            os.makedirs(out, exist_ok=True)
            with tarfile.open(path) as t:
                t.extractall(out, filter="data")


def read_wav(path):
    with wave.open(path) as w:
        if w.getframerate() != SR or w.getsampwidth() != 2 or w.getnchannels() != 1:
            raise ValueError(path)
        return np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.int16)


def _read_clip(path):
    x = read_wav(path)[:CLIP]
    return np.pad(x, (0, CLIP - len(x))), len(x)


def _load_clips(paths):
    out = np.zeros((len(paths), CLIP), np.int16)
    lengths = np.zeros(len(paths), np.int32)
    with Pool(os.cpu_count()) as pool:
        for i, (x, n) in enumerate(pool.imap(_read_clip, paths, chunksize=256)):
            out[i], lengths[i] = x, n
    return out, lengths


def speaker_of(name):
    return name.split("_nohash_")[0].split("_")[-1]


def build_index():
    root = os.path.join(WORK, ARCHIVES["train"][2])
    lists = {}
    for split, fn in (("val", "validation_list.txt"), ("test", "testing_list.txt")):
        with open(os.path.join(root, fn)) as f:
            for line in f:
                if line.strip():
                    lists[line.strip()] = split
    recs = []
    for word in sorted(COMMANDS + OTHERS + HELD_OUT):
        for fn in sorted(os.listdir(os.path.join(root, word))):
            rel = word + "/" + fn
            recs.append({"path": rel, "word": word, "split": lists.get(rel, "train"),
                         "speaker": speaker_of(fn)})
    oroot = os.path.join(WORK, ARCHIVES["official_test"][2])
    orecs = []
    for d in sorted(os.listdir(oroot)):
        if not os.path.isdir(os.path.join(oroot, d)):
            continue
        for fn in sorted(os.listdir(os.path.join(oroot, d))):
            word = d if d != "_unknown_" else fn.split("_")[0]
            orecs.append({"path": d + "/" + fn, "word": word, "folder": d,
                          "speaker": speaker_of(fn) if d != SILENCE else ""})
    return recs, orecs


def load_noise():
    root = os.path.join(WORK, ARCHIVES["train"][2], "_background_noise_")
    train, test = [], []
    for fn in sorted(os.listdir(root)):
        if fn.endswith(".wav"):
            x = read_wav(os.path.join(root, fn)).astype(np.float32) / 32768.0
            cut = int(len(x) * 0.8)
            train.append(x[:cut])
            test.append(x[cut:])
    return train, test


def rms_db(x):
    return 10 * np.log10(np.mean(np.asarray(x, np.float64) ** 2) + 1e-12)


def peak_power(x, win=4000):
    """Mean power of the loudest 0.25 s: the word's level."""
    p = np.asarray(x, np.float64) ** 2
    c = np.concatenate([[0.0], np.cumsum(p)])
    w = min(win, len(p))
    return float(np.max(c[w:] - c[:-w]) / w) + 1e-12


def noise_segment(noises, rng, n):
    src = noises[rng.integers(len(noises))]
    if len(src) <= n:
        src = np.tile(src, n // len(src) + 1)
    o = rng.integers(0, len(src) - n + 1)
    seg = src[o:o + n].astype(np.float64)
    return seg / np.sqrt(np.mean(seg ** 2) + 1e-12)


def to_pcm(x):
    return np.clip(np.round(x * 32768.0), -32768, 32767).astype(np.int16)


def make_stream(clip, rng, noises):
    """A training stream (int16) around one clip, or silence if clip is None."""
    pre = int(rng.integers(0, int(0.35 * SR)))
    post = int(rng.integers(0, int(0.35 * SR)))
    if clip is None:
        n = int(rng.integers(SR, int(1.7 * SR)))
        if rng.random() < 0.7:
            x = noise_segment(noises, rng, n) * 10 ** (rng.uniform(-75, -25) / 20)
        else:
            x = np.zeros(n)
        x = x + rng.standard_normal(n) * 10 ** (rng.uniform(-95, -60) / 20)
        return to_pcm(x)
    core = clip.astype(np.float64) / 32768.0
    shift = int(rng.integers(-1600, 1601))
    if shift > 0:
        core = np.concatenate([np.zeros(shift), core[:-shift]])
    elif shift < 0:
        core = np.concatenate([core[-shift:], np.zeros(-shift)])
    core = core * 10 ** (rng.uniform(-10, 10) / 20)
    x = np.concatenate([np.zeros(pre), core, np.zeros(post)])
    n = len(x)
    x = x + rng.standard_normal(n) * 10 ** (rng.uniform(-85, -65) / 20)
    if rng.random() < 0.75:
        snr = rng.uniform(3, 25)
        level = peak_power(core) / 10 ** (snr / 10)
        x = x + noise_segment(noises, rng, n) * np.sqrt(level)
    return to_pcm(x)


def stream_input(pcm, rng=None):
    """Raw (unnormalised) 48 x 40 log-mel window of a stream, chosen as on
    the K210; with rng, moved by up to +-2 frames half the time."""
    logmel, energy, power = F.analyse(pcm)
    s = F.choose_window(energy, power)
    if rng is not None and rng.random() < 0.5:
        s = int(np.clip(s + rng.integers(-2, 3), 0, max(0, len(energy) - F.WINDOW_FRAMES)))
    return F.window(logmel, s)


# --- workers (state set up per process) ---
_W = {}


def _init_worker(clips_path, noise_kind):
    _W["clips"] = np.load(clips_path, mmap_mode="r")
    tr, te = load_noise()
    _W["noise"] = tr if noise_kind == "train" else te


def _plain_job(i):
    return stream_input(_W["clips"][i])


def _aug_job(args):
    seed, variant, pos, i = args
    rng = np.random.default_rng([seed, variant, pos])
    clip = None if i < 0 else _W["clips"][i]
    return stream_input(make_stream(clip, rng, _W["noise"]), rng)


def _silence_eval_job(args):
    seed, i = args
    rng = np.random.default_rng([seed, i])
    noises = _W["noise"]
    if i % 4 == 3:
        x = rng.standard_normal(CLIP) * 10 ** (rng.uniform(-95, -60) / 20)
    else:
        x = noise_segment(noises, rng, CLIP) * 10 ** (rng.uniform(-75, -25) / 20)
        x = x + rng.standard_normal(CLIP) * 10 ** (-80 / 20)
    return stream_input(to_pcm(x))


def _map(job, items, clips_path, noise_kind):
    out = np.zeros((len(items), F.WINDOW_FRAMES, F.BANDS), np.float16)
    with Pool(os.cpu_count(), _init_worker, (clips_path, noise_kind)) as pool:
        for i, x in enumerate(pool.imap(job, items, chunksize=128)):
            out[i] = x
    return out


def cache_path(name):
    return os.path.join(CACHE, name)


def prepare():
    """Fetch and build the caches (index, clips, plain features, eval silence)."""
    fetch()
    os.makedirs(CACHE, exist_ok=True)
    if not os.path.exists(cache_path("index.json")):
        recs, orecs = build_index()
        root = os.path.join(WORK, ARCHIVES["train"][2])
        oroot = os.path.join(WORK, ARCHIVES["official_test"][2])
        clips, lengths = _load_clips([os.path.join(root, r["path"]) for r in recs])
        np.save(cache_path("clips.npy"), clips)
        for r, n in zip(recs, lengths):
            r["samples"] = int(n)
        oclips, olengths = _load_clips([os.path.join(oroot, r["path"]) for r in orecs])
        np.save(cache_path("official_clips.npy"), oclips)
        for r, n in zip(orecs, olengths):
            r["samples"] = int(n)
        with open(cache_path("index.json"), "w") as f:
            json.dump({"clips": recs, "official": orecs}, f)
    if not os.path.exists(cache_path("plain.npy")):
        idx = load_index()
        print("plain features for %d clips" % len(idx["clips"]))
        np.save(cache_path("plain.npy"),
                _map(_plain_job, range(len(idx["clips"])), cache_path("clips.npy"), "test"))
        np.save(cache_path("official_plain.npy"),
                _map(_plain_job, range(len(idx["official"])), cache_path("official_clips.npy"), "test"))
    if not os.path.exists(cache_path("silence_eval.npy")):
        np.save(cache_path("silence_eval.npy"),
                _map(_silence_eval_job, [(7001, i) for i in range(2 * SILENCE_EVAL)],
                     cache_path("clips.npy"), "test"))


def load_index():
    with open(cache_path("index.json")) as f:
        return json.load(f)


def train_items():
    """(clip indices, labels) of the training split, plus -1 for each
    _silence_ example."""
    idx = load_index()["clips"]
    ids = [i for i, r in enumerate(idx) if r["split"] == "train" and r["word"] in LABEL_INDEX]
    labels = [LABEL_INDEX[idx[i]["word"]] for i in ids]
    ids += [-1] * SILENCE_TRAIN
    labels += [LABEL_INDEX[SILENCE]] * SILENCE_TRAIN
    return np.array(ids), np.array(labels, np.int32)


def train_variant(seed, variant):
    """Augmented training inputs for one variant (cached), float16 [N,48,40]."""
    path = cache_path("train_s%d_v%d.npy" % (seed, variant))
    if not os.path.exists(path):
        ids, _ = train_items()
        x = _map(_aug_job, [(seed, variant, pos, int(i)) for pos, i in enumerate(ids)],
                 cache_path("clips.npy"), "train")
        np.save(path + ".tmp.npy", x)
        os.replace(path + ".tmp.npy", path)
    return np.load(path, mmap_mode="r")


def eval_set(split):
    """(inputs float16 [N,48,40], labels or -1, records) for 'val' or
    'test': the split's trained words plus its _silence_ examples. Held-out
    words are returned separately by heldout_set()."""
    idx = load_index()["clips"]
    plain = np.load(cache_path("plain.npy"), mmap_mode="r")
    ids = [i for i, r in enumerate(idx) if r["split"] == split and r["word"] in LABEL_INDEX]
    sil = np.load(cache_path("silence_eval.npy"))
    sil = sil[:SILENCE_EVAL] if split == "val" else sil[SILENCE_EVAL:]
    x = np.concatenate([plain[ids], sil])
    y = np.array([LABEL_INDEX[idx[i]["word"]] for i in ids] + [LABEL_INDEX[SILENCE]] * len(sil), np.int32)
    recs = [idx[i] for i in ids] + [{"word": SILENCE, "split": split, "speaker": ""}] * len(sil)
    return x, y, recs


if __name__ == "__main__":
    prepare()
    idx = load_index()
    by = {}
    for r in idx["clips"]:
        by[r["split"]] = by.get(r["split"], 0) + 1
    print("clips:", by, "official test:", len(idx["official"]))
    test_paths = {r["path"] for r in idx["clips"] if r["split"] == "test"}
    off = [r for r in idx["official"] if r["folder"] != SILENCE]
    same = sum(1 for r in off if (r["word"] + "/" + r["path"].split("/")[1].split(r["word"] + "_", 1)[-1]
                                  .replace(".wav.wav", ".wav")) in test_paths or r["path"] in test_paths)
    print("official test clips also in testing_list.txt: %d of %d" % (same, len(off)))

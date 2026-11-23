"""Evaluate the float TFLite model and the K210 kmodel; write kws.json.

    python3 evaluate.py [--model-dir DIR] [--kmodel FILE] [--ncc PATH] [--no-json]

Runs every validation and test clip of Speech Commands v0.02 (all 35
words), the _silence_ examples cut from the held-back 20 % of the noise
files, and the official 12-class test set (speech_commands_test_set_v0.02)
through the float model (kws.tflite, TFLite interpreter) and through the
kmodel (`ncc infer`, nncase's simulator of the K210 kmodel: uint8 KPU
layers, the same code paths as the K210 runtime). Inputs are
features.kws_input of each 1 s clip, normalised with norm.json: what the
K210 computes for a stream that holds just that clip.

Commands. The handheld's rule: top = argmax(probabilities); act if top is
one of the 20 commands and its probability >= the command threshold.
Reported per split:
  - top-1 accuracy over the 20 commands, and at the threshold the rates of
    accepted-correct, false reject and wrong command;
  - per-command top-1/accepted accuracy and the most frequent confusion,
    and the 31 x 31 confusion matrix (results.json) on the test split;
  - false accepts (any command accepted) of the 10 trained "other" words,
    of _silence_, and of the 5 never-trained words (backward, follow,
    forward, learn, visual), which stand in for speech the model never
    heard;
  - the official test set: 12-class accuracy (10 commands, _unknown_,
    _silence_; argmax mapped onto those) and the 20-command numbers on its
    clips (its _unknown_ folder holds the digits).
The command threshold is chosen on the validation split (each model's own
outputs) as the one minimising the sum of the five error rates (false
reject, wrong command, false accepts of other words, of never-trained
words and of silence); test numbers are reported at that threshold, and
the whole curve is in results.json.

Few-shot (user-trained words): speakers of the validation and test splits
(never seen in training) with >= 4 utterances of a held-out word. Each
such word is enrolled with 3 random utterances' embeddings (output 1,
L2-normalised); every other clip of those speakers (the rest of the
enrolled words = positives; all their other words = negatives) is matched
by cosine against the enrolled embeddings: "nn" takes the highest cosine
over all enrolled utterances (the brief's rule), "proto" the highest
cosine to each word's prototype (the normalised mean of its 3). Accepted
if the cosine >= tau. 10 random enrolments per speaker. tau is the lowest
on the grid with false accepts <= 2 %.

System: the same few-shot trials with commands and user words together,
at the chosen command threshold and tau, in two orders: user words first
(a user word wins if it matches, else the command rule) and commands
first. Reported: user words recognised / taken for a command; commands
recognised / hijacked by a user word; anything else acted on.

Writes results.json in the model directory and, unless --no-json,
kws.json next to this file (labels, input, normalisation, thresholds,
accuracies, kmodel facts, dataset licence).
"""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys

os.environ.setdefault("TF_USE_LEGACY_KERAS", "1")
os.environ.setdefault("TF_CPP_MIN_LOG_LEVEL", "2")

import numpy as np  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import dataset as D  # noqa: E402
import features as F  # noqa: E402

MODEL_DIR = os.path.join(D.WORK, "model")
EVAL_DIR = os.path.join(D.WORK, "eval")
NCMD = len(D.COMMANDS)
SIL = D.LABEL_INDEX[D.SILENCE]
GRID = [0.0] + [round(float(x), 2) for x in np.arange(0.30, 0.991, 0.05)] + [0.97, 0.98, 0.99]
TAU_GRID = [round(float(x), 3) for x in np.arange(0.40, 0.981, 0.02)]
FEWSHOT_FA = 0.02


def examples():
    """All evaluation inputs (unnormalised, float16) and their records."""
    idx = D.load_index()
    plain = np.load(D.cache_path("plain.npy"), mmap_mode="r")
    clip_ids = [i for i, r in enumerate(idx["clips"]) if r["split"] in ("val", "test")]
    recs = [dict(idx["clips"][i], set="clips") for i in clip_ids]
    xs = [plain[clip_ids]]
    sil = np.load(D.cache_path("silence_eval.npy"))
    for j in range(len(sil)):
        recs.append({"word": D.SILENCE, "split": "val" if j < D.SILENCE_EVAL else "test",
                     "speaker": "", "set": "clips"})
    xs.append(sil)
    xs.append(np.load(D.cache_path("official_plain.npy")))
    for r in idx["official"]:
        recs.append(dict(r, split="official", set="official"))
    return np.concatenate(xs), recs


def run_tflite(x, path):
    import tensorflow as tf
    it = tf.lite.Interpreter(model_path=path)
    it.allocate_tensors()
    i0 = it.get_input_details()[0]["index"]
    o = [d["index"] for d in it.get_output_details()]
    probs = np.zeros((len(x), len(D.LABELS)), np.float32)
    emb = np.zeros((len(x), 64), np.float32)
    for n in range(len(x)):
        it.set_tensor(i0, x[n:n + 1])
        it.invoke()
        probs[n] = it.get_tensor(o[0]).reshape(-1)
        emb[n] = it.get_tensor(o[1]).reshape(-1)
    return probs, emb


def run_kmodel(x, kmodel, ncc):
    """x: [N, 1, 48, 40] (NCHW). Cached by the kmodel's sha256."""
    digest = hashlib.sha256(open(kmodel, "rb").read()).hexdigest()[:16]
    cache = os.path.join(EVAL_DIR, "kmodel_%s.npz" % digest)
    if os.path.exists(cache):
        d = np.load(cache)
        if len(d["probs"]) == len(x):
            return d["probs"], d["emb"]
    din, dout = os.path.join(EVAL_DIR, "in"), os.path.join(EVAL_DIR, "out")
    for d in (din, dout):
        shutil.rmtree(d, ignore_errors=True)
        os.makedirs(d)
    for n in range(len(x)):
        x[n].astype("<f4").tofile(os.path.join(din, "%06d.bin" % n))
    subprocess.run([ncc, "infer", kmodel, dout, "--dataset", din, "--dataset-format", "raw"],
                   check=True, stdout=subprocess.DEVNULL)
    probs = np.zeros((len(x), len(D.LABELS)), np.float32)
    emb = np.zeros((len(x), 64), np.float32)
    for n in range(len(x)):
        v = np.fromfile(os.path.join(dout, "%06d.bin" % n), "<f4")
        probs[n], emb[n] = v[:len(D.LABELS)], v[len(D.LABELS):len(D.LABELS) + 64]
    shutil.rmtree(din)
    shutil.rmtree(dout)
    np.savez(cache, probs=probs, emb=emb)
    return probs, emb


def decide(probs, t):
    """The handheld's decision: command index, or -1 (ignore)."""
    top = probs.argmax(1)
    ok = (top < NCMD) & (probs.max(1) >= t)
    return np.where(ok, top, -1)


def command_metrics(probs, words, t):
    lab = np.array([D.LABEL_INDEX.get(w, -2) for w in words])
    dec = decide(probs, t)
    top = probs.argmax(1)
    cmd = (lab >= 0) & (lab < NCMD)
    oth = (lab >= NCMD) & (lab != SIL)
    sil = lab == SIL
    unseen = np.array([w in D.HELD_OUT for w in words])
    r = {"threshold": float(t), "commands": int(cmd.sum())}
    if cmd.any():
        r["top1_accuracy"] = float(np.mean(top[cmd] == lab[cmd]))
        r["accepted_correct"] = float(np.mean(dec[cmd] == lab[cmd]))
        r["false_reject"] = float(np.mean(dec[cmd] == -1))
        r["wrong_command"] = float(np.mean((dec[cmd] >= 0) & (dec[cmd] != lab[cmd])))
    for name, m in (("others", oth), ("silence", sil), ("unseen_words", unseen)):
        if m.any():
            r["false_accept_" + name] = float(np.mean(dec[m] >= 0))
            r["n_" + name] = int(m.sum())
    return r


def unseen_words(probs, words, t):
    """Per never-trained word: how often a command is accepted, and which."""
    out = {}
    dec = decide(probs, t)
    for w in D.HELD_OUT:
        m = words == w
        got = dec[m][dec[m] >= 0]
        cnt = np.bincount(got, minlength=NCMD) if len(got) else np.zeros(NCMD, int)
        top = [(D.LABELS[int(i)], int(cnt[i])) for i in np.argsort(-cnt)[:2] if cnt[i] > 0]
        out[w] = {"n": int(m.sum()), "accepted_as_command": round(float(np.mean(dec[m] >= 0)), 4),
                  "mostly_as": top}
    return out


def error_sum(m):
    return (m["false_reject"] + m["wrong_command"] + m["false_accept_others"]
            + m["false_accept_unseen_words"] + m["false_accept_silence"])


def per_command(probs, words, t):
    lab = np.array([D.LABEL_INDEX.get(w, -2) for w in words])
    top = probs.argmax(1)
    dec = decide(probs, t)
    out = {}
    conf = np.zeros((len(D.LABELS), len(D.LABELS)), int)
    for c in range(NCMD):
        m = lab == c
        wrong = top[m][top[m] != c]
        common = np.bincount(wrong, minlength=len(D.LABELS)) if len(wrong) else None
        out[D.LABELS[c]] = {
            "n": int(m.sum()), "top1": round(float(np.mean(top[m] == c)), 4),
            "accepted": round(float(np.mean(dec[m] == c)), 4),
            "most_confused_with": D.LABELS[int(common.argmax())] if common is not None else None}
    for a, b in zip(lab, top):
        if a >= 0:
            conf[a, b] += 1
    return out, conf.tolist()


def official_metrics(probs, recs, t):
    ten = D.COMMANDS[:10]
    want, got = [], []
    for r, k in zip(recs, probs.argmax(1)):
        want.append(r["folder"])
        lab = D.LABELS[k]
        got.append(lab if lab in ten else (D.SILENCE if lab == D.SILENCE else "_unknown_"))
    words = [r["word"] if r["folder"] != D.SILENCE else D.SILENCE for r in recs]
    m = command_metrics(probs, words, t)
    m["accuracy_12_class"] = float(np.mean([a == b for a, b in zip(want, got)]))
    return m


def fewshot_trials(emb, recs, mask, rounds=10, seed=5805):
    """Every (enrolment, query) trial: query index, true word, positive?,
    best nn cosine and its word, best prototype cosine and its word."""
    e = emb / np.maximum(np.linalg.norm(emb, axis=1, keepdims=True), 1e-9)
    by_spk = {}
    for n in np.flatnonzero(mask):
        by_spk.setdefault(recs[n]["speaker"], []).append(n)
    rng = np.random.default_rng(seed)
    cols = {k: [] for k in ("query", "true", "pos", "nn_sim", "nn_word", "proto_sim", "proto_word")}
    n_spk = n_words = 0
    for spk in sorted(by_spk):
        items = by_spk[spk]
        words = {}
        for n in items:
            if recs[n]["word"] in D.HELD_OUT:
                words.setdefault(recs[n]["word"], []).append(n)
        enrol = sorted(w for w, v in words.items() if len(v) >= 4)
        if not enrol:
            continue
        n_spk += 1
        n_words += len(enrol)
        for _ in range(rounds):
            picked, pw, protos = [], [], []
            for w in enrol:
                pick = list(rng.choice(words[w], size=3, replace=False))
                picked += pick
                pw += [w] * 3
                p = e[pick].mean(0)
                protos.append(p / max(np.linalg.norm(p), 1e-9))
            ps = set(picked)
            q = np.array([n for n in items if n not in ps])
            s_nn = e[q] @ e[picked].T
            s_pr = e[q] @ np.array(protos).T
            cols["query"].append(q)
            cols["true"] += [recs[n]["word"] for n in q]
            cols["pos"] += [recs[n]["word"] in enrol for n in q]
            cols["nn_sim"].append(s_nn.max(1))
            cols["nn_word"] += [pw[i] for i in s_nn.argmax(1)]
            cols["proto_sim"].append(s_pr.max(1))
            cols["proto_word"] += [enrol[i] for i in s_pr.argmax(1)]
    tr = {k: (np.concatenate(v) if k in ("query", "nn_sim", "proto_sim") else np.array(v))
          for k, v in cols.items()}
    return tr, n_spk, n_words


def fewshot_curve(tr, kind, rounds=10):
    sim, word, true, pos = tr[kind + "_sim"], tr[kind + "_word"], tr["true"], tr["pos"]
    out = {}
    for tau in TAU_GRID:
        acc = sim >= tau
        out[tau] = {"accuracy": float(np.mean((acc & (word == true))[pos])),
                    "confused": float(np.mean((acc & (word != true))[pos])),
                    "false_accept": float(np.mean(acc[~pos])),
                    "false_accept_trained_words": float(np.mean(acc[~pos & np.isin(true, D.LABELS)])),
                    "positives": int(pos.sum()) // rounds, "negatives": int((~pos).sum()) // rounds}
    return out


def system(tr, probs, t, tau, kind, user_first):
    q, sim, uw, true, pos = tr["query"], tr[kind + "_sim"], tr[kind + "_word"], tr["true"], tr["pos"]
    p = probs[q]
    top = p.argmax(1)
    cmd_ok = (top < NCMD) & (p.max(1) >= t)
    user_ok = sim >= tau
    if user_first:
        act_user, act_cmd = user_ok, ~user_ok & cmd_ok
    else:
        act_cmd, act_user = cmd_ok, ~cmd_ok & user_ok
    lab = np.array([D.LABEL_INDEX.get(w, -1) for w in true])
    is_cmd = (lab >= 0) & (lab < NCMD)
    rest = ~pos & ~is_cmd
    return {"order": "user words first" if user_first else "commands first",
            "user_word_recognised": float(np.mean((act_user & (uw == true))[pos])),
            "user_word_taken_as_command": float(np.mean(act_cmd[pos])),
            "command_recognised": float(np.mean((act_cmd & (top == lab))[is_cmd])),
            "command_hijacked_by_user_word": float(np.mean(act_user[is_cmd])),
            "other_speech_acted_on": float(np.mean((act_user | act_cmd)[rest]))}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ncc", default=os.environ.get("NCC", os.path.join(D.WORK, "ncc", "ncc")))
    ap.add_argument("--model-dir", default=MODEL_DIR)
    ap.add_argument("--kmodel", default=os.path.join(HERE, "kws.kmodel"))
    ap.add_argument("--no-json", action="store_true")
    args = ap.parse_args()
    os.makedirs(EVAL_DIR, exist_ok=True)
    with open(os.path.join(args.model_dir, "norm.json")) as f:
        norm = json.load(f)
    mean, inv_std = np.array(norm["mean"], np.float32), np.array(norm["inv_std"], np.float32)
    raw, recs = examples()
    x = ((raw.astype(np.float32) - mean) * inv_std)[:, :, :, None].astype(np.float32)
    print("%d evaluation inputs" % len(x), flush=True)
    outputs = {"float": run_tflite(x, os.path.join(args.model_dir, "kws.tflite")),
               "kmodel": run_kmodel(x[:, None, :, :, 0], args.kmodel, args.ncc)}

    split = np.array([r["split"] for r in recs])
    words = np.array([r["word"] for r in recs])
    fs_mask = np.isin(split, ["val", "test"]) & (words != D.SILENCE)
    res = {}
    for kind, (probs, emb) in outputs.items():
        rk = res[kind] = {}
        val = split == "val"
        curve = [command_metrics(probs[val], words[val], t) for t in GRID]
        rk["val_curve"] = curve
        rk["threshold"] = min(curve, key=error_sum)["threshold"]
        for s in ("val", "test"):
            m = split == s
            rk[s] = command_metrics(probs[m], words[m], rk["threshold"])
            rk[s + "_argmax"] = command_metrics(probs[m], words[m], 0.0)
        m = split == "test"
        rk["test_curve"] = [command_metrics(probs[m], words[m], t) for t in GRID]
        rk["per_command"], rk["confusion"] = per_command(probs[m], words[m], rk["threshold"])
        rk["unseen_per_word"] = unseen_words(probs[m], words[m], rk["threshold"])
        mo = split == "official"
        rk["official"] = official_metrics(probs[mo], [recs[i] for i in np.flatnonzero(mo)], rk["threshold"])
        tr, n_spk, n_words = fewshot_trials(emb, recs, fs_mask)
        for fk in ("nn", "proto"):
            fc = fewshot_curve(tr, fk)
            good = [t for t in TAU_GRID if fc[t]["false_accept"] <= FEWSHOT_FA]
            tau = good[0] if good else TAU_GRID[-1]
            rk["fewshot_" + fk + "_curve"] = {str(t): v for t, v in fc.items()}
            rk["fewshot_" + fk] = dict(fc[tau], tau=tau, speakers=n_spk, enrolled_words=n_words)
            rk["system_" + fk] = [dict(system(tr, probs, rk["threshold"], tau, fk, uf), tau=tau)
                                  for uf in (True, False)]
    pf, pk = outputs["float"][0], outputs["kmodel"][0]
    ef, ek = outputs["float"][1], outputs["kmodel"][1]
    cos = np.sum(ef * ek, 1) / np.maximum(np.linalg.norm(ef, axis=1) * np.linalg.norm(ek, axis=1), 1e-9)
    res["agreement"] = {"top1_same": float(np.mean(pf.argmax(1) == pk.argmax(1))),
                        "mean_abs_prob_diff": float(np.mean(np.abs(pf - pk))),
                        "top_prob_mean_abs_diff": float(np.mean(np.abs(pf.max(1) - pk.max(1)))),
                        "embedding_cosine_mean": float(np.mean(cos)),
                        "embedding_cosine_min": float(np.min(cos))}
    with open(os.path.join(args.model_dir, "results.json"), "w") as f:
        json.dump(res, f, indent=1)
    report(res)
    if not args.no_json:
        write_kws_json(res, norm, args.model_dir, args.kmodel)


def report(res):
    def r4(d):
        return {k: round(v, 4) if isinstance(v, float) else v for k, v in d.items()}
    for kind in ("float", "kmodel"):
        rk = res[kind]
        print("== %s: command threshold %.2f (from val)" % (kind, rk["threshold"]))
        for s in ("val_argmax", "val", "test_argmax", "test", "official"):
            print("  %-12s %s" % (s, r4(rk[s])))
        print("  never-trained words (test, at the threshold): %s" % rk["unseen_per_word"])
        print("  per command (test): %s" % {k: (v["top1"], v["accepted"], v["most_confused_with"])
                                            for k, v in rk["per_command"].items()})
        for fk in ("nn", "proto"):
            print("  few-shot %-5s %s" % (fk, r4(rk["fewshot_" + fk])))
            for s in rk["system_" + fk]:
                print("    system %s" % r4(s))
        print("  test curve:")
        for c in rk["test_curve"]:
            print("    t=%.2f acc %.4f fr %.4f wrong %.4f | FA others %.4f silence %.4f unseen %.4f | sum %.4f" % (
                c["threshold"], c["accepted_correct"], c["false_reject"], c["wrong_command"],
                c["false_accept_others"], c["false_accept_silence"], c["false_accept_unseen_words"],
                error_sum(c)))
        print("  few-shot curves (nn | proto):")
        for t in TAU_GRID[::2]:
            a, b = rk["fewshot_nn_curve"][str(t)], rk["fewshot_proto_curve"][str(t)]
            print("    tau=%.2f acc %.4f confused %.4f FA %.4f | acc %.4f confused %.4f FA %.4f" % (
                t, a["accuracy"], a["confused"], a["false_accept"],
                b["accuracy"], b["confused"], b["false_accept"]))
    print("agreement float vs kmodel:", r4(res["agreement"]))


def write_kws_json(res, norm, model_dir, kmodel):
    conv = json.load(open(os.path.join(model_dir, "convert", "convert.json")))
    hist = json.load(open(os.path.join(model_dir, "history.json")))
    km = res["kmodel"]
    data = open(kmodel, "rb").read()
    fs = km["fewshot_nn"]

    def rnd(d):
        return {k: (round(v, 4) if isinstance(v, float) else v) for k, v in d.items()}

    out = {
        "name": "Catalyst Coprocessor keyword spotter",
        "labels": D.LABELS,
        "commands": D.COMMANDS,
        "command_indices": list(range(NCMD)),
        "other_words": D.OTHERS,
        "silence_label": D.SILENCE,
        "held_out_words": D.HELD_OUT,
        "input": {"shape": [1, 1, F.WINDOW_FRAMES, F.BANDS], "dtype": "float32",
                  "layout": "48 rows (frames, 20 ms apart, oldest first) x 40 mel bands (low to high); "
                            "fe_window() writes it"},
        "outputs": [
            {"index": 0, "name": "probabilities", "shape": [1, len(D.LABELS)],
             "note": "softmax over labels; the kmodel computes exp through a uint8 table, so values "
                     "are coarse (steps of about 0.004)"},
            {"index": 1, "name": "embedding", "shape": [1, 64, 1, 1],
             "note": "global-average-pooled features, already centred and whitened (train.py); "
                     "L2-normalise, compare by cosine"}],
        "normalisation": {"formula": "(L[frame][band] - mean[band]) * inv_std[band]",
                          "mean": [round(v, 6) for v in norm["mean"]],
                          "inv_std": [round(v, 6) for v in norm["inv_std"]]},
        "frontend": {
            "sample_rate": F.SAMPLE_RATE, "window": F.WIN, "hop": F.HOP, "fft": F.WIN,
            "window_function": "periodic Hann", "bands": F.BANDS, "fmin_hz": F.FMIN, "fmax_hz": F.FMAX,
            "mel": "HTK 2595*log10(1+f/700), 42 edges evenly spaced in mel, triangles linear in mel, peak 1",
            "log": "ln(mel + 1e-6)", "energy": "10*log10(mean(x^2) + 1e-10) dB, x = s/32768",
            "vad": {"floor_start_db": "frame 0, at most %g" % F.VAD_FLOOR_CAP, "floor_rise_db_per_frame": F.VAD_RISE,
                    "margin_db": F.VAD_MARGIN, "min_db": F.VAD_MIN_DB, "onset_frames": F.VAD_ONSET,
                    "preroll_frames": F.VAD_PREROLL, "hangover_frames": F.VAD_HANG},
            "window_frames": F.WINDOW_FRAMES,
            "window_choice": "fe_best_window(s, vad_state ? vad_start : 0, fe_count(s) - 1)",
            "reference": "features.py; C: fe.c / fe.h"},
        "thresholds": {
            "command": km["threshold"],
            "command_rule": "top = argmax(probabilities); act if top < 20 and probabilities[top] >= command",
            "user_word_cosine": fs["tau"],
            "user_word_rule": "enrol 3 utterances (L2-normalised embeddings); take the highest cosine over "
                              "all enrolled embeddings; accept if >= user_word_cosine. Check user words "
                              "before the command rule (see accuracy.kmodel_system). Matching against each "
                              "word's prototype (the L2-normalised mean of its 3 embeddings) with "
                              "user_word_cosine_prototype does better (accuracy.kmodel_fewshot_prototype).",
            "user_word_cosine_prototype": km["fewshot_proto"]["tau"],
            "user_word_enrol_utterances": 3},
        "accuracy": {
            "note": "Speech Commands v0.02 testing_list.txt (speakers never trained on); commands = the 20; "
                    "thresholds from the validation split; kmodel = ncc infer (nncase's K210 simulator); "
                    "few-shot on validation+test speakers",
            "float_test_top1_commands": round(res["float"]["test_argmax"]["top1_accuracy"], 4),
            "kmodel_test_top1_commands": round(km["test_argmax"]["top1_accuracy"], 4),
            "kmodel_test_at_threshold": rnd(km["test"]),
            "kmodel_test_other_thresholds": [
                {k: round(c[k], 4) for k in ("threshold", "accepted_correct", "false_reject", "wrong_command",
                                             "false_accept_others", "false_accept_silence",
                                             "false_accept_unseen_words")}
                for c in km["test_curve"] if c["threshold"] in (0.5, 0.6, 0.7, 0.8, 0.9)],
            "float_test_at_threshold": rnd(res["float"]["test"]),
            "kmodel_official_test_set": rnd(km["official"]),
            "float_official_test_set": rnd(res["float"]["official"]),
            "kmodel_per_command": km["per_command"],
            "kmodel_never_trained_words": km["unseen_per_word"],
            "kmodel_fewshot": rnd(fs),
            "float_fewshot": rnd(res["float"]["fewshot_nn"]),
            "kmodel_fewshot_prototype": rnd(km["fewshot_proto"]),
            "kmodel_system": [rnd(s) for s in km["system_nn"]],
            "float_kmodel_agreement": rnd(res["agreement"])},
        "model": {
            "architecture": hist["architecture"],
            "note": "all convs 'same', stride 1; BN folded by the TFLite converter",
            "params": hist["params"], "macs": hist["macs"],
            "training": {k: hist["args"][k] for k in ("seed", "epochs", "arch", "outliers", "aux")
                         if k in hist["args"]},
            "best_val_accuracy": round(hist["best_val_acc"], 4)},
        "kmodel": {
            "file": "kws.kmodel", "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
            "format": "kmodel v4 (nncase v0.2), K210", "nncase": conv["nncase"],
            "ncc_archive_sha256": conv["ncc_archive_sha256"], "ncc_command": conv["ncc_command"],
            "rebuild_note": "ncc v0.2 leaves a few reserved/padding bytes uninitialised, so recompiling gives "
                            "a functionally identical kmodel with a different sha256",
            "calibration": "%d normalised windows of augmented training data" % conv["calibration_inputs"],
            "memory": conv["memory"],
            "memory_note": "working_memory_bytes: nncase's main-memory buffer (activations on the CPU side); "
                           "kpu_ram_bytes: KPU feature maps in the K210's 2 MB AI SRAM; plus the kmodel itself "
                           "(loaded into RAM) and the caller's 7680-byte float input",
            "layers": conv["layers"]},
        "dataset": {
            "name": D.DATASET,
            "urls": [v[0] for v in D.ARCHIVES.values()],
            "sha256": {v[0].rsplit("/", 1)[1]: v[1] for v in D.ARCHIVES.values()},
            "license": D.LICENSE, "attribution": D.ATTRIBUTION},
    }
    if hashlib.sha256(data).hexdigest() != conv["kmodel_sha256"]:
        raise SystemExit("%s is not the kmodel convert.json describes" % kmodel)
    with open(os.path.join(HERE, "kws.json"), "w", newline="\n") as f:
        json.dump(out, f, indent=1)
        f.write("\n")
    print("wrote", os.path.join(HERE, "kws.json"))


if __name__ == "__main__":
    main()

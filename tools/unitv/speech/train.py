"""Train the Catalyst Coprocessor keyword-spotting CNN (Keras, CPU).

    python3 train.py [--seed 5805] [--epochs 40] [--variants 12] [--arch 16p,32p,64p,96]

Reproducible: Python/numpy/TensorFlow seeds and TensorFlow's deterministic
ops are set from --seed; the augmented training sets are drawn from
per-example seeds (dataset.train_variant), so a rerun gives the same model
on the same machine and TensorFlow build. Needs dataset.py's caches (run
it first, or this runs it). Writes the work directory's model/ (or --out):
kws.h5 (the exported two-output model of the best epoch by validation
accuracy, embedding whitened, see below), norm.json (the per-band input
normalisation), history.json. About 10 minutes on 12 CPU cores, plus
about 10 minutes to make the 12 augmented training sets the first time.

The network (input 48 frames x 40 log-mel bands x 1, normalised per band),
as --arch 16p,32p,64p,96 (3x3 conv channels, p = 2x2 max pool after):

    conv3x3  16, BN, ReLU, maxpool 2x2   -> 24 x 20 x 16
    conv3x3  32, BN, ReLU, maxpool 2x2   -> 12 x 10 x 32
    conv3x3  64, BN, ReLU, maxpool 2x2   ->  6 x  5 x 64
    conv3x3  96, BN, ReLU                ->  6 x  5 x 96
    conv1x1  64 (+ bias; linear)         ->  6 x  5 x 64
    global average pool                  ->  64   output 1, the embedding
    dense 31, softmax                    ->  31   output 0, probabilities

All convs are stride 1, "same" padding, so nncase v0.2 maps each conv
(BN folded, ReLU, 2x2 max pool fused) onto the K210's KPU; it runs the
dense layer on the KPU too (as a 1x1 conv); the average pool and softmax
run on the CPU.

Training: each epoch uses one augmented variant of the training set
(cycled), SpecAugment-style masking (one time mask of 0..5 frames, one
band mask of 0..4 bands, each half the time), label smoothing 0.05,
dropout 0.2 before the dense layer, Adam with a 1-epoch warm-up and
cosine decay; the last 1x1 conv is trained with BN.

Whitened embedding: after training, the embeddings of the (unaugmented)
training clips give a mean mu and covariance S; with A = (S + 0.1 *
trace(S)/64 * I)^(-1/2), the last conv (BN folded in) is replaced by one
that outputs A (y - mu) and the dense layer's weights by A^-1 W (bias
b + mu W), so the probabilities are unchanged and the embedding output
is centred and decorrelated. That makes plain cosine matching of
user-trained words much better (evaluate.py, kmodel: 47 % -> 65 % of
utterances recognised at 2 % false accepts); the handheld only
L2-normalises it.

Tried and dropped (no gain on this data): outlier exposure (reversed and
spliced training words trained towards a uniform output, for rejecting
never-heard words) and a training-only cosine-softmax head on the
embedding; and a deeper variant (--arch 16p,32p,64,64p,96: 11 M MACs,
132 KB kmodel, +0.9 points command accuracy, worse user-word matching).
"""

import argparse
import json
import os
import random
import sys
import time

os.environ.setdefault("TF_USE_LEGACY_KERAS", "1")
os.environ.setdefault("TF_CPP_MIN_LOG_LEVEL", "2")

import numpy as np  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import dataset as D  # noqa: E402
import features as F  # noqa: E402

MODEL_DIR = os.path.join(D.WORK, "model")
ARCH = "16p,32p,64p,96"
EMB = 64
WHITEN_REG = 0.1


def describe(arch=ARCH, emb=EMB, n_classes=len(D.LABELS)):
    out = []
    for spec in arch.split(","):
        out.append("conv3x3 %s + BN + ReLU%s" % (spec.rstrip("p"), " + maxpool2" if spec.endswith("p") else ""))
    return out + ["conv1x1 %d (linear; output whitened)" % emb,
                  "global average pool (embedding, %d)" % emb, "dense %d + softmax" % n_classes]


def build_model(n_classes=len(D.LABELS), emb=EMB, seed=5805, arch=ARCH, deploy=False):
    """Outputs [probabilities, embedding]. arch: the 3x3 convs,
    comma-separated channel counts, 'p' = then a 2x2 max pool; a linear 1x1
    conv to `emb` channels follows (with BN for training; with a bias and
    no BN when deploy=True, for the whitened weights)."""
    import tf_keras as keras
    from tf_keras import layers as L

    seeds = iter(range(seed * 100, seed * 100 + 100))

    def init():
        # explicit seeds: tf_keras' own draw (random.randint(1, 1e9)) fails on Python 3.12
        return keras.initializers.HeUniform(seed=next(seeds))

    inp = keras.Input((F.WINDOW_FRAMES, F.BANDS, 1), name="logmel")
    x = inp
    for spec in arch.split(","):
        x = L.Conv2D(int(spec.rstrip("p")), 3, padding="same", use_bias=False, kernel_initializer=init())(x)
        x = L.BatchNormalization(momentum=0.99, epsilon=1e-3)(x)
        x = L.ReLU()(x)
        if spec.endswith("p"):
            x = L.MaxPooling2D(2)(x)
    x = L.Conv2D(emb, 1, use_bias=deploy, kernel_initializer=init(), name="emb_conv")(x)
    if not deploy:
        x = L.BatchNormalization(momentum=0.99, epsilon=1e-3, name="emb_bn")(x)
    e = L.GlobalAveragePooling2D(name="embedding")(x)
    d = L.Dropout(0.2, seed=next(seeds))(e)
    p = L.Dense(n_classes, activation="softmax", name="probs",
                kernel_initializer=keras.initializers.GlorotUniform(seed=next(seeds)))(d)
    return keras.Model(inp, [p, e], name="catalyst_kws")


def whiten(model, x, seed, arch):
    """The deploy model: same probabilities, whitened embedding."""
    probs, e = model.predict(x, batch_size=1024, verbose=0)
    e = e.astype(np.float64)
    mu = e.mean(0)
    cov = np.cov((e - mu).T, bias=True)
    vals, vecs = np.linalg.eigh(cov + WHITEN_REG * np.trace(cov) / EMB * np.eye(EMB))
    a = vecs @ np.diag(vals ** -0.5) @ vecs.T
    a_inv = vecs @ np.diag(vals ** 0.5) @ vecs.T
    k = model.get_layer("emb_conv").get_weights()[0].astype(np.float64)       # 1 x 1 x C x 64
    bn = model.get_layer("emb_bn")
    gamma, beta, mm, mv = [w.astype(np.float64) for w in bn.get_weights()]
    s = gamma / np.sqrt(mv + bn.epsilon)
    w_eff = k[0, 0] * s
    b_eff = beta - mm * s
    wd, bd = [w.astype(np.float64) for w in model.get_layer("probs").get_weights()]
    dep = build_model(seed=seed, arch=arch, deploy=True)
    skip = {"emb_conv", "emb_bn", "probs"}
    src = [layer for layer in model.layers if layer.weights and layer.name not in skip]
    dst = [layer for layer in dep.layers if layer.weights and layer.name not in skip]
    for a_l, b_l in zip(src, dst):
        b_l.set_weights(a_l.get_weights())
    dep.get_layer("emb_conv").set_weights([(w_eff @ a)[None, None].astype(np.float32),
                                           ((b_eff - mu) @ a).astype(np.float32)])
    dep.get_layer("probs").set_weights([(a_inv @ wd).astype(np.float32), (bd + mu @ wd).astype(np.float32)])
    p2, e2 = dep.predict(x[:4096], batch_size=1024, verbose=0)
    dp = float(np.abs(p2 - probs[:4096]).max())
    de = float(np.abs(e2 - (e[:4096] - mu) @ a).max())
    if dp > 1e-3 or de > 1e-2:
        raise SystemExit("whitening changed the model: max prob diff %g, embedding diff %g" % (dp, de))
    return dep, {"reg": WHITEN_REG, "clips": len(x), "max_prob_change": dp}


def macs(model):
    total = 0
    for layer in model.layers:
        cfg = layer.get_config()
        if layer.__class__.__name__ == "Conv2D":
            _, h, w, c = layer.output_shape
            kh, kw = cfg["kernel_size"]
            total += h * w * c * kh * kw * layer.input_shape[-1]
        elif layer.__class__.__name__ == "Dense":
            total += layer.input_shape[-1] * cfg["units"]
    return total


def masks(x, rng):
    """SpecAugment-style: zero (= the mean after normalisation) one run of
    frames and one run of bands, each with probability 0.5."""
    n = len(x)
    t = np.arange(F.WINDOW_FRAMES)[None, :]
    f = np.arange(F.BANDS)[None, :]
    tw = rng.integers(0, 6, n) * (rng.random(n) < 0.5)
    t0 = rng.integers(0, F.WINDOW_FRAMES - 5, n)
    fw = rng.integers(0, 5, n) * (rng.random(n) < 0.5)
    f0 = rng.integers(0, F.BANDS - 4, n)
    mt = (t >= t0[:, None]) & (t < (t0 + tw)[:, None])
    mf = (f >= f0[:, None]) & (f < (f0 + fw)[:, None])
    x[mt[:, :, None] | mf[:, None, :]] = 0.0
    return x


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=5805)
    ap.add_argument("--epochs", type=int, default=40)
    ap.add_argument("--variants", type=int, default=12)
    ap.add_argument("--batch", type=int, default=256)
    ap.add_argument("--lr", type=float, default=3e-3)
    ap.add_argument("--arch", default=ARCH)
    ap.add_argument("--out", default=MODEL_DIR)
    args = ap.parse_args()

    D.prepare()
    _, y_train = D.train_items()
    t0 = time.time()
    # (before TensorFlow is imported: the variants are made in forked workers)
    variants = [D.train_variant(args.seed, v) for v in range(args.variants)]
    print("training variants ready (%d x %d examples) in %.0f s"
          % (len(variants), len(y_train), time.time() - t0), flush=True)

    import tensorflow as tf
    import tf_keras as keras

    random.seed(args.seed)
    np.random.seed(args.seed)
    keras.utils.set_random_seed(args.seed)
    tf.config.experimental.enable_op_determinism()

    v0 = np.asarray(variants[0], np.float32)
    mean = v0.mean(axis=(0, 1)).astype(np.float32)
    std = v0.std(axis=(0, 1)).astype(np.float32)
    inv_std = (1.0 / np.maximum(std, 1e-3)).astype(np.float32)
    del v0
    out = args.out
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "norm.json"), "w") as f:
        json.dump({"mean": mean.tolist(), "inv_std": inv_std.tolist()}, f)

    def norm(x):
        return (np.asarray(x, np.float32) - mean) * inv_std

    xv, yv, _ = D.eval_set("val")
    xv = norm(xv)[..., None]

    model = build_model(seed=args.seed, arch=args.arch)
    model.summary(print_fn=lambda s: print(s))
    print("params %d, MACs %d" % (model.count_params(), macs(model)))
    steps = int(np.ceil(len(y_train) / args.batch))
    total = steps * args.epochs
    sched = keras.optimizers.schedules.CosineDecay(
        0.0, decay_steps=total - steps, alpha=0.01,
        warmup_target=args.lr, warmup_steps=steps)
    model.compile(optimizer=keras.optimizers.Adam(sched),
                  loss=[keras.losses.CategoricalCrossentropy(label_smoothing=0.05), None],
                  metrics=[["accuracy"], []])
    rng = np.random.default_rng(args.seed)
    eye = np.eye(len(D.LABELS), dtype=np.float32)
    best, hist = -1.0, []
    best_path = os.path.join(out, "best_trained.h5")
    for epoch in range(args.epochs):
        t1 = time.time()
        x = masks(norm(variants[epoch % len(variants)]), rng)[..., None]
        order = rng.permutation(len(x))
        h = model.fit(x[order], eye[y_train[order]], batch_size=args.batch, epochs=1,
                      shuffle=False, verbose=0)
        del x
        pv = model.predict(xv, batch_size=1024, verbose=0)[0]
        acc = float(np.mean(pv.argmax(1) == yv))
        cmd = yv < len(D.COMMANDS)
        cacc = float(np.mean(pv[cmd].argmax(1) == yv[cmd]))
        loss = float(h.history["loss"][-1])
        hist.append({"epoch": epoch + 1, "loss": loss, "train_acc": float(h.history["probs_accuracy"][-1]),
                     "val_acc": acc, "val_command_acc": cacc, "seconds": time.time() - t1})
        print("epoch %2d  loss %.4f  train %.4f  val %.4f  val commands %.4f  %.0f s"
              % (epoch + 1, loss, hist[-1]["train_acc"], acc, cacc, time.time() - t1), flush=True)
        if acc > best:
            best = acc
            model.save_weights(best_path)
    model.load_weights(best_path)

    idx = D.load_index()["clips"]
    plain = np.load(D.cache_path("plain.npy"), mmap_mode="r")
    ids = [i for i, r in enumerate(idx) if r["split"] == "train" and r["word"] in D.LABEL_INDEX]
    deploy, wh = whiten(model, norm(plain[ids])[..., None], args.seed, args.arch)
    deploy.save(os.path.join(out, "kws.h5"), include_optimizer=False)
    os.remove(best_path)
    with open(os.path.join(out, "history.json"), "w") as f:
        json.dump({"args": vars(args), "architecture": describe(args.arch),
                   "params": int(deploy.count_params()), "macs": int(macs(deploy)),
                   "best_val_acc": best, "whitening": wh, "epochs": hist}, f, indent=1)
    print("best val accuracy %.4f; whitened (%s); model in %s" % (best, wh, out))


if __name__ == "__main__":
    main()

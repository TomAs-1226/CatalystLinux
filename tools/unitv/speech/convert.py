"""Keras -> TFLite (float) -> K210 kmodel v4 with nncase v0.2.0-beta4.

    python3 convert.py [--model-dir DIR] [--out kws.kmodel] [--ncc PATH]
                       [--calib 400] [--calibrate-method l2]

Reads kws.h5 and norm.json from the model directory (train.py; default the
work directory's model/), writes kws.tflite there, calibration inputs
(real, augmented training windows, normalised, raw float32 NCHW =
1 x 1 x 48 x 40 each) in its convert/calib/, and compiles:

    ncc compile kws.tflite kws.kmodel -i tflite -o kmodel -t k210
        --dataset calib --dataset-format raw --inference-type uint8
        --input-type float --calibrate-method l2 --dump-ir

The KPU layers are uint8-quantised; the input stays float32 (the kmodel
quantises it on the CPU). The compile log and the IR graphs (.dot) go to
convert/; the kmodel is copied to --out (default: kws.kmodel next to
this file) and convert/convert.json records the flags, the nncase
version, the layer placement (KPU vs CPU, from the IR), the memory
nncase reports ("Working memory usage", = the kmodel header's main_mem)
and the KPU RAM the KPU layers use (read from their layer arguments).

ncc leaves a few reserved/padding bytes of the kmodel uninitialised, so
two compiles of the same model differ in about 8 bytes (and in sha256)
but not in behaviour; kws.json records the sha256 of the shipped file.

Two workarounds for nncase v0.2's TFLite importer are applied here: the
embedding output goes through a reshape (to 1 x 1 x 1 x 64), because a
graph output whose tensor also feeds another op is left unconnected (ncc
then crashes); and the subgraph's output list is reordered so the
probabilities are output 0 (the converter emits the embedding first).

ncc: $NCC, or the work directory's ncc/ncc (from ncc_linux_x86_64.tar.xz,
nncase v0.2.0-beta4, sha256 b20768e4...599ae).
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys

os.environ.setdefault("TF_USE_LEGACY_KERAS", "1")
os.environ.setdefault("TF_CPP_MIN_LOG_LEVEL", "2")

import numpy as np  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import dataset as D  # noqa: E402

MODEL_DIR = os.path.join(D.WORK, "model")
NCC_ARCHIVE_SHA256 = "b20768e444540aafd1896e72b88be7bad15302b371bf35f6039f08f6011599ae"
NCC_VERSION = "nncase v0.2.0-beta4 (ncc_linux_x86_64.tar.xz)"


def export_tflite(h5, out):
    import tensorflow as tf
    import tf_keras as keras
    model = keras.models.load_model(h5, compile=False)

    # The embedding output goes through a reshape to 1 x 1 x 1 x 64: nncase
    # v0.2's TFLite importer leaves a graph output unconnected when the same
    # tensor also feeds another op (here the dense layer), and then crashes.
    @tf.function(input_signature=[tf.TensorSpec([1] + list(model.input_shape[1:]), tf.float32)])
    def serve(x):
        p, e = model(x, training=False)
        return [p, tf.reshape(e, [1, 1, 1, -1])]

    conv = tf.lite.TFLiteConverter.from_concrete_functions([serve.get_concrete_function()], model)
    data = conv.convert()
    with open(out, "wb") as f:
        f.write(data)
    # The converter lists the subgraph's outputs in graph order (the
    # embedding is computed first); put the probabilities first, as output 0.
    from tensorflow.lite.tools import flatbuffer_utils as fbu
    m = fbu.read_model(out)
    sg = m.subgraphs[0]
    shapes = [list(sg.tensors[t].shape) for t in sg.outputs]
    if shapes[0][-1] != len(D.LABELS):
        sg.outputs = list(reversed(list(sg.outputs)))
        m.signatureDefs = []
        fbu.write_model(m, out)
    it = tf.lite.Interpreter(model_path=out)
    outs = [(o["name"], [int(v) for v in o["shape"]]) for o in it.get_output_details()]
    ins = [(i["name"], [int(v) for v in i["shape"]]) for i in it.get_input_details()]
    if outs[0][1][-1] != len(D.LABELS) or outs[1][1][-1] != 64:
        raise SystemExit("TFLite outputs are %s; want probabilities first" % outs)
    ops = sorted({d["op_name"] for d in it._get_ops_details()})
    return ins, outs, ops


def write_calibration(model_dir, conv_dir, n, seed):
    with open(os.path.join(model_dir, "norm.json")) as f:
        norm = json.load(f)
    mean, inv_std = np.array(norm["mean"], np.float32), np.array(norm["inv_std"], np.float32)
    ids, labels = D.train_items()
    x = D.train_variant(seed, 0)
    rng = np.random.default_rng(seed + 1)
    pick = []
    for c in range(len(D.LABELS)):
        members = np.flatnonzero(labels == c)
        pick += list(rng.choice(members, size=n // len(D.LABELS) + 1, replace=False))
    pick = sorted(pick)[:n] if len(pick) > n else sorted(pick)
    out = os.path.join(conv_dir, "calib")
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    for j, i in enumerate(pick):
        w = ((x[i].astype(np.float32) - mean) * inv_std).astype("<f4")
        w.tofile(os.path.join(out, "c%04d.bin" % j))
    return out, len(pick)


def placement(ir_dir):
    """The ops of the final IR nncase dumped (ir_lowering.dot), in order,
    with where they run and their input/output shapes."""
    path = os.path.join(ir_dir, "ir_lowering.dot")
    if not os.path.exists(path):
        return []
    text = open(path, encoding="utf-8", errors="replace").read()
    nodes = re.findall(r'^"([^"]+)" \[label="\{([A-Za-z0-9_]+)\}"\]', text, re.M)
    edges = re.findall(r'^"([^"]+)"->"([^"]+)" \[label="([^"]*)"\]', text, re.M)
    ins, outs = {}, {}
    for a, b, shape in edges:
        outs.setdefault(a, shape)
        ins.setdefault(b, shape)
    layers = []
    for name, op in nodes:
        if op in ("Input", "Output", "Constant", "Ignore"):
            continue
        unit = "KPU" if op == "KPUConv2D" else "CPU"
        layers.append({"op": op, "unit": unit, "in": ins.get(name), "out": outs.get(name)})
    return layers


def kmodel_header(data):
    """kmodel v4 header fields (nncase v0.2 runtime/model.h)."""
    f = struct.unpack_from("<4s9I", data, 0)
    return {"version": f[1], "target": f[3], "constants_bytes": f[4], "main_mem_bytes": f[5],
            "nodes": f[6], "inputs": f[7], "outputs": f[8]}


def kpu_ram(data):
    """Highest byte of the K210's KPU RAM (the 2 MB AI SRAM) the kmodel's
    KPU layers touch, from their kpu_layer_argument_t (image_addr,
    image_channel_num, channel/write-back strides; units of 64 bytes).
    nncase v0.2 does not print this."""
    _, _, _, _, consts, _, nodes, nin, nout, _ = struct.unpack_from("<4s9I", data, 0)
    off = 40 + nin * 32 + nout * 16 + consts
    heads = [struct.unpack_from("<2I", data, off + 8 * i) for i in range(nodes)]
    off += 8 * nodes
    top = 0

    def bits(v, at, n):
        return (v >> at) & ((1 << n) - 1)

    for op, size in heads:
        if op == 0x2002:   # k210 kpu_conv2d: memory_range, batches, reserved, then the layer argument
            a = struct.unpack_from("<12Q", data, off + 24)
            src, dst = bits(a[1], 0, 15), bits(a[1], 32, 15)
            ich, och = bits(a[2], 0, 10) + 1, bits(a[2], 32, 10) + 1
            top = max(top, src + ich * bits(a[7], 0, 15), dst + och * bits(a[8], 0, 15))
        off += size
    return top * 64


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ncc", default=os.environ.get("NCC", os.path.join(D.WORK, "ncc", "ncc")))
    ap.add_argument("--calib", type=int, default=400)
    ap.add_argument("--seed", type=int, default=5805)
    ap.add_argument("--calibrate-method", default="l2", choices=["no_clip", "l2"])
    ap.add_argument("--model-dir", default=MODEL_DIR)
    ap.add_argument("--out", default=os.path.join(HERE, "kws.kmodel"))
    args = ap.parse_args()

    conv_dir = os.path.join(args.model_dir, "convert")
    os.makedirs(conv_dir, exist_ok=True)
    tfl = os.path.join(args.model_dir, "kws.tflite")
    ins, outs, tfl_ops = export_tflite(os.path.join(args.model_dir, "kws.h5"), tfl)
    print("tflite:", ins, outs, tfl_ops)
    calib, ncal = write_calibration(args.model_dir, conv_dir, args.calib, args.seed)

    ir = os.path.join(conv_dir, "ir")
    shutil.rmtree(ir, ignore_errors=True)
    os.makedirs(ir)
    kmodel = os.path.join(conv_dir, "kws.kmodel")
    flags = ["-i", "tflite", "-o", "kmodel", "-t", "k210", "--dataset", calib,
             "--dataset-format", "raw", "--inference-type", "uint8", "--input-type", "float",
             "--calibrate-method", args.calibrate_method, "--dump-ir"]
    cmd = [args.ncc, "compile", tfl, kmodel] + flags
    print(" ".join(cmd))
    p = subprocess.run(cmd, cwd=ir, capture_output=True, text=True)
    log = p.stdout + p.stderr
    with open(os.path.join(conv_dir, "ncc.log"), "w") as f:
        f.write(log)
    print(log)
    if p.returncode != 0 or not os.path.exists(kmodel):
        raise SystemExit("ncc failed (%d)" % p.returncode)
    shutil.copyfile(kmodel, args.out)
    data = open(args.out, "rb").read()
    mem = {}
    for key, pat in (("working_memory_bytes", r"[Ww]orking memory usage:\s*(\d+)"),
                     ("main_memory_bytes", r"[Mm]ain memory usage:\s*(\d+)"),
                     ("kpu_memory_bytes", r"KPU memory usage:\s*(\d+)"),
                     ("constant_bytes", r"[Cc]onstant usage:\s*(\d+)")):
        m = re.search(pat, log)
        if m:
            mem[key] = int(m.group(1))
    layers = placement(ir)
    hdr = kmodel_header(data)
    mem["kmodel_main_mem_bytes"] = hdr["main_mem_bytes"]
    mem["kpu_ram_bytes"] = kpu_ram(data)
    info = {
        "nncase": NCC_VERSION, "ncc_archive_sha256": NCC_ARCHIVE_SHA256,
        "ncc_command": "ncc compile kws.tflite kws.kmodel " + " ".join(
            "calib/" if a == calib else a for a in flags),
        "calibration_inputs": ncal, "tflite_ops": tfl_ops,
        "tflite_inputs": ins, "tflite_outputs": outs,
        "kmodel_bytes": len(data), "kmodel_sha256": hashlib.sha256(data).hexdigest(),
        "kmodel_header": hdr, "memory": mem, "layers": layers,
    }
    with open(os.path.join(conv_dir, "convert.json"), "w") as f:
        json.dump(info, f, indent=1)
    print(json.dumps({k: v for k, v in info.items() if k != "layers"}, indent=1))
    for layer in layers:
        print("  %-4s %-14s %-12s -> %s" % (layer["unit"], layer["op"], layer["in"], layer["out"]))


if __name__ == "__main__":
    main()

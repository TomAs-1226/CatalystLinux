#!/bin/sh
# The offline assistant's evaluation on a development PC: llama.cpp's
# llama-server (the release the image builds, b8117) and the small models it
# is tried with, kept in a tools folder outside the repository, then the
# questions of `--offline-eval` asked of each model in turn.
#
#   scripts/offline-eval.sh fetch            # llama-server and the models
#   scripts/offline-eval.sh run [model...]   # the evaluation (all by default)
#
# TOOLS (default ~/catalyst-offline-eval) holds llama/, models/ and results/.
# The simulator is built with the `sim` profile into CARGO_TARGET_DIR.
set -eu

TOOLS=${TOOLS:-$HOME/catalyst-offline-eval}
LLAMA_TAG=b8117
PORT=${PORT:-8091}
HERE=$(cd "$(dirname "$0")/.." && pwd)

# (Hugging Face repository, file) for each model tried: 135M to 1.5B.
MODELS="
bartowski/SmolLM2-135M-Instruct-GGUF SmolLM2-135M-Instruct-Q8_0.gguf
unsloth/gemma-3-270m-it-GGUF gemma-3-270m-it-Q8_0.gguf
LiquidAI/LFM2-350M-GGUF LFM2-350M-Q4_K_M.gguf
bartowski/SmolLM2-360M-Instruct-GGUF SmolLM2-360M-Instruct-Q4_K_M.gguf
unsloth/Qwen3-0.6B-GGUF Qwen3-0.6B-Q4_K_M.gguf
LiquidAI/LFM2-700M-GGUF LFM2-700M-Q4_K_M.gguf
unsloth/Qwen3.5-0.8B-GGUF Qwen3.5-0.8B-Q4_K_M.gguf
unsloth/gemma-3-1b-it-GGUF gemma-3-1b-it-Q4_K_M.gguf
LiquidAI/LFM2.5-1.2B-Instruct-GGUF LFM2.5-1.2B-Instruct-Q4_K_M.gguf
bartowski/Qwen2.5-1.5B-Instruct-GGUF Qwen2.5-1.5B-Instruct-Q4_K_M.gguf
"

fetch() {
    mkdir -p "$TOOLS/llama" "$TOOLS/models"
    if [ ! -e "$TOOLS/llama/llama-server.exe" ] && [ ! -e "$TOOLS/llama/llama-server" ]; then
        case "$(uname -s)" in
            MINGW* | MSYS* | CYGWIN*) asset=llama-$LLAMA_TAG-bin-win-cpu-x64.zip ;;
            *) asset=llama-$LLAMA_TAG-bin-ubuntu-x64.zip ;;
        esac
        curl -sSL -o "$TOOLS/$asset" "https://github.com/ggml-org/llama.cpp/releases/download/$LLAMA_TAG/$asset"
        (cd "$TOOLS/llama" && unzip -q -o "../$asset")
    fi
    echo "$MODELS" | while read -r repo file; do
        [ -n "$repo" ] || continue
        if [ -s "$TOOLS/models/$file" ]; then
            echo "have $file"
            continue
        fi
        curl -sSL --retry 3 -o "$TOOLS/models/$file.part" "https://huggingface.co/$repo/resolve/main/$file" &&
            mv "$TOOLS/models/$file.part" "$TOOLS/models/$file" && echo "got $file" || echo "FAILED $file"
    done
}

server_bin() {
    for b in "$TOOLS/llama/llama-server.exe" "$TOOLS/llama/llama-server" "$TOOLS"/llama/*/llama-server; do
        [ -x "$b" ] && { echo "$b"; return; }
    done
    echo "no llama-server in $TOOLS/llama: run '$0 fetch'" >&2
    exit 1
}

run() {
    bin=$(server_bin)
    mkdir -p "$TOOLS/results"
    # The simulator built with `--profile sim`, or SIM naming one (a copy,
    # so building again while this runs does not trip over it).
    sim=${SIM:-${CARGO_TARGET_DIR:-$HERE/userland/target}/sim/catalyst-shell}
    [ -e "$sim.exe" ] && sim=$sim.exe
    wanted=${*:-}
    echo "$MODELS" | while read -r repo file; do
        [ -n "$file" ] || continue
        if [ -n "$wanted" ] && ! echo " $wanted " | grep -qi " ${file%%-*}\| $file "; then
            continue
        fi
        # As the handheld runs it: a 2048 context, one conversation, an
        # 8-bit cache; three threads here too, so prompt sizes compare.
        "$bin" -m "$TOOLS/models/$file" --host 127.0.0.1 --port "$PORT" -c 2048 -t 3 -np 1 \
            -b 256 -ub 128 -fa on -ctk q8_0 -ctv q8_0 --no-warmup >"$TOOLS/results/${file%.gguf}.server.log" 2>&1 &
        pid=$!
        for _ in $(seq 1 120); do
            curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break
            sleep 1
        done
        CATALYST_LOCAL_URL="http://127.0.0.1:$PORT/v1" CATALYST_EVAL_MODEL="${file%.gguf}" \
            "$sim" --offline-eval "$TOOLS/results/${file%.gguf}.json" || echo "eval failed for $file"
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    done
}

case "${1:-}" in
    fetch) fetch ;;
    run) shift; run "$@" ;;
    *) echo "usage: $0 fetch | run [model...]" >&2; exit 2 ;;
esac

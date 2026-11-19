#!/bin/bash
# Run an aarch64 program built against the image's musl, under user-mode
# emulation: through the image's own loader and libraries, with every other
# path (/tmp, /dev, programs it starts) left as the host's. (qemu's -L would
# send every absolute path into the image's root instead.)
ROOT="${CATALYST_BUILD:-$HOME/catalyst-linux-build}/output/target"
exec qemu-aarch64-static "$ROOT/lib/ld-musl-aarch64.so.1" --library-path "$ROOT/lib:$ROOT/usr/lib" "$@"

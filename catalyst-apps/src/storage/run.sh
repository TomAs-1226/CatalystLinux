#!/bin/sh
# What is using the card: the data partition's size and what is left, then
# its largest folders.

df -k /data | tail -1 | awk '{ printf "data: %.0f of %.0f mb used, %.0f mb free\n\n", $3 / 1024, $2 / 1024, $4 / 1024 }'
du -sk /data/* 2>/dev/null | sort -rn | head -8 | awk '{ printf "%9.1f mb  %s\n", $1 / 1024, $2 }'

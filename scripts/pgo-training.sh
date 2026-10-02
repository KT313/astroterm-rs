#!/bin/sh
# Run an instrumented astroterm binary through typical workloads, so PGO and BOLT can record which code is hot.
# Each run happens in a 120x45 pseudo-terminal (via `script`) at an unlimited frame rate, gets a few keys, and is quit
# with `q` so the profile is written on a normal exit.
#
# Usage: scripts/pgo-training.sh <binary>    (TRAINING_SECONDS sets the length of each run, default 6)

set -eu

binary=$1
seconds=${TRAINING_SECONDS:-6}

# run the binary with the given options, pressing `keys` halfway through, then quit
train() {
    keys=$1
    shift
    echo "  training: $*"
    half=$(awk "BEGIN { print $seconds / 2 }")
    { sleep "$half"; printf '%s' "$keys"; sleep "$half"; printf q; } |
        script -qec "stty cols 120 rows 45; $binary --fps 100000 $*" /dev/null >/dev/null
}

train '++hhkk' -i Tokyo -d 2025-03-01T20:00:00 -s 10000 -cCu -m
train ']]r0' -i Tokyo -s 100000 -cCub -g --debug-frametimes
train '--jjll' -a -33.87 -o 151.21 -F NNW -T 20 -z 120 -s 1000 -u -R -m
train ' ' -i Boston -e -z 300 -F N -s -5000 -cC
train '+' -i Oslo -t 8 -l 3 -s 100 -C

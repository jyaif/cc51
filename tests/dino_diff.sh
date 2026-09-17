#!/bin/bash
# Build the dino game with cc51 and compare its video-register write trace with the SDCC build.
cd "$(dirname "$0")/.."
. ./dino_args.sh
CC=${CC:-./target/release/cc51}
T=${TMPDIR:-/tmp}/cc51-dino
mkdir -p $T
CYCLES=${CYCLES:-100000000}
$CC $DINO_ARGS -o $T/dino.ihx --lst $T/dino.lst --map $T/dino.map || exit 1
SDCC_HEX=$HOME/Code/minitel-native/examples/dino_game/out/dino_game.ihx
[ -f $T/sdcc.trace ] || ./target/release/minitel_sim $SDCC_HEX $T/sdcc.trace $CYCLES
./target/release/minitel_sim $T/dino.ihx $T/cc51.trace $CYCLES
n=$(wc -l < $T/cc51.trace)
if head -n $n $T/sdcc.trace | cmp -s - $T/cc51.trace; then
  echo "dino: OK ($n writes match)"
else
  echo "dino: MISMATCH"
  head -n $n $T/sdcc.trace | cmp - $T/cc51.trace
  exit 1
fi

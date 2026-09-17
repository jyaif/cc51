#!/bin/bash
# Compile a test with SDCC and run it in sim51 to obtain reference output.
cd "$(dirname "$0")/.."
TMP=${TMPDIR:-/tmp}/cc51-sdcc
mkdir -p $TMP
f=$1
name=$(basename $f .c)
sdcc -mmcs51 --std-sdcc2x -Itests/c -o $TMP/ $f >$TMP/$name.sdccout 2>&1 || \
  sdcc -mmcs51 --model-large --std-sdcc2x -Itests/c -o $TMP/ $f >$TMP/$name.sdccout 2>&1 || { cat $TMP/$name.sdccout; exit 1; }
./target/debug/sim51 --max-cycles 50000000 -v $TMP/$name.ihx

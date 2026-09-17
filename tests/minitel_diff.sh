#!/bin/bash
# Compare video-write traces of the SDCC and cc51 CMake builds produced by minitel_all.sh.
cd "$(dirname "$0")/.."
B=${TMPDIR:-/tmp}/minitel-builds
CYCLES=${CYCLES:-60000000}
rc=0
for ex in ${EXAMPLES:-dino dino_game}; do
  for model in nfz330 nfz400 722039m; do
    s=$B/$ex-$model-sdcc/$ex.ihx
    c=$B/$ex-$model-cc51/$ex.ihx
    [ -f $s ] && [ -f $c ] || { echo "$ex $model: missing build"; rc=1; continue; }
    ./target/release/minitel_sim $s $B/$ex-$model.sdcc.trace $CYCLES $model 2>/dev/null
    ./target/release/minitel_sim $c $B/$ex-$model.cc51.trace $CYCLES $model 2>/dev/null
    n=$(wc -l < $B/$ex-$model.cc51.trace)
    m=$(wc -l < $B/$ex-$model.sdcc.trace)
    [ $n -gt $m ] && n=$m
    if cmp -s <(head -n $n $B/$ex-$model.sdcc.trace) <(head -n $n $B/$ex-$model.cc51.trace); then
      echo "$ex $model: OK ($n writes)"
    else
      echo "$ex $model: MISMATCH"; rc=1
    fi
  done
done
exit $rc

#!/bin/bash
# Build all minitel-native examples for all models with SDCC and with cc51 (via CMake), report sizes.
M=${M:-$HOME/Code/minitel-native}
CC51=$(cd "$(dirname "$0")/.." && pwd)
B=${TMPDIR:-/tmp}/minitel-builds
mkdir -p $B
printf "%-14s %-9s %8s %8s %7s\n" example model sdcc cc51 ratio
for ex in ${EXAMPLES:-hello_world video_stream dino dino_game}; do
  for model in nfz330 nfz400 722039m; do
    for tc in sdcc cc51; do
      d=$B/$ex-$model-$tc
      rm -rf $d; mkdir -p $d
      if [ $tc = cc51 ]; then P=$CC51/compat/bin:$PATH; else P=$PATH; fi
      ( cd $d && PATH=$P cmake $M/examples/$ex -DMINITEL_MODEL=$model >cmake.log 2>&1 && PATH=$P make -j4 >make.log 2>&1 ) || echo "  build failed: $ex $model $tc ($d)"
    done
    s1=$(stat -f%z $B/$ex-$model-sdcc/$ex.bin 2>/dev/null || echo 0)
    s2=$(stat -f%z $B/$ex-$model-cc51/$ex.bin 2>/dev/null || echo 0)
    r=$(python3 -c "print(f'{$s2/$s1*100:.1f}%' if $s1 else '-')")
    printf "%-14s %-9s %8s %8s %7s\n" $ex $model $s1 $s2 $r
  done
done

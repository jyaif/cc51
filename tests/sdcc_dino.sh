#!/bin/bash
# Build the dino game with SDCC for a given MODEL (for size comparison / reference traces).
cd "$(dirname "$0")/.."
. ./dino_args.sh
OUT=${TMPDIR:-/tmp}/sdcc-dino-$MODEL
rm -rf $OUT; mkdir -p $OUT
rels=""
libs=""
for f in $DINO_SRCS; do
  b=$(basename $f .c)
  sdcc -mmcs51 --std-sdcc2x $DINO_INC -c $f -o $OUT/$b.rel >/dev/null 2>$OUT/$b.err || { cat $OUT/$b.err; exit 1; }
  case $f in
    */lib/*) libs="$libs $OUT/$b.rel" ;;
    *) rels="$rels $OUT/$b.rel" ;;
  esac
done
sdar -rc $OUT/minitel.lib $libs
sdcc -mmcs51 --std-sdcc2x -o $OUT/dino.ihx $rels -L $OUT -l minitel.lib >/dev/null 2>&1 || exit 1
grep "ROM/EPROM" $OUT/dino.mem
echo $OUT/dino.ihx

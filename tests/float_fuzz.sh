#!/bin/bash
# Check both soft float variants (--float=fast|small) bit for bit against the reference model.
cd "$(dirname "$0")/.."
CC=${CC:-./target/release/cc51}
SIM=./target/release/sim51
T=${TMPDIR:-/tmp}/cc51-float
mkdir -p $T
python3 tests/float_model.py > $T/expect.txt || exit 1
ok=0
for v in fast small; do
  $CC --float=$v tests/float_fuzz.c -o $T/$v.ihx || { ok=1; continue; }
  $SIM --max-cycles 1000000000 $T/$v.ihx > $T/$v.txt
  if cmp -s $T/$v.txt $T/expect.txt; then
    echo "float $v: OK ($(wc -l < $T/expect.txt | tr -d ' ') lines match)"
  else
    echo "float $v: MISMATCH"
    diff $T/$v.txt $T/expect.txt | head -5
    ok=1
  fi
done
exit $ok

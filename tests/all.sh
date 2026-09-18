#!/bin/bash
# Full regression: C unit tests (debug build), soft float fuzz and dino differential tests for all boards (release build).
cd "$(dirname "$0")/.."
cargo build -q 2>&1 | grep -E "^error" && exit 1
cargo build -q --release 2>&1 | grep -E "^error" && exit 1
ok=0
./tests/run.sh || ok=1
./tests/float_fuzz.sh || ok=1
for m in nfz330 nfz400 722039m; do
  out=$(MODEL=$m ./tests/dino_diff.sh 2>&1) || ok=1
  echo "$out" | grep -E "^(dino|code)" | tr '\n' ' '; echo
done
exit $ok

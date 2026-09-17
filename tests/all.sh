#!/bin/bash
# Full regression: C unit tests (debug build) and dino differential tests for all boards (release build).
cd "$(dirname "$0")/.."
cargo build -q 2>&1 | grep -E "^error" && exit 1
cargo build -q --release 2>&1 | grep -E "^error" && exit 1
ok=0
./tests/run.sh || ok=1
for m in nfz330 nfz400 722039m; do
  MODEL=$m ./tests/dino_diff.sh 2>&1 | grep -v "^/" | grep -v "^code:" || true
  MODEL=$m ./tests/dino_diff.sh >/dev/null 2>&1 || ok=1
done
exit $ok

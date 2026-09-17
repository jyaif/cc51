#!/bin/bash
# Run the C test suite: compile each tests/c/*.c with cc51, simulate, compare with the EXPECT block.
cd "$(dirname "$0")/.."
CC=${CC:-./target/debug/cc51}
SIM=./target/debug/sim51
TMP=${TMPDIR:-/tmp}/cc51-tests
mkdir -p $TMP
pass=0; fail=0; failed=()
for f in tests/c/*.c; do
  name=$(basename $f .c)
  [ -n "$1" ] && [[ "$name" != $1 ]] && continue
  expected=$(awk '/^\/\* EXPECT:/{flag=1;next} /^\*\//{flag=0} flag' $f)
  [ -z "$expected" ] && continue
  if ! $CC $CCFLAGS $f -o $TMP/$name.ihx --lst $TMP/$name.lst --map $TMP/$name.map 2>$TMP/$name.err >/dev/null; then
    fail=$((fail+1)); failed+=("$name(compile)"); continue
  fi
  actual=$($SIM --max-cycles 50000000 $TMP/$name.ihx 2>$TMP/$name.simerr)
  rc=$?
  if [ "$actual" == "$expected" ] && [ $rc -eq 0 ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed+=("$name")
    if [ -n "$VERBOSE" ]; then
      echo "--- $name: expected"; echo "$expected"; echo "--- got (rc=$rc)"; echo "$actual"; cat $TMP/$name.simerr
    fi
  fi
done
echo "passed: $pass, failed: $fail"
for n in "${failed[@]}"; do echo "  FAIL $n"; done
[ $fail -eq 0 ]

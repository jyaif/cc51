#!/usr/bin/env python3
"""Regenerate the EXPECT block of test files from SDCC's output."""
import subprocess, sys, re, os
os.chdir(os.path.join(os.path.dirname(__file__), '..'))
for f in sys.argv[1:]:
    r = subprocess.run(['./tests/sdcc_ref.sh', f], capture_output=True, text=True)
    if r.returncode != 0:
        print(f"{f}: sdcc/sim failed\n{r.stdout}{r.stderr}")
        continue
    out = r.stdout
    src = open(f).read()
    src = re.sub(r'/\* EXPECT:\n.*?\*/\n?', '', src, flags=re.S)
    src = src.rstrip('\n') + '\n/* EXPECT:\n' + out + ('' if out.endswith('\n') else '\n') + '*/\n'
    open(f, 'w').write(src)
    print(f"{f}: updated ({len(out.splitlines())} lines)")

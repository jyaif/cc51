#!/usr/bin/env python3
"""Run SDCC's regression suite with cc51 + sim51.

usage: run.py [-j N] [-v] [--failed] [pattern...]
Env: SDCC_REG = path to sdcc/support/regression, OUT = work dir.
"""
import os, sys, subprocess, glob, re, fnmatch
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
REG = os.environ.get('SDCC_REG') or sys.exit('set SDCC_REG')
OUT = os.environ.get('OUT', '/tmp/cc51-sdccreg')
CC = os.environ.get('CC', os.path.join(ROOT, 'target/release/cc51'))
SIM = os.path.join(ROOT, 'target/release/sim51')
sys.path.insert(0, HERE)
import gen

args = sys.argv[1:]
jobs, verbose, pats, failed_only = os.cpu_count(), False, [], False
while args:
    a = args.pop(0)
    if a == '-j': jobs = int(args.pop(0))
    elif a == '-v': verbose = True
    elif a == '--failed': failed_only = True
    else: pats.append(a)

srcs = sorted(glob.glob(os.path.join(REG, 'tests', '*.c')) + glob.glob(os.path.join(REG, 'tests', '*.c.in')))
if pats:
    srcs = [s for s in srcs if any(fnmatch.fnmatch(os.path.basename(s).split('.')[0], p) for p in pats)]
os.makedirs(OUT, exist_ok=True)
insts = []
for s in srcs:
    insts += gen.generate(s, os.path.join(OUT, 'src'))

# Tests whose SDCC-specific expectations we knowingly do not match.
EXPECTED_DIFF = {
    # SDCC's small-model printf prints "<NO FLOAT>"; cc51 prints the value.
    'snprintf_type_FLOAT',
}

prev = {}
if os.path.exists(os.path.join(OUT, 'results.txt')):
    for l in open(os.path.join(OUT, 'results.txt')):
        w = l.split()
        if len(w) >= 2 and w[0] in ('PASS', 'FAIL', 'CERR', 'CTIMEOUT', 'STIMEOUT', 'NOSUMMARY'):
            prev[w[1]] = w[0]
if failed_only:
    insts = [p for p in insts if prev.get(os.path.basename(p)[:-2], 'PASS') not in ('PASS', 'XFAIL')]

def run(path):
    name = os.path.basename(path)[:-2]
    d = os.path.join(OUT, 'build'); os.makedirs(d, exist_ok=True)
    ihx = os.path.join(d, name + '.ihx')
    cmd = [CC, '-I', os.path.join(REG, 'fwk/include'), '-I', os.path.join(REG, 'tests'), '-I', os.path.join(HERE, 'include'),
           path, os.path.join(REG, 'fwk/lib/testfwk.c'), os.path.join(REG, 'fwk/lib/statics.c'),
           os.path.join(REG, 'fwk/lib/extern1.c'), os.path.join(REG, 'fwk/lib/extern2.c'),
           os.path.join(HERE, 'support.c'), '-o', ihx,
           '--lst', os.path.join(d, name + '.lst')]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return name, 'CTIMEOUT', ''
    if r.returncode != 0:
        lines = r.stderr.strip().splitlines() or ['?']
        errs = [l for l in lines if 'error' in l or 'panicked' in l] or lines
        msg = errs[0].replace(os.path.join(OUT, 'src') + '/', '')
        return name, 'CERR', msg if not verbose else r.stderr
    try:
        r = subprocess.run([SIM, '--max-cycles', '200000000', ihx], capture_output=True, timeout=120)
    except subprocess.TimeoutExpired:
        return name, 'STIMEOUT', ''
    out = r.stdout.decode('latin-1')
    m = re.search(r'--- Summary: (\d+)/(\d+)/(\d+)', out)
    if not m:
        return name, 'NOSUMMARY', (r.stderr.decode() + out[-300:])
    if int(m.group(1)) != 0:
        return name, 'FAIL', '\n'.join(l.replace(os.path.join(OUT, 'src') + '/', '') for l in out.splitlines() if 'FAIL' in l)
    return name, 'PASS', m.group(2)

counts = {}
with ThreadPoolExecutor(jobs) as ex:
    results = sorted(ex.map(run, insts))
results = [(n, 'XFAIL' if (st == 'FAIL' and n in EXPECTED_DIFF) else st, m) for n, st, m in results]
for name, st, msg in results:
    if prev.get(name) == 'PASS' and st != 'PASS':
        print(f'REGRESSION {name}')
    prev[name] = st
with open(os.path.join(OUT, 'results.txt'), 'w') as f:
    for name in sorted(prev):
        f.write(f'{prev[name]} {name}\n')
for name, st, msg in results:
        counts[st] = counts.get(st, 0) + 1
        if st not in ('PASS', 'XFAIL'):
            print(f'{st:9} {name}: {msg}' if verbose else f'{st:9} {name}: {msg.splitlines()[0] if msg else ""}')
print(counts)

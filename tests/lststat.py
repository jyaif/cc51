#!/usr/bin/env python3
"""Instruction pattern statistics from a cc51 listing."""
import sys, re, collections
cnt=collections.Counter(); byt=collections.Counter(); pat=collections.Counter()
total=0
for line in open(sys.argv[1]):
    m=re.match(r'^[0-9a-f]{4}  ((?:[0-9a-f]{2} ?)+)\s*\t(\S+)\s*(.*)$', line)
    if not m: continue
    n=len(m.group(1).split()); mn=m.group(2); ops=m.group(3)
    if mn=='.db': continue
    total+=n
    cnt[mn]+=1; byt[mn]+=n
    shape=re.sub(r'#[^,]+','#imm',ops)
    shape=re.sub(r'\b(__s\d+_\d+|__o\d+_\d+(?: \+ \d+)?|_[A-Za-z]\w*(?: \+ \d+)?|0x[0-9a-f]+)\b','dir',shape)
    shape=re.sub(r'L\d+_\w+','L',shape)
    pat[f"{mn} {shape} [{n}]"]+=n
print("code bytes:", total)
for k,v in byt.most_common(12): print(f"{v:6} {cnt[k]:5} {k}")
print()
for k,v in pat.most_common(int(sys.argv[2]) if len(sys.argv)>2 else 40): print(f"{v:6} {k}")

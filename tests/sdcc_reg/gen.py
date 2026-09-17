#!/usr/bin/env python3
"""Expand SDCC regression templates (.c / .c.in) into test instances with a suite table."""
import sys, re, os, itertools

def generate(inname, outdir):
    lines = open(inname, encoding='latin-1').read().splitlines(keepends=True)
    base = os.path.basename(inname)
    if base.endswith('.in'): base = base[:-3]
    base = base[:-2]
    repl, funcs, inheader = [], [], True
    for line in lines:
        l = line.strip()
        if inheader:
            if ':' in l:
                name, raw = l.split(':', 1)
                repl.append((name.strip(), [v.strip() for v in raw.split(',')]))
            elif '*/' in l:
                inheader = False
        else:
            m = re.match(r'^(?:\W*void\W+)?\W*(test\w*)\W*\(\W*void\W*\)', l)
            if m: funcs.append(m.group(1))
    if not funcs: return []
    body = ''.join(lines)
    body += '\nvoid\n__runSuite(void)\n{\n'
    for f in funcs:
        body += '  __prints("Running %s\\n");\n  %s();\n' % (f, f)
    body += '}\n\nconst int __numCases = %d;\n' % len(funcs)
    body += '\n__code const char *\n__getSuiteName(void)\n{\n  return "{testcase}";\n}\n'
    out = []
    os.makedirs(outdir, exist_ok=True)
    keys = [k for k, _ in repl]
    for combo in itertools.product(*[v for _, v in repl]) if repl else [()]:
        name = base
        for k, v in zip(keys, combo):
            name += '_' + k + '_' + re.sub(r'\s+', '_', v or 'none')
        text = body
        subs = dict(zip(keys, combo)); subs['testcase'] = name
        # HTMLgen TemplateDocument: replace {key} for known keys only.
        text = re.sub(r'\{(\w+)\}', lambda m: subs.get(m.group(1), m.group(0)), text)
        path = os.path.join(outdir, name + '.c')
        open(path, 'w', encoding='latin-1').write(text)
        out.append(path)
    return out

if __name__ == '__main__':
    os.makedirs(sys.argv[1], exist_ok=True)
    for f in sys.argv[2:]:
        for p in generate(f, sys.argv[1]):
            print(p)

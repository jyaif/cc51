# Reference model of the cc51 soft float (runtime/rt.s): round to nearest with ties away from zero,
# denormal inputs read as 1.m*2^-127, underflow to zero. Prints the results tests/float_fuzz.c must produce.
import sys
M32 = 0xffffffff
SIGN = 0x80000000
QNAN = 0x7fc00000
def isnan(a): return (a & 0x7fffffff) > 0x7f800000
def isinf(a): return (a & 0x7fffffff) == 0x7f800000
def fexp(x): return ((x >> 23) & 0xff) - 127
def mant(x): return ((x & 0x7fffff) | 0x800000) << 3
def pack(sign, e, m):
    if not m: return sign
    while m >= 0x8000000: m = (m >> 1) | (m & 1); e += 1
    while m < 0x4000000: m <<= 1; e -= 1
    m += 4
    if m >= 0x8000000: m >>= 1; e += 1
    m >>= 3; e += 127
    if e >= 255: return sign | 0x7f800000
    if e <= 0: return sign
    return sign | (e << 23) | (m & 0x7fffff)
def fsadd(a, b):
    if isnan(a) or isnan(b): return QNAN
    if isinf(a): return QNAN if (isinf(b) and (a ^ b) & SIGN) else a
    if isinf(b): return b
    if not a & 0x7fffffff: return b
    if not b & 0x7fffffff: return a
    ea, eb, ma, mb = fexp(a), fexp(b), mant(a), mant(b)
    if eb > ea or (eb == ea and mb > ma): a, b, ea, eb, ma, mb = b, a, eb, ea, mb, ma
    d = ea - eb
    if d > 30: mb = 1 if mb else 0
    else:
        lost = mb & ((1 << d) - 1); mb >>= d
        if lost: mb |= 1
    ma = ma - mb if (a ^ b) & SIGN else ma + mb
    return pack(a & SIGN, ea, ma)
def fssub(a, b): return fsadd(a, b ^ SIGN)
def fsmul(a, b):
    sign = (a ^ b) & SIGN
    if isnan(a) or isnan(b): return QNAN
    if isinf(a) or isinf(b):
        if not a & 0x7fffffff or not b & 0x7fffffff: return QNAN
        return sign | 0x7f800000
    if not a & 0x7fffffff or not b & 0x7fffffff: return sign
    p = ((a & 0x7fffff) | 0x800000) * ((b & 0x7fffff) | 0x800000)
    return pack(sign, fexp(a) + fexp(b), p >> 20)
def fsdiv(a, b):
    sign = (a ^ b) & SIGN
    if isnan(a) or isnan(b): return QNAN
    if isinf(a): return QNAN if isinf(b) else sign | 0x7f800000
    if isinf(b): return sign
    if not b & 0x7fffffff: return (sign | 0x7f800000) if a & 0x7fffffff else QNAN
    if not a & 0x7fffffff: return sign
    ma, mb, q = (a & 0x7fffff) | 0x800000, (b & 0x7fffff) | 0x800000, 0
    for _ in range(27):
        q <<= 1
        if ma >= mb: ma -= mb; q |= 1
        ma <<= 1
    if ma: q |= 1
    return pack(sign, fexp(a) - fexp(b), q)
def sl2fs(v):
    if v & SIGN: v -= 1 << 32
    sign = 0
    if v < 0: sign = SIGN; v = -v
    m = v & M32
    if not m: return 0
    e = 26
    while m >= 0x8000000: m = (m >> 1) | (m & 1); e += 1
    return pack(sign, e, m)
def ul2fs(m):
    if not m: return 0
    e = 26
    while m >= 0x8000000: m = (m >> 1) | (m & 1); e += 1
    return pack(0, e, m)
def fs2ul(a):
    e = fexp(a)
    if e < 0 or a & SIGN: return 0
    m = (a & 0x7fffff) | 0x800000
    if e > 31: return M32
    if e >= 23: return (m << (e - 23)) & M32
    return m >> (23 - e)
def fs2sl(a):
    v = fs2ul(a & 0x7fffffff)
    return (-v) & M32 if a & SIGN else v
def fseq(a, b):
    if isnan(a) or isnan(b): return 0
    return int(a == b or not (a | b) & 0x7fffffff)
def fslt(a, b):
    if isnan(a) or isnan(b): return 0
    if not (a | b) & 0x7fffffff: return 0
    if (a ^ b) & SIGN: return int(bool(a & SIGN))
    neg = bool(a & SIGN)
    a &= 0x7fffffff; b &= 0x7fffffff
    if a == b: return 0
    return int((a < b) != neg)

seed = 12345
def rnd():
    global seed
    seed = (seed * 1103515245 + 12345) & M32
    return seed
special = [0x00000000, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000,
  0x7f7fffff, 0x00800000, 0x00000001, 0x3f800001, 0x3fc00000, 0x4f000000, 0xcf000000,
  0x4f800000, 0x4b7fffff, 0x3effffff, 0x7f000000, 0x01000000, 0x33800000]
def one(x, y):
    h = lambda v: "%08x" % v
    print(" ".join([h(fsadd(x, y)), h(fssub(x, y)), h(fsmul(x, y)), h(fsdiv(x, y)), "%d%d" % (fslt(x, y), fseq(x, y)),
                    h(fs2sl(x)), h(fs2ul(x)), h(sl2fs(x)), h(ul2fs(x))]))
for x in special:
    for y in special: one(x, y)
for n in range(6000):
    i = n & 0xff
    x, y = rnd(), rnd()
    k = (rnd() >> 16) & 7
    if k < 5: y = (y & 0x807fffff) | (((x & 0x7f800000) + ((k & 3) << 23)) & 0x7f800000)
    if k == 5: y = x ^ ((rnd() >> 20) & 0xff)
    if k == 7: y = (y & 0x807fffff) | (((x & 0x7f800000) - ((rnd() & 0x1f00000) << 3)) & 0x7f800000)
    if k == 6: x = ((rnd() >> (rnd() & 31)) * (1 if i & 1 else -1)) & M32
    one(x, y)

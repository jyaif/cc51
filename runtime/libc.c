/* cc51 C library. Compiled as a weak translation unit: user definitions take precedence and
   unused functions are removed by the linker. */
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

int putchar(int c);

/* ---- string.h ---- */

void *memcpy(void *dst, const void *src, size_t n) {
  char *d = dst;
  const char *s = src;
  while (n--) *d++ = *s++;
  return dst;
}

void *memmove(void *dst, const void *src, size_t n) {
  char *d = dst;
  const char *s = src;
  if (d < s) {
    while (n--) *d++ = *s++;
  } else {
    d += n;
    s += n;
    while (n--) *--d = *--s;
  }
  return dst;
}

void *memset(void *dst, int c, size_t n) {
  char *d = dst;
  while (n--) *d++ = c;
  return dst;
}

int memcmp(const void *a, const void *b, size_t n) {
  const unsigned char *p = a, *q = b;
  while (n--) {
    if (*p != *q) return *p - *q;
    p++;
    q++;
  }
  return 0;
}

void *memchr(const void *s, int c, size_t n) {
  const unsigned char *p = s;
  while (n--) {
    if (*p == (unsigned char)c) return (void *)p;
    p++;
  }
  return NULL;
}

size_t strlen(const char *s) {
  const char *p = s;
  while (*p) p++;
  return p - s;
}

char *strcpy(char *dst, const char *src) {
  char *d = dst;
  while ((*d++ = *src++));
  return dst;
}

char *strncpy(char *dst, const char *src, size_t n) {
  char *d = dst;
  while (n && (*d = *src)) {
    d++;
    src++;
    n--;
  }
  while (n--) *d++ = 0;
  return dst;
}

char *strcat(char *dst, const char *src) {
  char *d = dst;
  while (*d) d++;
  while ((*d++ = *src++));
  return dst;
}

char *strncat(char *dst, const char *src, size_t n) {
  char *d = dst;
  while (*d) d++;
  while (n-- && (*d = *src)) {
    d++;
    src++;
  }
  *d = 0;
  return dst;
}

int strcmp(const char *a, const char *b) {
  while (*a && *a == *b) {
    a++;
    b++;
  }
  return (unsigned char)*a - (unsigned char)*b;
}

int strncmp(const char *a, const char *b, size_t n) {
  while (n && *a && *a == *b) {
    a++;
    b++;
    n--;
  }
  if (!n) return 0;
  return (unsigned char)*a - (unsigned char)*b;
}

char *strchr(const char *s, int c) {
  for (;; s++) {
    if (*s == (char)c) return (char *)s;
    if (!*s) return NULL;
  }
}

char *strrchr(const char *s, int c) {
  const char *r = NULL;
  for (;; s++) {
    if (*s == (char)c) r = s;
    if (!*s) return (char *)r;
  }
}

/* ---- stdlib.h ---- */

int abs(int j) { return j < 0 ? -j : j; }

long labs(long j) { return j < 0 ? -j : j; }

int atoi(const char *s) {
  int v = 0;
  char neg = 0;
  while (*s == ' ' || (*s >= '\t' && *s <= '\r')) s++;
  if (*s == '-') {
    neg = 1;
    s++;
  } else if (*s == '+') {
    s++;
  }
  while (*s >= '0' && *s <= '9') v = v * 10 + (*s++ - '0');
  return neg ? -v : v;
}

long atol(const char *s) {
  long v = 0;
  char neg = 0;
  while (*s == ' ' || (*s >= '\t' && *s <= '\r')) s++;
  if (*s == '-') {
    neg = 1;
    s++;
  } else if (*s == '+') {
    s++;
  }
  while (*s >= '0' && *s <= '9') v = v * 10 + (*s++ - '0');
  return neg ? -v : v;
}

static unsigned long __rand_next = 1;

int rand(void) {
  __rand_next = __rand_next * 1103515245UL + 12345;
  return (unsigned int)(__rand_next >> 16) & 0x7fff;
}

void srand(unsigned int seed) { __rand_next = seed; }

/* ---- stdio.h ---- */

static char *__sprintf_buf;

static void __out_putchar(char c) { putchar(c); }

static void __out_buf(char c) { *__sprintf_buf++ = c; }

static int __vprint(void (*out)(char), const char *fmt, va_list ap) {
  int count = 0;
  char c;
  char buf[12];
  while ((c = *fmt++)) {
    if (c != '%') {
      out(c);
      count++;
      continue;
    }
    char left = 0, zero = 0, is_long = 0, sign = 0;
    unsigned char width = 0;
    c = *fmt++;
    if (c == '-') {
      left = 1;
      c = *fmt++;
    }
    if (c == '0') {
      zero = 1;
      c = *fmt++;
    }
    while (c >= '0' && c <= '9') {
      width = width * 10 + (c - '0');
      c = *fmt++;
    }
    if (c == 'l') {
      is_long = 1;
      c = *fmt++;
    } else if (c == 'h') {
      c = *fmt++;
    }
    const char *s;
    unsigned char len;
    switch (c) {
      case 'c':
        buf[0] = (char)va_arg(ap, int);
        s = buf;
        len = 1;
        zero = 0;
        break;
      case 's':
        s = va_arg(ap, char *);
        len = 0;
        while (s[len]) len++;
        zero = 0;
        break;
      case 'd':
      case 'i':
      case 'u':
      case 'x':
      case 'X':
      case 'o':
      case 'p': {
        unsigned long v;
        unsigned char base = 10;
        if (c == 'x' || c == 'X' || c == 'p') base = 16;
        if (c == 'o') base = 8;
        if (c == 'p') {
          v = (unsigned int)va_arg(ap, char *);
        } else if (is_long) {
          v = va_arg(ap, unsigned long);
          if ((c == 'd' || c == 'i') && (long)v < 0) {
            sign = '-';
            v = -(long)v;
          }
        } else {
          int iv = va_arg(ap, int);
          if ((c == 'd' || c == 'i') && iv < 0) {
            sign = '-';
            v = (unsigned int)-iv;
          } else {
            v = (unsigned int)iv;
          }
        }
        char *p = buf + sizeof(buf);
        len = 0;
        do {
          unsigned char d = v % base;
          v /= base;
          *--p = d < 10 ? '0' + d : (c == 'X' ? 'A' : 'a') + d - 10;
          len++;
        } while (v);
        s = p;
        break;
      }
      case 0:
        return count;
      default:
        buf[0] = c;
        s = buf;
        len = 1;
        zero = 0;
        break;
    }
    unsigned char total = len + (sign ? 1 : 0);
    unsigned char pad = width > total ? width - total : 0;
    if (sign && zero) {
      out(sign);
      count++;
      sign = 0;
    }
    if (!left) {
      while (pad) {
        out(zero ? '0' : ' ');
        pad--;
        count++;
      }
    }
    if (sign) {
      out(sign);
      count++;
    }
    while (len--) {
      out(*s++);
      count++;
    }
    while (pad) {
      out(' ');
      pad--;
      count++;
    }
  }
  return count;
}

int vprintf(const char *fmt, va_list ap) { return __vprint(__out_putchar, fmt, ap); }

int printf(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  return __vprint(__out_putchar, fmt, ap);
}

int vsprintf(char *buf, const char *fmt, va_list ap) {
  __sprintf_buf = buf;
  int n = __vprint(__out_buf, fmt, ap);
  *__sprintf_buf = 0;
  return n;
}

int sprintf(char *buf, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  __sprintf_buf = buf;
  int n = __vprint(__out_buf, fmt, ap);
  *__sprintf_buf = 0;
  return n;
}

int puts(const char *s) {
  while (*s) putchar(*s++);
  putchar('\n');
  return 0;
}

/* ---- IEEE-754 single precision soft float (floats are passed as unsigned long bit patterns) ---- */

#define __FS_SIGN 0x80000000UL
#define __FS_MANT 0x00ffffffUL

/* Pack sign, unbiased exponent and a mantissa normalized to bit 26 (3 guard bits). */
static unsigned long __fs_pack(unsigned long sign, int exp, unsigned long m) {
  if (!m) return sign;
  while (m >= 0x8000000UL) {
    m = (m >> 1) | (m & 1);
    exp++;
  }
  while (m < 0x4000000UL) {
    m <<= 1;
    exp--;
  }
  /* round to nearest (guard bits: 3) */
  m += 4;
  if (m >= 0x8000000UL) {
    m >>= 1;
    exp++;
  }
  m >>= 3;
  exp += 127;
  if (exp >= 255) return sign | 0x7f800000UL;
  if (exp <= 0) return sign;
  return sign | ((unsigned long)exp << 23) | (m & 0x7fffffUL);
}

static int __fs_exp(unsigned long x) { return (int)((x >> 23) & 0xff) - 127; }

static unsigned long __fs_mant(unsigned long x) { return ((x & 0x7fffffUL) | 0x800000UL) << 3; }

unsigned long __fsadd(unsigned long a, unsigned long b) {
  if (!(a & 0x7fffffffUL)) return b;
  if (!(b & 0x7fffffffUL)) return a;
  int ea = __fs_exp(a), eb = __fs_exp(b);
  unsigned long ma = __fs_mant(a), mb = __fs_mant(b);
  /* make |a| >= |b| */
  if (eb > ea || (eb == ea && mb > ma)) {
    unsigned long t = a;
    a = b;
    b = t;
    int te = ea;
    ea = eb;
    eb = te;
    t = ma;
    ma = mb;
    mb = t;
  }
  int d = ea - eb;
  if (d > 30) {
    mb = mb ? 1 : 0;
  } else {
    unsigned long lost = mb & ((1UL << d) - 1);
    mb >>= d;
    if (lost) mb |= 1;
  }
  if ((a ^ b) & __FS_SIGN)
    ma -= mb;
  else
    ma += mb;
  return __fs_pack(a & __FS_SIGN, ea, ma);
}

unsigned long __fssub(unsigned long a, unsigned long b) { return __fsadd(a, b ^ __FS_SIGN); }

unsigned long __fsmul(unsigned long a, unsigned long b) {
  unsigned long sign = (a ^ b) & __FS_SIGN;
  if (!(a & 0x7fffffffUL) || !(b & 0x7fffffffUL)) return sign;
  unsigned long ma = (a & 0x7fffffUL) | 0x800000UL;
  unsigned long mb = (b & 0x7fffffUL) | 0x800000UL;
  /* 24x24 bit product in two halves */
  unsigned long ah = ma >> 12, al = ma & 0xfff, bh = mb >> 12, bl = mb & 0xfff;
  unsigned long lo = al * bl;
  unsigned long mid = ah * bl + al * bh + (lo >> 12);
  unsigned long hi = ah * bh + (mid >> 12);
  /* hi = product >> 24; keep sticky information from the lower part */
  unsigned long m = (hi << 3) | (((mid & 0xfff) | (lo & 0xfff)) ? 1 : 0);
  return __fs_pack(sign, __fs_exp(a) + __fs_exp(b) + 1 - 1, m << 1);
}

unsigned long __fsdiv(unsigned long a, unsigned long b) {
  unsigned long sign = (a ^ b) & __FS_SIGN;
  if (!(b & 0x7fffffffUL)) return sign | 0x7f800000UL;
  if (!(a & 0x7fffffffUL)) return sign;
  unsigned long ma = (a & 0x7fffffUL) | 0x800000UL;
  unsigned long mb = (b & 0x7fffffUL) | 0x800000UL;
  unsigned long q = 0;
  unsigned char i;
  for (i = 0; i < 27; i++) {
    q <<= 1;
    if (ma >= mb) {
      ma -= mb;
      q |= 1;
    }
    ma <<= 1;
  }
  if (ma) q |= 1;
  return __fs_pack(sign, __fs_exp(a) - __fs_exp(b), q);
}

unsigned long __sl2fs(long v) {
  unsigned long sign = 0;
  if (v < 0) {
    sign = __FS_SIGN;
    v = -v;
  }
  unsigned long m = (unsigned long)v;
  if (!m) return 0;
  int e = 26;
  while (m >= 0x8000000UL) {
    m = (m >> 1) | (m & 1);
    e++;
  }
  return __fs_pack(sign, e, m);
}

unsigned long __ul2fs(unsigned long m) {
  if (!m) return 0;
  int e = 26;
  while (m >= 0x8000000UL) {
    m = (m >> 1) | (m & 1);
    e++;
  }
  return __fs_pack(0, e, m);
}

unsigned long __fs2ul(unsigned long a) {
  int e = __fs_exp(a);
  if (e < 0 || (a & __FS_SIGN)) return 0;
  unsigned long m = (a & 0x7fffffUL) | 0x800000UL;
  if (e > 31) return 0xffffffffUL;
  if (e >= 23) return m << (e - 23);
  return m >> (23 - e);
}

long __fs2sl(unsigned long a) {
  unsigned long v = __fs2ul(a & 0x7fffffffUL);
  return (a & __FS_SIGN) ? -(long)v : (long)v;
}

char __fseq(unsigned long a, unsigned long b) { return a == b || !((a | b) & 0x7fffffffUL); }

char __fslt(unsigned long a, unsigned long b) {
  if (!((a | b) & 0x7fffffffUL)) return 0;
  if ((a ^ b) & __FS_SIGN) return (a & __FS_SIGN) != 0;
  char neg = (a & __FS_SIGN) != 0;
  a &= 0x7fffffffUL;
  b &= 0x7fffffffUL;
  if (a == b) return 0;
  return (a < b) != neg;
}

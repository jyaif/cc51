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

/* Print a float with `prec` decimals. Only linked when the program passes floats to printf. */
static unsigned char __fmt_float(char *buf, float f, unsigned char prec) {
  unsigned long scaled;
  unsigned char n = 0, k;
  float scale = 1.0f;
  for (k = 0; k < prec; k++) scale *= 10.0f;
  f *= scale;
  if (f >= 4294967040.0f) {
    buf[0] = '?';
    return 1;
  }
  scaled = (unsigned long)f;
  if (f - (float)scaled >= 0.5f) scaled++;
  do {
    buf[n++] = '0' + (unsigned char)(scaled % 10);
    scaled /= 10;
    if (n == prec) buf[n++] = '.';
  } while (scaled || n <= prec);
  /* digits were produced in reverse */
  for (k = 0; k < n / 2; k++) {
    char t = buf[k];
    buf[k] = buf[n - 1 - k];
    buf[n - 1 - k] = t;
  }
  return n;
}

static int __vprint(void (*out)(char), const char *fmt, va_list ap) {
  int count = 0;
  char c;
  char buf[16];
  while ((c = *fmt++)) {
    if (c != '%') {
      out(c);
      count++;
      continue;
    }
    char left = 0, zero = 0, is_long = 0, is_byte = 0, sign = 0, alt = 0;
    unsigned char width = 0, prec = 0;
    char has_prec = 0;
    c = *fmt++;
    for (;;) {
      if (c == '-') left = 1;
      else if (c == '0') zero = 1;
      else if (c == '+') sign = '+';
      else if (c == ' ') { if (!sign) sign = ' '; }
      else if (c == '#') alt = 1;
      else break;
      c = *fmt++;
    }
    while (c >= '0' && c <= '9') {
      width = width * 10 + (c - '0');
      c = *fmt++;
    }
    if (c == '.') {
      has_prec = 1;
      c = *fmt++;
      while (c >= '0' && c <= '9') {
        prec = prec * 10 + (c - '0');
        c = *fmt++;
      }
    }
    if (c == 'l') {
      is_long = 1;
      c = *fmt++;
    } else if (c == 'h') {
      c = *fmt++;
    } else if (c == 'b') {
      is_byte = 1;
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
        sign = 0;
        break;
      case 's':
        s = va_arg(ap, char *);
        len = 0;
        while (s[len] && (!has_prec || len < prec)) len++;
        zero = 0;
        sign = 0;
        break;
      case 'f':
      case 'F':
        if (__builtin_float_varargs()) {
          float fv = (float)va_arg(ap, float);
          if (fv < 0.0f) {
            sign = '-';
            fv = -fv;
          }
          len = __fmt_float(buf, fv, has_prec ? prec : 6);
          s = buf;
        } else {
          s = "<NO FLOAT>";
          len = 10;
          sign = 0;
          zero = 0;
        }
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
        if (c != 'd' && c != 'i') sign = 0;
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
          if (is_byte) iv = (c == 'd' || c == 'i') ? (signed char)iv : (unsigned char)iv;
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
        if (alt && base == 8 && *p != '0') {
          *--p = '0';
          len++;
        }
        while (has_prec && len < prec) {
          *--p = '0';
          len++;
        }
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
        sign = 0;
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

/* ---- ctype ---- */

int isdigit(int c) { return (unsigned char)(c - '0') < 10; }
int isupper(int c) { return (unsigned char)(c - 'A') < 26; }
int islower(int c) { return (unsigned char)(c - 'a') < 26; }
int isalpha(int c) { return (unsigned char)((c | 0x20) - 'a') < 26; }
int isalnum(int c) { return isalpha(c) || isdigit(c); }
int isxdigit(int c) { return isdigit(c) || (unsigned char)((c | 0x20) - 'a') < 6; }
int isspace(int c) { return c == ' ' || (unsigned char)(c - '\t') < 5; }
int isblank(int c) { return c == ' ' || c == '\t'; }
int isprint(int c) { return (unsigned char)(c - ' ') < 95; }
int isgraph(int c) { return (unsigned char)(c - '!') < 94; }
int iscntrl(int c) { return (unsigned char)c < 32 || c == 127; }
int ispunct(int c) { return isgraph(c) && !isalnum(c); }
int isascii(int c) { return (unsigned char)c < 128; }
int toascii(int c) { return c & 0x7f; }
int toupper(int c) { return islower(c) ? c - 32 : c; }
int tolower(int c) { return isupper(c) ? c + 32 : c; }

/* ---- strings ---- */

void *memccpy(void *dst, const void *src, int c, size_t n) {
  char *d = dst;
  const char *s = src;
  while (n--) {
    char x = *s++;
    *d++ = x;
    if (x == (char)c) return d;
  }
  return 0;
}

char *strstr(const char *h, const char *n) {
  if (!*n) return (char *)h;
  while (*h) {
    const char *a = h, *b = n;
    while (*a && *b && *a == *b) { a++; b++; }
    if (!*b) return (char *)h;
    h++;
  }
  return 0;
}

size_t strspn(const char *s, const char *set) {
  size_t n = 0;
  while (s[n] && strchr(set, s[n])) n++;
  return n;
}

size_t strcspn(const char *s, const char *set) {
  size_t n = 0;
  while (s[n] && !strchr(set, s[n])) n++;
  return n;
}

char *strpbrk(const char *s, const char *set) {
  for (; *s; s++)
    if (strchr(set, *s)) return (char *)s;
  return 0;
}

static char *__strtok_save;

char *strtok(char *s, const char *sep) {
  char *p;
  if (!s) s = __strtok_save;
  if (!s) return 0;
  s += strspn(s, sep);
  if (!*s) { __strtok_save = 0; return 0; }
  p = s + strcspn(s, sep);
  if (*p) { *p++ = 0; }
  __strtok_save = p;
  return s;
}

size_t memalignment(const void *p) {
  size_t a = 1;
  unsigned int v = (unsigned int)(unsigned long)p;
  if (!v) return 0;
  while (!(v & a)) a <<= 1;
  return a;
}

/* ---- conversions ---- */

char *__uitoa(unsigned int value, char *buf, unsigned char radix) {
  char tmp[17];
  unsigned char n = 0;
  char *p = buf;
  do {
    unsigned char d = value % radix;
    value /= radix;
    tmp[n++] = d < 10 ? '0' + d : 'A' + d - 10;
  } while (value);
  while (n) *p++ = tmp[--n];
  *p = 0;
  return buf;
}

char *__itoa(int value, char *buf, unsigned char radix) {
  if (radix == 10 && value < 0) {
    buf[0] = '-';
    __uitoa((unsigned int)-value, buf + 1, radix);
    return buf;
  }
  return __uitoa((unsigned int)value, buf, radix);
}

char *__ultoa(unsigned long value, char *buf, unsigned char radix) {
  char tmp[33];
  unsigned char n = 0;
  char *p = buf;
  do {
    unsigned char d = value % radix;
    value /= radix;
    tmp[n++] = d < 10 ? '0' + d : 'A' + d - 10;
  } while (value);
  while (n) *p++ = tmp[--n];
  *p = 0;
  return buf;
}

char *__ltoa(long value, char *buf, unsigned char radix) {
  if (radix == 10 && value < 0) {
    buf[0] = '-';
    __ultoa((unsigned long)-value, buf + 1, radix);
    return buf;
  }
  return __ultoa((unsigned long)value, buf, radix);
}

/* ---- sorting and searching ---- */

void qsort(void *base, size_t n, size_t size, int (*cmp)(const void *, const void *)) {
  /* Insertion sort: small code, and n is small on this target. */
  char *b = base;
  size_t i, j;
  for (i = 1; i < n; i++) {
    for (j = i; j && cmp(b + (j - 1) * size, b + j * size) > 0; j--) {
      char *x = b + (j - 1) * size, *y = b + j * size;
      size_t k;
      for (k = 0; k < size; k++) {
        char t = x[k];
        x[k] = y[k];
        y[k] = t;
      }
    }
  }
}

void *bsearch(const void *key, const void *base, size_t n, size_t size, int (*cmp)(const void *, const void *)) {
  const char *b = base;
  while (n) {
    size_t m = n / 2;
    const char *p = b + m * size;
    int c = cmp(key, p);
    if (c == 0) return (void *)p;
    if (c > 0) {
      b = p + size;
      n -= m + 1;
    } else {
      n = m;
    }
  }
  return 0;
}

/* ---- heap ---- */

/* The heap lives in external RAM and is only linked in when malloc is used. */
#ifndef __CC51_HEAP_SIZE
#define __CC51_HEAP_SIZE 1024
#endif

struct __blk {
  unsigned int size; /* payload size; low bit set while allocated */
  struct __blk __xdata *next;
};

static __xdata char __heap[__CC51_HEAP_SIZE];
static struct __blk __xdata *__heap_head;
static char __heap_ready;

void *malloc(size_t n) {
  struct __blk __xdata *b;
  struct __blk __xdata *prev;
  if (!__heap_ready) {
    __heap_ready = 1;
    __heap_head = (struct __blk __xdata *)__heap;
    __heap_head->size = sizeof(__heap) - sizeof(struct __blk);
    __heap_head->next = 0;
  }
  if (n > (size_t)-1 - sizeof(struct __blk) - 4) return 0;
  n = (n + 1) & ~1u;
  if (!n) n = 2;
  prev = 0;
  for (b = __heap_head; b; b = b->next) {
    unsigned int size = b->size & ~1u;
    if (b->size & 1 || size < n) continue;
    if (size >= n + sizeof(struct __blk) + 2) {
      /* split */
      struct __blk __xdata *nb = (struct __blk __xdata *)((__xdata char *)b + sizeof(struct __blk) + n);
      nb->size = size - n - sizeof(struct __blk);
      nb->next = b->next;
      b->next = nb;
      b->size = n;
    }
    b->size |= 1;
    (void)prev;
    return (__xdata char *)b + sizeof(struct __blk);
  }
  return 0;
}

void free(void *p) {
  struct __blk __xdata *b;
  struct __blk __xdata *c;
  if (!p) return;
  b = (struct __blk __xdata *)((__xdata char *)p - sizeof(struct __blk));
  b->size &= ~1u;
  /* coalesce forward */
  for (c = __heap_head; c; c = c->next) {
    while (c->next && !(c->size & 1) && !(c->next->size & 1)) {
      c->size += c->next->size + sizeof(struct __blk);
      c->next = c->next->next;
    }
  }
}

void *calloc(size_t n, size_t size) {
  size_t total;
  void *p;
  if (n && size > (size_t)-1 / n) return 0; /* overflow */
  total = n * size;
  p = malloc(total);
  if (p) memset(p, 0, total);
  return p;
}

void *realloc(void *p, size_t n) {
  struct __blk __xdata *b;
  void *q;
  if (!p) return malloc(n);
  b = (struct __blk __xdata *)((__xdata char *)p - sizeof(struct __blk));
  if ((b->size & ~1u) >= n) return p;
  q = malloc(n);
  if (q) {
    memcpy(q, p, b->size & ~1u);
    free(p);
  }
  return q;
}

/* ---- errno ---- */

int errno;

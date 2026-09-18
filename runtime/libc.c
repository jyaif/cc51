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
  if (buf[n - 1] == '.') buf[n++] = '0'; /* leading zero */
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
      case 'p':
      case 'P': {
        /* A generic pointer prints as <space>:0x<address> (SDCC). */
        unsigned long pv = (unsigned long)va_arg(ap, char *);
        unsigned char memtype = (unsigned char)(pv >> 16);
        unsigned char nd = 4;
        char *q = buf;
        if (memtype >= 0x80) *q = 'C';
        else if (memtype >= 0x60) { *q = 'P'; nd = 2; }
        else if (memtype >= 0x40) { *q = 'I'; nd = 2; }
        else *q = 'X';
        q++;
        *q++ = ':';
        *q++ = '0';
        *q++ = 'x';
        while (nd--) {
          unsigned char d = (unsigned char)((pv >> (nd * 4)) & 0xf);
          *q++ = d < 10 ? '0' + d : (c == 'P' ? 'A' : 'a') + d - 10;
        }
        s = buf;
        len = (unsigned char)(q - buf);
        zero = 0;
        sign = 0;
        break;
      }
      case 'd':
      case 'i':
      case 'u':
      case 'x':
      case 'X':
      case 'o': {
        unsigned long v;
        unsigned char base = 10;
        if (c == 'x' || c == 'X') base = 16;
        if (c == 'o') base = 8;
        if (c != 'd' && c != 'i') sign = 0;
        if (is_long) {
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

#define __FS_QNAN 0x7fc00000UL

static char __fs_isnan(unsigned long a) { return (a & 0x7fffffffUL) > 0x7f800000UL; }
static char __fs_isinf(unsigned long a) { return (a & 0x7fffffffUL) == 0x7f800000UL; }

unsigned long __fsadd(unsigned long a, unsigned long b) {
  if (__fs_isnan(a) || __fs_isnan(b)) return __FS_QNAN;
  if (__fs_isinf(a)) return (__fs_isinf(b) && ((a ^ b) & __FS_SIGN)) ? __FS_QNAN : a;
  if (__fs_isinf(b)) return b;
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
  if (__fs_isnan(a) || __fs_isnan(b)) return __FS_QNAN;
  if (__fs_isinf(a) || __fs_isinf(b)) {
    if (!(a & 0x7fffffffUL) || !(b & 0x7fffffffUL)) return __FS_QNAN; /* inf * 0 */
    return sign | 0x7f800000UL;
  }
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
  if (__fs_isnan(a) || __fs_isnan(b)) return __FS_QNAN;
  if (__fs_isinf(a)) return __fs_isinf(b) ? __FS_QNAN : (sign | 0x7f800000UL);
  if (__fs_isinf(b)) return sign;
  if (!(b & 0x7fffffffUL)) return (a & 0x7fffffffUL) ? (sign | 0x7f800000UL) : __FS_QNAN;
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

char __fseq(unsigned long a, unsigned long b) {
  if (__fs_isnan(a) || __fs_isnan(b)) return 0;
  return a == b || !((a | b) & 0x7fffffffUL);
}

char __fslt(unsigned long a, unsigned long b) {
  if (__fs_isnan(a) || __fs_isnan(b)) return 0;
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

/* ---- math (single precision) ---- */

union __fbits {
  float f;
  unsigned long u;
};

float fabsf(float x) {
  union __fbits v;
  v.f = x;
  v.u &= 0x7fffffffUL;
  return v.f;
}

float ldexpf(float x, int n) {
  union __fbits v;
  int e;
  v.f = x;
  e = (int)((v.u >> 23) & 0xff);
  if (e == 0 || e == 255) return x; /* zero, subnormal, inf or nan */
  e += n;
  if (e >= 255) {
    v.u = (v.u & 0x80000000UL) | 0x7f800000UL;
    return v.f;
  }
  if (e <= 0) {
    v.u &= 0x80000000UL;
    return v.f;
  }
  v.u = (v.u & 0x807fffffUL) | ((unsigned long)e << 23);
  return v.f;
}

float frexpf(float x, int *e) {
  union __fbits v;
  int ex;
  v.f = x;
  ex = (int)((v.u >> 23) & 0xff);
  if (ex == 0 || ex == 255) {
    *e = 0;
    return x;
  }
  *e = ex - 126;
  v.u = (v.u & 0x807fffffUL) | 0x3f000000UL; /* mantissa in [0.5, 1) */
  return v.f;
}

float truncf(float x) {
  if (fabsf(x) >= 8388608.0f) return x;
  return (float)(long)x;
}

float floorf(float x) {
  float t;
  if (fabsf(x) >= 8388608.0f) return x;
  t = (float)(long)x;
  return t > x ? t - 1.0f : t;
}

float ceilf(float x) {
  float t;
  if (fabsf(x) >= 8388608.0f) return x;
  t = (float)(long)x;
  return t < x ? t + 1.0f : t;
}

float roundf(float x) { return x < 0.0f ? -floorf(0.5f - x) : floorf(x + 0.5f); }

float fmodf(float x, float y) {
  float q;
  if (y == 0.0f) return 0.0f;
  q = truncf(x / y);
  return x - q * y;
}

float modff(float x, float *ip) {
  float i = truncf(x);
  *ip = i;
  return x - i;
}

float sqrtf(float x) {
  float m, r;
  int e;
  if (x <= 0.0f) return 0.0f;
  m = frexpf(x, &e);
  if (e & 1) {
    m *= 2.0f;
    e--;
  }
  /* Newton iterations from a linear estimate on [0.5, 2) */
  r = 0.5f + 0.5f * m;
  r = 0.5f * (r + m / r);
  r = 0.5f * (r + m / r);
  r = 0.5f * (r + m / r);
  r = 0.5f * (r + m / r);
  return ldexpf(r, e / 2);
}

#define __LN2_HI 0.693359375f
#define __LN2_LO (-2.12194440e-4f)

float expf(float x) {
  int k;
  float r, y;
  if (x > 88.72f) return 3.4028234664e38f * 3.4028234664e38f; /* +inf */
  if (x < -103.0f) return 0.0f;
  k = (int)(x * 1.44269504f + (x >= 0.0f ? 0.5f : -0.5f));
  r = (x - (float)k * __LN2_HI) - (float)k * __LN2_LO;
  y = 1.0f + r * (1.0f + r * (0.5f + r * (0.16666667f + r * (0.041666667f + r * (0.0083333333f + r * 0.0013888889f)))));
  return ldexpf(y, k);
}

float logf(float x) {
  float m, s, z, y;
  int e;
  if (x <= 0.0f) return x < 0.0f ? 0.0f : -3.4028234664e38f * 3.4028234664e38f;
  m = frexpf(x, &e);
  if (m < 0.70710678f) {
    m *= 2.0f;
    e--;
  }
  s = (m - 1.0f) / (m + 1.0f);
  z = s * s;
  y = 2.0f * s * (1.0f + z * (0.33333333f + z * (0.2f + z * (0.14285714f + z * (0.11111111f + z * 0.09090909f)))));
  return y + (float)e * 0.69314718f;
}

float log10f(float x) { return logf(x) * 0.43429448f; }

/* Argument reduction in units of pi/4 (Cephes): x = y*(pi/4) + z, |z| < pi/4. */
#define __FOPI 1.27323954f
#define __DP1 0.78515625f
#define __DP2 2.4187564849853515625e-4f
#define __DP3 3.77489497744594108e-8f

static float __sin_poly(float z) {
  float w = z * z;
  return z + z * w * (-1.6666654611e-1f + w * (8.3321608736e-3f + w * -1.9515295891e-4f));
}

static float __cos_poly(float z) {
  float w = z * z;
  return 1.0f - 0.5f * w + w * w * (4.166664568298827e-2f + w * (-1.388731625493765e-3f + w * 2.443315711809948e-5f));
}

/* quad 0: sin, 1: cos */
static float __trig(float x, unsigned char quad) {
  float y, z;
  long j;
  signed char sign = 1;
  if (x < 0.0f) {
    x = -x;
    if (!quad) sign = -1;
  }
  j = (long)(__FOPI * x);
  y = (float)j;
  if (j & 1) {
    j += 1;
    y += 1.0f;
  }
  j &= 7;
  if (j > 3) {
    j -= 4;
    sign = -sign;
  }
  if (quad && j > 1) sign = -sign;
  z = ((x - y * __DP1) - y * __DP2) - y * __DP3;
  if (quad) y = (j == 1 || j == 2) ? __sin_poly(z) : __cos_poly(z);
  else y = (j == 1 || j == 2) ? __cos_poly(z) : __sin_poly(z);
  return sign < 0 ? -y : y;
}

float sinf(float x) { return __trig(x, 0); }
float cosf(float x) { return __trig(x, 1); }

float tanf(float x) {
  float y, z, w;
  long j;
  signed char sign = 1;
  if (x < 0.0f) {
    x = -x;
    sign = -1;
  }
  j = (long)(__FOPI * x);
  y = (float)j;
  if (j & 1) {
    j += 1;
    y += 1.0f;
  }
  z = ((x - y * __DP1) - y * __DP2) - y * __DP3;
  w = z * z;
  if (w > 1e-8f) {
    z = z + z * w * (3.33331568548e-1f + w * (1.33387994085e-1f + w * (5.34112807005e-2f + w * (2.44301354525e-2f + w * (3.11992232697e-3f + w * 9.38540185543e-3f)))));
  }
  if (j & 2) z = -1.0f / z;
  return sign < 0 ? -z : z;
}

float atanf(float x) {
  float a = fabsf(x), z, r;
  char inv = 0;
  if (a > 1.0f) {
    a = 1.0f / a;
    inv = 1;
  }
  z = a * a;
  r = a * (0.99999933f + z * (-0.33329856f + z * (0.19946536f + z * (-0.13908534f + z * (0.09642004f + z * (-0.05590989f + z * (0.02186123f - z * 0.00405406f)))))));
  if (inv) r = 1.5707963f - r;
  return x < 0.0f ? -r : r;
}

float atan2f(float y, float x) {
  if (x > 0.0f) return atanf(y / x);
  if (x < 0.0f) return y >= 0.0f ? atanf(y / x) + 3.14159265f : atanf(y / x) - 3.14159265f;
  if (y > 0.0f) return 1.57079633f;
  if (y < 0.0f) return -1.57079633f;
  return 0.0f;
}

float asinf(float x) {
  float a = fabsf(x), r;
  if (a >= 1.0f) r = 1.57079633f;
  else r = atanf(a / sqrtf(1.0f - a * a));
  return x < 0.0f ? -r : r;
}

float acosf(float x) { return 1.57079633f - asinf(x); }

float sinhf(float x) {
  float e;
  if (fabsf(x) < 0.35f) {
    float z = x * x;
    return x + x * z * (0.16666667f + z * (0.0083333333f + z * 0.00019841270f));
  }
  e = expf(x);
  return 0.5f * (e - 1.0f / e);
}

float coshf(float x) {
  float e = expf(fabsf(x));
  return 0.5f * (e + 1.0f / e);
}

float tanhf(float x) {
  float a = fabsf(x), e, r;
  if (a > 9.0f) r = 1.0f;
  else if (a < 0.3f) {
    float z = x * x;
    r = a * (1.0f + z * (-0.33333333f + z * (0.13333333f - z * 0.053968254f)));
  } else {
    e = expf(2.0f * a);
    r = (e - 1.0f) / (e + 1.0f);
  }
  return x < 0.0f ? -r : r;
}

float powf(float x, float y) {
  long n;
  if (y == 0.0f) return 1.0f;
  if (x == 0.0f) return 0.0f;
  n = (long)y;
  if (x < 0.0f) {
    /* Only integral exponents are defined for a negative base. */
    if ((float)n != y) return 0.0f;
    return (n & 1) ? -expf(y * logf(-x)) : expf(y * logf(-x));
  }
  return expf(y * logf(x));
}

float expm1f(float x) { return fabsf(x) < 0.25f ? x * (1.0f + x * (0.5f + x * (0.16666667f + x * 0.041666667f))) : expf(x) - 1.0f; }
float log1pf(float x) { return logf(1.0f + x); }

int __signbitf(float x) {
  union __fbits v;
  v.f = x;
  return (int)(v.u >> 31);
}

/* ---- 64-bit multiply and divide ---- */

unsigned long long __mullonglong(unsigned long long a, unsigned long long b) {
  unsigned long long r = 0;
  while (b) {
    if (b & 1) r += a;
    a <<= 1;
    b >>= 1;
  }
  return r;
}

static unsigned long long __udivmod64(unsigned long long a, unsigned long long b, unsigned long long *rem) {
  unsigned long long q = 0, r = 0;
  unsigned char i = 64;
  if (!b) {
    if (rem) *rem = 0;
    return 0;
  }
  while (i--) {
    r = (r << 1) | ((a >> 63) & 1);
    a <<= 1;
    q <<= 1;
    if (r >= b) {
      r -= b;
      q |= 1;
    }
  }
  if (rem) *rem = r;
  return q;
}

unsigned long long __divulonglong(unsigned long long a, unsigned long long b) { return __udivmod64(a, b, 0); }

unsigned long long __modulonglong(unsigned long long a, unsigned long long b) {
  unsigned long long r;
  __udivmod64(a, b, &r);
  return r;
}

long long __divslonglong(long long a, long long b) {
  char neg = 0;
  unsigned long long q;
  if (a < 0) { a = -a; neg = 1; }
  if (b < 0) { b = -b; neg = !neg; }
  q = __udivmod64((unsigned long long)a, (unsigned long long)b, 0);
  return neg ? -(long long)q : (long long)q;
}

long long __modslonglong(long long a, long long b) {
  char neg = 0;
  unsigned long long r;
  if (a < 0) { a = -a; neg = 1; }
  if (b < 0) b = -b;
  __udivmod64((unsigned long long)a, (unsigned long long)b, &r);
  return neg ? -(long long)r : (long long)r;
}

/* ---- stdbit.h helpers ---- */

unsigned char __stdc_ones(unsigned long long v) {
  unsigned char n = 0;
  while (v) {
    n += (unsigned char)(v & 1);
    v >>= 1;
  }
  return n;
}

unsigned char __stdc_width(unsigned long long v) {
  unsigned char n = 0;
  while (v) {
    n++;
    v >>= 1;
  }
  return n;
}

unsigned char __stdc_ctz(unsigned long long v, unsigned char bits) {
  unsigned char n = 0;
  if (!v) return bits;
  while (!(v & 1)) {
    n++;
    v >>= 1;
  }
  return n;
}

/* ---- integer division results ---- */

typedef struct { int quot, rem; } div_t;
typedef struct { long quot, rem; } ldiv_t;
typedef struct { long long quot, rem; } lldiv_t;

div_t div(int num, int den) {
  div_t r;
  r.quot = num / den;
  r.rem = num % den;
  return r;
}

ldiv_t ldiv(long num, long den) {
  ldiv_t r;
  r.quot = num / den;
  r.rem = num % den;
  return r;
}

lldiv_t lldiv(long long num, long long den) {
  lldiv_t r;
  r.quot = num / den;
  r.rem = num % den;
  return r;
}

long long llabs(long long j) { return j < 0 ? -j : j; }

/* ---- multibyte and wide characters (UTF-8 <-> UTF-32/UTF-16) ---- */

#include <errno.h>
#include <wchar.h>
#include <uchar.h>

#define __MBERR ((size_t)-1)
#define __MBINC ((size_t)-2)
#define __MBSURR ((size_t)-3)

static mbstate_t __mbs_in, __mbs_len, __mbs_c16i, __mbs_c16o;

int mbsinit(const mbstate_t *ps) { return !ps || !ps->c[0]; }

/* Encode one code point as UTF-8; s must have room for 4 bytes. Returns the length, or -1. */
static int __utf8_enc(char *s, unsigned long wc) {
  if (wc < 0x80) {
    s[0] = (char)wc;
    return 1;
  }
  if (wc < 0x800) {
    s[0] = (char)(0xc0 | (wc >> 6));
    s[1] = (char)(0x80 | (wc & 0x3f));
    return 2;
  }
  if (wc >= 0xd800 && wc <= 0xdfff) return -1;
  if (wc < 0x10000) {
    s[0] = (char)(0xe0 | (wc >> 12));
    s[1] = (char)(0x80 | ((wc >> 6) & 0x3f));
    s[2] = (char)(0x80 | (wc & 0x3f));
    return 3;
  }
  if (wc > 0x10ffff) return -1;
  s[0] = (char)(0xf0 | (wc >> 18));
  s[1] = (char)(0x80 | ((wc >> 12) & 0x3f));
  s[2] = (char)(0x80 | ((wc >> 6) & 0x3f));
  s[3] = (char)(0x80 | (wc & 0x3f));
  return 4;
}

size_t mbrtowc(wchar_t *pwc, const char *s, size_t n, mbstate_t *ps) {
  unsigned char need, seen;
  unsigned long v;
  size_t used = 0;
  if (!ps) ps = &__mbs_in;
  if (!s) {
    ps->c[0] = 0;
    return 0;
  }
  need = ps->c[0] >> 4;
  seen = ps->c[0] & 0x0f;
  v = ((unsigned long)ps->c[1] << 8) | ps->c[2];
  while (n) {
    unsigned char c = (unsigned char)*s++;
    n--;
    used++;
    if (!seen) {
      if (c < 0x80) {
        need = 1;
        v = c;
      } else if ((c & 0xe0) == 0xc0) {
        need = 2;
        v = c & 0x1f;
      } else if ((c & 0xf0) == 0xe0) {
        need = 3;
        v = c & 0x0f;
      } else if ((c & 0xf8) == 0xf0) {
        need = 4;
        v = c & 0x07;
      } else {
        errno = EILSEQ;
        return __MBERR;
      }
      seen = 1;
    } else {
      if ((c & 0xc0) != 0x80) {
        errno = EILSEQ;
        return __MBERR;
      }
      v = (v << 6) | (c & 0x3f);
      seen++;
    }
    if (seen == need) {
      ps->c[0] = 0;
      if ((v >= 0xd800 && v <= 0xdfff) || v > 0x10ffff) {
        errno = EILSEQ;
        return __MBERR;
      }
      if (pwc) *pwc = v;
      return v ? used : 0;
    }
  }
  ps->c[0] = (unsigned char)((need << 4) | seen);
  ps->c[1] = (unsigned char)(v >> 8);
  ps->c[2] = (unsigned char)v;
  return __MBINC;
}

size_t mbrlen(const char *s, size_t n, mbstate_t *ps) { return mbrtowc(0, s, n, ps ? ps : &__mbs_len); }

size_t wcrtomb(char *s, wchar_t wc, mbstate_t *ps) {
  char buf[4];
  int r;
  (void)ps;
  if (!s) {
    s = buf;
    wc = 0;
  }
  r = __utf8_enc(s, wc);
  if (r < 0) {
    errno = EILSEQ;
    return __MBERR;
  }
  return (size_t)r;
}

int mbtowc(wchar_t *pwc, const char *s, size_t n) {
  mbstate_t st;
  size_t r;
  if (!s) return 0;
  st.c[0] = 0;
  r = mbrtowc(pwc, s, n, &st);
  if (r == __MBERR || r == __MBINC) return -1;
  return (int)r;
}

int mblen(const char *s, size_t n) {
  mbstate_t st;
  size_t r;
  if (!s) return 0;
  st.c[0] = 0;
  r = mbrtowc(0, s, n, &st);
  if (r == __MBERR || r == __MBINC) return -1;
  return (int)r;
}

int wctomb(char *s, wchar_t wc) {
  if (!s) return 0;
  return __utf8_enc(s, wc);
}

size_t mbstowcs(wchar_t *pwcs, const char *s, size_t n) {
  mbstate_t st;
  size_t cnt = 0;
  st.c[0] = 0;
  while (cnt < n) {
    wchar_t w;
    size_t r = mbrtowc(&w, s, 4, &st);
    if (r == __MBERR || r == __MBINC) return __MBERR;
    if (pwcs) pwcs[cnt] = w;
    if (!w) return cnt;
    s += r;
    cnt++;
  }
  return cnt;
}

size_t wcstombs(char *s, const wchar_t *pwcs, size_t n) {
  size_t cnt = 0;
  char buf[4];
  while (cnt < n) {
    int r = __utf8_enc(buf, *pwcs);
    size_t i;
    if (r < 0) {
      errno = EILSEQ;
      return __MBERR;
    }
    if (!*pwcs) {
      if (s) s[cnt] = 0;
      return cnt;
    }
    if (cnt + (size_t)r > n) return cnt;
    if (s)
      for (i = 0; i < (size_t)r; i++) s[cnt + i] = buf[i];
    cnt += r;
    pwcs++;
  }
  return cnt;
}

wint_t btowc(int c) {
  if (c == -1 || (unsigned char)c >= 0x80) return WEOF;
  return (wint_t)(unsigned char)c;
}

int wctob(wint_t c) { return c <= 0x7f ? (int)c : -1; }

size_t wcslen(const wchar_t *s) {
  size_t n = 0;
  while (s[n]) n++;
  return n;
}

size_t wcsnlen(const wchar_t *s, size_t n) {
  size_t i = 0;
  while (i < n && s[i]) i++;
  return i;
}

int wcscmp(const wchar_t *s1, const wchar_t *s2) {
  while (*s1 && *s1 == *s2) {
    s1++;
    s2++;
  }
  return *s1 == *s2 ? 0 : (*s1 < *s2 ? -1 : 1);
}

int wcsncmp(const wchar_t *s1, const wchar_t *s2, size_t n) {
  while (n && *s1 && *s1 == *s2) {
    s1++;
    s2++;
    n--;
  }
  if (!n) return 0;
  return *s1 == *s2 ? 0 : (*s1 < *s2 ? -1 : 1);
}

size_t mbrtoc32(char32_t *pc32, const char *s, size_t n, mbstate_t *ps) { return mbrtowc((wchar_t *)pc32, s, n, ps); }

size_t c32rtomb(char *s, char32_t c32, mbstate_t *ps) { return wcrtomb(s, c32, ps); }

size_t mbrtoc16(char16_t *pc16, const char *s, size_t n, mbstate_t *ps) {
  wchar_t w;
  size_t r;
  if (!ps) ps = &__mbs_c16i;
  if (ps->c[0] == 0xff) {
    /* The low surrogate of the pair decoded by the previous call. */
    ps->c[0] = 0;
    if (pc16) *pc16 = ((char16_t)ps->c[1] << 8) | ps->c[2];
    return __MBSURR;
  }
  r = mbrtowc(&w, s, n, ps);
  if (r == __MBERR || r == __MBINC) return r;
  if (w >= 0x10000) {
    unsigned long x = w - 0x10000;
    unsigned int lo = 0xdc00 + (unsigned int)(x & 0x3ff);
    ps->c[0] = 0xff;
    ps->c[1] = (unsigned char)(lo >> 8);
    ps->c[2] = (unsigned char)lo;
    if (pc16) *pc16 = (char16_t)(0xd800 + (unsigned int)(x >> 10));
  } else if (pc16)
    *pc16 = (char16_t)w;
  return r;
}

size_t c16rtomb(char *s, char16_t c16, mbstate_t *ps) {
  if (!ps) ps = &__mbs_c16o;
  if (ps->c[0] == 0xff) {
    unsigned int hi = ((unsigned int)ps->c[1] << 8) | ps->c[2];
    ps->c[0] = 0;
    if (c16 >= 0xdc00 && c16 <= 0xdfff)
      return wcrtomb(s, 0x10000UL + ((unsigned long)(hi - 0xd800) << 10) + (c16 - 0xdc00), 0);
    errno = EILSEQ;
    return __MBERR;
  }
  if (c16 >= 0xd800 && c16 <= 0xdbff) {
    ps->c[0] = 0xff;
    ps->c[1] = (unsigned char)(c16 >> 8);
    ps->c[2] = (unsigned char)c16;
    return 0;
  }
  if (c16 >= 0xdc00 && c16 <= 0xdfff) {
    errno = EILSEQ;
    return __MBERR;
  }
  return wcrtomb(s, c16, 0);
}

size_t __mbstoc16s(char16_t *c16s, const char *s, size_t n) {
  mbstate_t st;
  size_t cnt = 0;
  st.c[0] = 0;
  while (cnt < n) {
    char16_t c;
    size_t r = mbrtoc16(&c, s, 4, &st);
    if (r == __MBERR || r == __MBINC) return __MBERR;
    c16s[cnt] = c;
    if (r == __MBSURR) {
      cnt++;
      continue;
    }
    if (!c) return cnt;
    s += r;
    cnt++;
  }
  return cnt;
}

size_t __c16stombs(char *s, const char16_t *c16s, size_t n) {
  mbstate_t st;
  size_t cnt = 0;
  char buf[4];
  st.c[0] = 0;
  while (cnt < n) {
    size_t i, r = c16rtomb(buf, *c16s, &st);
    if (r == __MBERR) return __MBERR;
    if (!*c16s) {
      s[cnt] = 0;
      return cnt;
    }
    c16s++;
    if (!r) continue;
    if (cnt + r > n) return cnt;
    for (i = 0; i < r; i++) s[cnt + i] = buf[i];
    cnt += r;
  }
  return cnt;
}

/* ---- stdatomic.h ---- */

#include <stdatomic.h>

_Bool atomic_flag_test_and_set(volatile atomic_flag *object) {
  _Bool r;
  __critical {
    r = object->flag;
    object->flag = 1;
  }
  return r;
}

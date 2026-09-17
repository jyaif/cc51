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

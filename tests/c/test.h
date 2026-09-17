#ifndef TEST_H
#define TEST_H
#include <8052.h>
#include <stdint.h>
#include <stdbool.h>

static void putch(char c) { SBUF = c; }

static void puts_(const char *s) {
  while (*s) putch(*s++);
}

static void put_hex8(uint8_t v) {
  putch("0123456789abcdef"[v >> 4]);
  putch("0123456789abcdef"[v & 15]);
}

static void put_hex16(uint16_t v) {
  put_hex8(v >> 8);
  put_hex8(v);
}

static void put_hex32(uint32_t v) {
  put_hex16(v >> 16);
  put_hex16(v);
}

static void put_u16(uint16_t v) {
  char buf[6];
  uint8_t i = 5;
  buf[5] = 0;
  do {
    buf[--i] = '0' + v % 10;
    v /= 10;
  } while (v);
  puts_(buf + i);
}

static void put_s16(int16_t v) {
  if (v < 0) {
    putch('-');
    v = -v;
  }
  put_u16(v);
}

static void put_u32(uint32_t v) {
  char buf[11];
  uint8_t i = 10;
  buf[10] = 0;
  do {
    buf[--i] = '0' + v % 10;
    v /= 10;
  } while (v);
  puts_(buf + i);
}

static void put_s32(int32_t v) {
  if (v < 0) {
    putch('-');
    v = -v;
  }
  put_u32(v);
}

static void nl(void) { putch('\n'); }

#endif

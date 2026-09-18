/* Soft float fuzz test: every operation on special values and 6000 pseudo-random pairs.
   tests/float_fuzz.sh compares the output with tests/float_model.py. */
#include "c/test.h"

typedef union { uint32_t u; float f; long l; } fu;

static uint32_t seed = 12345;
static uint32_t rnd(void) {
  seed = seed * 1103515245UL + 12345UL;
  return seed;
}

static const uint32_t special[] = {
  0x00000000UL, 0x80000000UL, 0x3f800000UL, 0xbf800000UL, 0x7f800000UL, 0xff800000UL, 0x7fc00000UL,
  0x7f7fffffUL, 0x00800000UL, 0x00000001UL, 0x3f800001UL, 0x3fc00000UL, 0x4f000000UL, 0xcf000000UL,
  0x4f800000UL, 0x4b7fffffUL, 0x3effffffUL, 0x7f000000UL, 0x01000000UL, 0x33800000UL,
};
#define NSPEC (sizeof(special) / sizeof(special[0]))

static void one(uint32_t x, uint32_t y) {
  fu a, b, r;
  a.u = x;
  b.u = y;
  r.f = a.f + b.f; put_hex32(r.u); putch(' ');
  r.f = a.f - b.f; put_hex32(r.u); putch(' ');
  r.f = a.f * b.f; put_hex32(r.u); putch(' ');
  r.f = a.f / b.f; put_hex32(r.u); putch(' ');
  putch('0' + (a.f < b.f)); putch('0' + (a.f == b.f));
  putch(' ');
  put_hex32((uint32_t)(long)a.f); putch(' ');
  put_hex32((unsigned long)a.f); putch(' ');
  r.f = (float)a.l; put_hex32(r.u); putch(' ');
  r.f = (float)a.u; put_hex32(r.u);
  putch('\n');
}

int main(void) {
  uint8_t i, j;
  uint16_t n;
  for (i = 0; i < NSPEC; i++)
    for (j = 0; j < NSPEC; j++) one(special[i], special[j]);
  for (n = 0; n < 6000; n++) {
    i = n;
    uint32_t x = rnd(), y = rnd();
    uint8_t k = (rnd() >> 16) & 7;
    /* bring exponents close together for most pairs so that additions interact */
    if (k < 5) y = (y & 0x807fffffUL) | ((x & 0x7f800000UL) + ((uint32_t)(k & 3) << 23)) & 0x7f800000UL;
    if (k == 5) y = x ^ ((rnd() >> 20) & 0xff);
    if (k == 7) y = (y & 0x807fffffUL) | ((x & 0x7f800000UL) - ((rnd() & 0x1f00000UL) << 3)) & 0x7f800000UL;
    if (k == 6) {
      uint32_t t = rnd();
      x = (t >> (rnd() & 31)) * ((i & 1) ? 1 : -1);
    }
    one(x, y);
  }
  return 0;
}

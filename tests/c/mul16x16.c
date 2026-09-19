#include "test.h"

/* 32-bit products of 16-bit operands use the 16x16->32 helpers; compare with full 32-bit multiplies. */
static const int16_t sv[] = {0, 1, -1, 2, -2, 127, -128, 255, 256, 1234, -5678, 32767, -32768, 0x55aa, -0x2b3c};
volatile int32_t w1, w2;

static int16_t s16(uint8_t i) { return sv[i]; }

int main(void) {
  uint8_t i, j, bad = 0;
  uint32_t sum = 0;
  for (i = 0; i < sizeof(sv) / 2; i++) {
    for (j = 0; j < sizeof(sv) / 2; j++) {
      int16_t a = s16(i), b = s16(j);
      uint16_t ua = a, ub = b;
      int32_t p = (int32_t)a * b;
      uint32_t up = (uint32_t)ua * ub;
      int32_t pk = (int32_t)a * -300;
      uint32_t upk = (uint32_t)ua * 40000u;
      w1 = a;
      w2 = b;
      if (p != w1 * w2) bad++;
      w1 = ua;
      w2 = ub;
      if (up != (uint32_t)w1 * (uint32_t)w2) bad++;
      w1 = a;
      if (pk != w1 * -300) bad++;
      w1 = ua;
      if (upk != (uint32_t)w1 * 40000u) bad++;
      sum += p ^ up;
    }
  }
  put_u16(bad); nl();
  put_hex32(sum); nl();
  return 0;
}
/* EXPECT:
0
121c0000
*/

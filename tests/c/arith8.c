#include "test.h"

uint8_t g8 = 200;
int8_t s8 = -5;

uint8_t add(uint8_t a, uint8_t b) { return a + b; }
uint8_t sub(uint8_t a, uint8_t b) { return a - b; }
uint8_t mul(uint8_t a, uint8_t b) { return a * b; }
uint8_t divu(uint8_t a, uint8_t b) { return a / b; }
uint8_t modu(uint8_t a, uint8_t b) { return a % b; }
int8_t divs(int8_t a, int8_t b) { return a / b; }
int8_t mods(int8_t a, int8_t b) { return a % b; }
uint8_t shl(uint8_t a, uint8_t n) { return a << n; }
uint8_t shr(uint8_t a, uint8_t n) { return a >> n; }
int8_t sar(int8_t a, uint8_t n) { return a >> n; }

int main(void) {
  put_hex8(add(g8, 100)); nl();
  put_hex8(sub(3, 5)); nl();
  put_hex8(mul(13, 21)); nl();
  put_hex8(divu(200, 7)); nl();
  put_hex8(modu(200, 7)); nl();
  put_hex8(divs(s8 * 20, 7)); nl();
  put_hex8(mods(s8 * 20, 7)); nl();
  put_hex8(shl(0x35, 3)); nl();
  put_hex8(shr(0xb5, 3)); nl();
  put_hex8(sar(-100, 2)); nl();
  for (uint8_t i = 0; i < 8; i++) put_hex8(shl(1, i));
  nl();
  for (uint8_t i = 0; i < 8; i++) put_hex8(shr(0x80, i));
  nl();
  for (uint8_t i = 0; i < 8; i++) put_hex8(sar(-128, i));
  nl();
  uint8_t x = 0x5a;
  put_hex8(x << 1); put_hex8(x << 2); put_hex8(x << 3); put_hex8(x << 4);
  put_hex8(x << 5); put_hex8(x << 6); put_hex8(x << 7);
  nl();
  put_hex8(x >> 1); put_hex8(x >> 2); put_hex8(x >> 3); put_hex8(x >> 4);
  put_hex8(x >> 5); put_hex8(x >> 6); put_hex8(x >> 7);
  nl();
  int8_t y = -90;
  put_hex8(y >> 1); put_hex8(y >> 2); put_hex8(y >> 3); put_hex8(y >> 4);
  put_hex8(y >> 5); put_hex8(y >> 6); put_hex8(y >> 7);
  nl();
  put_hex8(~x); put_hex8(-x); put_hex8(!x); put_hex8(x & 0x0f); put_hex8(x | 0x81); put_hex8(x ^ 0xff);
  nl();
  return 0;
}
/* EXPECT:
2c
fe
11
1c
04
f2
fe
a8
16
e7
0102040810204080
8040201008040201
80c0e0f0f8fcfeff
b468d0a0408000
2d160b05020100
d3e9f4fafdfeff
a5a6000adba5
*/

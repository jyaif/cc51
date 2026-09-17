#include "test.h"

uint16_t ga = 40000, gb = 1234;
int16_t sa = -1234, sb = 77;

uint16_t f_add(uint16_t a, uint16_t b) { return a + b; }
uint16_t f_sub(uint16_t a, uint16_t b) { return a - b; }
uint16_t f_mul(uint16_t a, uint16_t b) { return a * b; }
uint16_t f_div(uint16_t a, uint16_t b) { return a / b; }
uint16_t f_mod(uint16_t a, uint16_t b) { return a % b; }
int16_t f_divs(int16_t a, int16_t b) { return a / b; }
int16_t f_mods(int16_t a, int16_t b) { return a % b; }
uint16_t f_shl(uint16_t a, uint8_t n) { return a << n; }
uint16_t f_shr(uint16_t a, uint8_t n) { return a >> n; }
int16_t f_sar(int16_t a, uint8_t n) { return a >> n; }

int main(void) {
  put_hex16(f_add(ga, gb)); nl();
  put_hex16(f_sub(gb, ga)); nl();
  put_hex16(f_mul(ga, gb)); nl();
  put_hex16(f_div(ga, gb)); nl();
  put_hex16(f_mod(ga, gb)); nl();
  put_s16(f_divs(sa, sb)); nl();
  put_s16(f_mods(sa, sb)); nl();
  put_s16(f_divs(-sa, -sb)); nl();
  put_s16(f_mods(sa, -sb)); nl();
  for (uint8_t i = 0; i < 16; i += 3) { put_hex16(f_shl(0x1234, i)); putch(' '); }
  nl();
  for (uint8_t i = 0; i < 16; i += 3) { put_hex16(f_shr(0x8234, i)); putch(' '); }
  nl();
  for (uint8_t i = 0; i < 16; i += 3) { put_hex16(f_sar(-30000, i)); putch(' '); }
  nl();
  uint16_t x = 0xa5c3;
  put_hex16(x << 1); put_hex16(x << 4); put_hex16(x << 8); put_hex16(x << 9); put_hex16(x << 15); nl();
  put_hex16(x >> 1); put_hex16(x >> 4); put_hex16(x >> 8); put_hex16(x >> 9); put_hex16(x >> 15); nl();
  int16_t y = (int16_t)0xa5c3;
  put_hex16(y >> 1); put_hex16(y >> 4); put_hex16(y >> 8); put_hex16(y >> 9); put_hex16(y >> 15); nl();
  put_hex16(~x); put_hex16(-x); put_hex16(x & 0xf0f0); put_hex16(x | 0x0101); put_hex16(x ^ 0xffff); nl();
  x++; put_hex16(x); x--; x--; put_hex16(x); nl();
  x = 0x00ff; x++; put_hex16(x); x--; put_hex16(x); nl();
  x += 300; put_hex16(x); x -= 1000; put_hex16(x); nl();
  x *= 3; put_hex16(x); x /= 7; put_hex16(x); x %= 100; put_hex16(x); nl();
  put_u16(65535); nl();
  put_s16(-32768); nl();
  return 0;
}
/* EXPECT:
a112
6892
2c80
0020
0200
-16
-2
-16
-2
1234 91a0 8d00 6800 4000 0000 
8234 1046 0208 0041 0008 0001 
8ad0 f15a fe2b ffc5 fff8 ffff 
4b865c30c30086008000
52e10a5c00a500520001
d2e1fa5cffa5ffd2ffff
5a3c5a3da0c0a5c35a3c
a5c4a5c2
010000ff
022bfe43
fac923d30047
65535
-32768
*/

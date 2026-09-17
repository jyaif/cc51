#include "test.h"

uint32_t ga = 3000000000UL, gb = 123456;
int32_t sa = -2000000000L, sb = 7777;

uint32_t f_add(uint32_t a, uint32_t b) { return a + b; }
uint32_t f_sub(uint32_t a, uint32_t b) { return a - b; }
uint32_t f_mul(uint32_t a, uint32_t b) { return a * b; }
uint32_t f_div(uint32_t a, uint32_t b) { return a / b; }
uint32_t f_mod(uint32_t a, uint32_t b) { return a % b; }
int32_t f_divs(int32_t a, int32_t b) { return a / b; }
int32_t f_mods(int32_t a, int32_t b) { return a % b; }
uint32_t f_shl(uint32_t a, uint8_t n) { return a << n; }
uint32_t f_shr(uint32_t a, uint8_t n) { return a >> n; }
int32_t f_sar(int32_t a, uint8_t n) { return a >> n; }

int main(void) {
  put_hex32(f_add(ga, gb)); nl();
  put_hex32(f_sub(gb, ga)); nl();
  put_hex32(f_mul(ga, gb)); nl();
  put_hex32(f_div(ga, gb)); nl();
  put_hex32(f_mod(ga, gb)); nl();
  put_s32(f_divs(sa, sb)); nl();
  put_s32(f_mods(sa, sb)); nl();
  put_s32(f_divs(-sa, -sb)); nl();
  put_s32(f_mods(sa, -sb)); nl();
  for (uint8_t i = 0; i < 32; i += 5) { put_hex32(f_shl(0x12345678, i)); putch(' '); }
  nl();
  for (uint8_t i = 0; i < 32; i += 5) { put_hex32(f_shr(0x82345678, i)); putch(' '); }
  nl();
  for (uint8_t i = 0; i < 32; i += 5) { put_hex32(f_sar(-300000000L, i)); putch(' '); }
  nl();
  uint32_t x = 0xdeadbeef;
  put_hex32(x << 3); put_hex32(x << 12); put_hex32(x << 24); nl();
  put_hex32(x >> 3); put_hex32(x >> 12); put_hex32(x >> 24); nl();
  int32_t y = (int32_t)x;
  put_hex32(y >> 3); put_hex32(y >> 12); put_hex32(y >> 24); nl();
  x++; put_hex32(x); x += 0x10000; put_hex32(x); x -= 0x1234567; put_hex32(x); nl();
  put_u32(4294967295UL); nl();
  put_s32(-2147483647L - 1); nl();
  uint16_t a16 = 50000;
  uint32_t w = (uint32_t)a16 * a16;
  put_u32(w); nl();
  int16_t n16 = -5;
  int32_t sw = n16;
  put_s32(sw * 1000000); nl();
  return 0;
}
/* EXPECT:
b2d24040
4d318440
05138000
00005eec
00004b00
-257168
-4464
-257168
-4464
12345678 468acf00 d159e000 2b3c0000 67800000 f0000000 00000000 
82345678 0411a2b3 00208d15 00010468 00000823 00000041 00000002 
ee1e5d00 ff70f2e8 fffb8797 ffffdc3c fffffee1 fffffff7 ffffffff 
f56df778dbeef000ef000000
1bd5b7dd000deadb000000de
fbd5b7ddfffdeadbffffffde
deadbef0deaebef0dd8b7989
4294967295
-2147483648
2500000000
-5000000
*/

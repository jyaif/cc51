#include "test.h"

static uint16_t counter;
uint8_t init_arr[4] = {0xde, 0xad, 0xbe, 0xef};
__bit gbit;
bool gbool = true;

typedef enum { RED, GREEN = 5, BLUE } Color;

uint8_t next_id(void) {
  static uint8_t id = 10;
  return id++;
}

void (*fp)(char);

uint8_t apply(uint8_t (*f)(uint8_t), uint8_t v) { return f(v); }
uint8_t twice(uint8_t v) { return v * 2; }
uint8_t inc(uint8_t v) { return v + 1; }

__bit bit_and(__bit a, __bit b) { return a & b; }

inline uint8_t sq(uint8_t x) { return x * x; }

int main(void) {
  counter += 5;
  put_u16(counter); nl();
  for (uint8_t i = 0; i < 4; i++) put_hex8(init_arr[i]);
  nl();
  put_u16(next_id()); put_u16(next_id()); put_u16(next_id()); nl();
  Color c = BLUE;
  put_u16(c); put_u16(sizeof(Color)); nl();
  fp = putch;
  fp('F'); fp('P'); nl();
  put_u16(apply(twice, 21)); put_u16(apply(inc, 41)); nl();
  gbit = 1;
  put_hex8(gbit); put_hex8(bit_and(gbit, 1)); put_hex8(bit_and(gbit, 0)); put_hex8(gbool); nl();
  gbit = !gbit;
  put_hex8(gbit); nl();
  __critical {
    counter++;
  }
  put_u16(counter); nl();
  put_u16(sq(12)); nl();
  const char msg[] = "local";
  puts_(msg); nl();
  uint8_t x = 3;
  uint8_t y = x++ + ++x;
  put_u16(y); put_u16(x); nl();
  int16_t z = -7;
  z = z * z - 50;
  put_s16(z); nl();
  return 0;
}
/* EXPECT:
5
deadbeef
101112
61
FP
4242
01010001
00
6
144
local
85
-1
*/

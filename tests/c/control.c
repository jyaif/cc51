#include "test.h"

uint8_t sw(uint8_t x) {
  switch (x) {
    case 0: return 'a';
    case 1: return 'b';
    case 2:
    case 3: return 'c';
    case 10: return 'd';
    case 200: return 'e';
    default: return '?';
  }
}

int16_t sw16(int16_t x) {
  switch (x) {
    case -1: return 1;
    case 1000: return 2;
    case 256: return 3;
    case 0: return 4;
  }
  return 0;
}

uint16_t fib(uint8_t n) {
  uint16_t a = 0, b = 1;
  while (n--) {
    uint16_t t = a + b;
    a = b;
    b = t;
  }
  return a;
}

uint8_t count_bits(uint16_t v) {
  uint8_t c = 0;
  for (; v; v >>= 1)
    if (v & 1) c++;
  return c;
}

int main(void) {
  for (uint8_t i = 0; i < 12; i++) putch(sw(i));
  putch(sw(200)); putch(sw(255)); nl();
  put_s16(sw16(-1)); put_s16(sw16(1000)); put_s16(sw16(256)); put_s16(sw16(0)); put_s16(sw16(5)); nl();
  for (uint8_t i = 0; i < 25; i++) { put_u16(fib(i)); putch(' '); }
  nl();
  put_u16(count_bits(0xffff)); put_u16(count_bits(0x8001)); put_u16(count_bits(0)); nl();
  // do-while / continue / break / goto
  uint8_t i = 0;
  do {
    i++;
    if (i == 3) continue;
    if (i == 7) break;
    put_hex8(i);
  } while (i < 10);
  nl();
  uint8_t n = 0;
again:
  n++;
  if (n < 5) goto again;
  put_hex8(n); nl();
  // nested loops
  uint16_t s = 0;
  for (uint8_t a = 0; a < 10; a++)
    for (uint8_t b = 0; b < a; b++)
      s += a * b;
  put_u16(s); nl();
  // short-circuit
  uint8_t k = 0;
  if (k != 0 && 10 / k > 1) putch('X'); else putch('Y');
  if (k == 0 || 10 / k > 1) putch('Z');
  k = 5;
  putch((k > 3 && k < 10) ? 'T' : 'F');
  putch((k < 3 || k > 10) ? 'T' : 'F');
  uint8_t r = (k > 3) + (k < 3) * 2;
  put_hex8(r);
  nl();
  return 0;
}
/* EXPECT:
abcc??????d?e?
12340
0 1 1 2 3 5 8 13 21 34 55 89 144 233 377 610 987 1597 2584 4181 6765 10946 17711 28657 46368 
1620
0102040506
05
870
YZTF01
*/

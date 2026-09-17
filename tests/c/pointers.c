#include "test.h"

uint8_t data_arr[10];
__xdata uint8_t xarr[10];
const uint8_t table[] = {10, 20, 30, 40, 50};
const uint16_t wtable[] = {1000, 2000, 3000};
const char *const names[] = {"zero", "one", "two"};

uint8_t sum(uint8_t *p, uint8_t n) {
  uint8_t s = 0;
  while (n--) s += *p++;
  return s;
}

uint8_t xsum(__xdata uint8_t *p, uint8_t n) {
  uint8_t s = 0;
  for (uint8_t i = 0; i < n; i++) s += p[i];
  return s;
}

void fill(uint8_t *p, uint8_t n, uint8_t v) {
  for (uint8_t i = 0; i < n; i++) p[i] = v + i;
}

void swap(uint16_t *a, uint16_t *b) {
  uint16_t t = *a;
  *a = *b;
  *b = t;
}

uint8_t strlen_(const char *s) {
  const char *p = s;
  while (*p) p++;
  return p - s;
}

int main(void) {
  fill(data_arr, 10, 3);
  put_hex8(sum(data_arr, 10)); nl();
  for (uint8_t i = 0; i < 10; i++) xarr[i] = i * i;
  put_hex8(xsum(xarr, 10)); nl();
  uint16_t s = 0;
  for (uint8_t i = 0; i < 5; i++) s += table[i];
  put_u16(s); nl();
  for (uint8_t i = 0; i < 3; i++) { put_u16(wtable[i]); putch(' '); }
  nl();
  for (uint8_t i = 0; i < 3; i++) { puts_(names[i]); putch(' '); put_u16(strlen_(names[i])); putch(' '); }
  nl();
  uint16_t a = 1, b = 2;
  swap(&a, &b);
  put_u16(a); put_u16(b); nl();
  uint8_t *p = &data_arr[2];
  p += 3;
  put_hex8(*p); put_hex8(p[-1]); put_hex8(*(p - 2)); put_u16(p - data_arr); nl();
  uint8_t local[4] = {1, 2, 3, 4};
  put_hex8(sum(local, 4)); nl();
  return 0;
}
/* EXPECT:
4b
1d
150
1000 2000 3000 
zero 4 one 3 two 3 
21
0807065
0a
*/

#include "test.h"

#define N 12
int16_t arr[N] = {5, -3, 99, 42, 0, -100, 7, 7, 1000, -1, 3, 8};

void isort(int16_t *a, uint8_t n) {
  for (uint8_t i = 1; i < n; i++) {
    int16_t v = a[i];
    int8_t j = i - 1;
    while (j >= 0 && a[j] > v) {
      a[j + 1] = a[j];
      j--;
    }
    a[j + 1] = v;
  }
}

uint16_t crc16(const uint8_t *p, uint8_t n) {
  uint16_t crc = 0xffff;
  while (n--) {
    crc ^= *p++;
    for (uint8_t k = 0; k < 8; k++) {
      if (crc & 1)
        crc = (crc >> 1) ^ 0xa001;
      else
        crc >>= 1;
    }
  }
  return crc;
}

uint32_t isqrt(uint32_t x) {
  uint32_t r = 0, bit = 1UL << 30;
  while (bit > x) bit >>= 2;
  while (bit) {
    if (x >= r + bit) {
      x -= r + bit;
      r = (r >> 1) + bit;
    } else {
      r >>= 1;
    }
    bit >>= 2;
  }
  return r;
}

uint8_t popcount(uint32_t v) {
  uint8_t c = 0;
  while (v) {
    v &= v - 1;
    c++;
  }
  return c;
}

uint16_t gcd(uint16_t a, uint16_t b) {
  while (b) {
    uint16_t t = a % b;
    a = b;
    b = t;
  }
  return a;
}

const uint8_t msg[] = "The quick brown fox jumps over the lazy dog";

int main(void) {
  isort(arr, N);
  for (uint8_t i = 0; i < N; i++) {
    put_s16(arr[i]);
    putch(' ');
  }
  nl();
  put_hex16(crc16(msg, sizeof(msg) - 1)); nl();
  put_u32(isqrt(1000000UL)); putch(' '); put_u32(isqrt(123456789UL)); nl();
  put_u16(popcount(0xdeadbeefUL)); putch(' '); put_u16(popcount(0)); nl();
  put_u16(gcd(1071, 462)); putch(' '); put_u16(gcd(65535, 257)); nl();
  // Sieve
  uint8_t sieve[64];
  for (uint8_t i = 0; i < 64; i++) sieve[i] = 1;
  for (uint8_t i = 2; i < 8; i++)
    if (sieve[i])
      for (uint8_t j = i * i; j < 64; j += i) sieve[j] = 0;
  for (uint8_t i = 2; i < 64; i++)
    if (sieve[i]) {
      put_u16(i);
      putch(',');
    }
  nl();
  return 0;
}
/* EXPECT:
-100 -3 -1 0 3 5 7 7 8 42 99 1000 
a89c
1000 11111
24 0
21 257
2,3,5,7,11,13,17,19,23,29,31,37,41,43,47,53,59,61,
*/

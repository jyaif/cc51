#include "test.h"

uint16_t fact(uint8_t n) { return n <= 1 ? 1 : n * fact(n - 1); }

uint16_t fib(uint8_t n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }

uint8_t is_even(uint8_t n);
uint8_t is_odd(uint8_t n) { return n == 0 ? 0 : is_even(n - 1); }
uint8_t is_even(uint8_t n) { return n == 0 ? 1 : is_odd(n - 1); }

uint16_t ackermann(uint8_t m, uint16_t n) {
  if (m == 0) return n + 1;
  if (n == 0) return ackermann(m - 1, 1);
  return ackermann(m - 1, ackermann(m, n - 1));
}

void hanoi(uint8_t n, char from, char to, char via) {
  if (!n) return;
  hanoi(n - 1, from, via, to);
  putch(from);
  putch(to);
  putch(' ');
  hanoi(n - 1, via, to, from);
}

uint8_t sum_digits(uint16_t v) {
  char buf[3];
  buf[0] = v % 10;
  if (v < 10) return buf[0];
  return buf[0] + sum_digits(v / 10);
}

int main(void) {
  put_u16(fact(7)); nl();
  put_u16(fib(15)); nl();
  put_u16(is_even(10)); put_u16(is_odd(7)); put_u16(is_even(7)); nl();
  put_u16(ackermann(2, 3)); nl();
  hanoi(3, 'A', 'C', 'B'); nl();
  put_u16(sum_digits(98765)); nl();
  return 0;
}
/* EXPECT:
5040
610
110
9
AC AB CB AC BA BC AC 
19
*/

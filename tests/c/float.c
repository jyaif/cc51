#include "test.h"

float fa = 1.5f, fb = -2.25f;

long ftol(float f) { return (long)f; }

int main(void) {
  float x = fa + fb;
  put_s32(ftol(x * 1000)); nl();
  put_s32(ftol(fa * fb * 1000)); nl();
  put_s32(ftol(fa / fb * 100000)); nl();
  put_s32(ftol((fa - fb) * 100)); nl();
  long n = 123456;
  float fn = n;
  put_s32(ftol(fn / 3)); nl();
  unsigned long u = 4000000000UL;
  float fu = u;
  put_u32((unsigned long)(fu / 1000)); nl();
  putch(fa < fb ? 'L' : 'G');
  putch(fb < fa ? 'L' : 'G');
  putch(fa == 1.5f ? 'E' : 'N');
  putch(fa != fb ? 'D' : 'S');
  putch(x <= -0.75f ? 'Y' : 'N');
  putch(x >= 0 ? 'P' : 'M');
  nl();
  float acc = 0;
  for (int i = 1; i <= 10; i++) acc += 1.0f / i;
  put_s32(ftol(acc * 1000000)); nl();
  long e = 0.5 + 10000 * (9600.0f - 9615.0f) / 9600.0f;
  put_s32(e); nl();
  put_u32((unsigned long)(0.5 + 11059200 / ((float)32 * 3))); nl();
  return 0;
}
/* EXPECT:
-750
-3375
-66666
375
41152
4000000
GLEDYM
2928968
-15
115200
*/

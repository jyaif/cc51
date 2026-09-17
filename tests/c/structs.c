#include "test.h"
#include <stddef.h>

typedef struct {
  uint8_t a;
  uint16_t b;
  int8_t c[3];
} S;

typedef struct {
  uint8_t kind : 4;
  uint8_t row : 4;
  int16_t x;
  uint8_t flag : 1;
  uint8_t val : 7;
} BF;

typedef union {
  uint16_t w;
  uint8_t b[2];
} U;

S gs = {1, 0x1234, {-1, -2, -3}};
S arr[3];
BF bf[2];

void show(S *p) {
  put_hex8(p->a); putch(' '); put_hex16(p->b); putch(' ');
  put_hex8(p->c[0]); put_hex8(p->c[1]); put_hex8(p->c[2]); nl();
}

S make(uint8_t a) {
  S s;
  s.a = a;
  s.b = a * 100;
  s.c[0] = a; s.c[1] = a + 1; s.c[2] = a + 2;
  return s;
}

uint8_t sum_struct(S s) {
  return s.a + s.c[0] + s.c[1] + s.c[2];
}

int main(void) {
  show(&gs);
  arr[1] = gs;
  arr[1].a = 9;
  show(&arr[1]);
  S t = make(5);
  show(&t);
  put_hex8(sum_struct(t)); nl();
  S u = {7, 8};
  show(&u);
  bf[0].kind = 5; bf[0].row = 10; bf[0].x = -300; bf[0].flag = 1; bf[0].val = 100;
  bf[1].kind = 15; bf[1].row = 0; bf[1].x = 300; bf[1].flag = 0; bf[1].val = 127;
  for (uint8_t i = 0; i < 2; i++) {
    put_hex8(bf[i].kind); put_hex8(bf[i].row); put_s16(bf[i].x); put_hex8(bf[i].flag); put_hex8(bf[i].val); putch(' ');
  }
  nl();
  bf[0].kind++;
  bf[0].val += 30;
  put_hex8(bf[0].kind); put_hex8(bf[0].row); put_hex8(bf[0].val); put_hex8(bf[0].flag); nl();
  U un;
  un.w = 0xabcd;
  put_hex8(un.b[0]); put_hex8(un.b[1]); nl();
  put_u16(sizeof(S)); put_u16(sizeof(BF)); put_u16(offsetof(S, c)); nl();
  return 0;
}
/* EXPECT:
01 1234 fffefd
09 1234 fffefd
05 01f4 050607
17
07 0008 000000
050a-3000164 0f00300007f 
060a0201
cdab
643
*/

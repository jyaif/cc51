#include "test.h"

void t8u(uint8_t a, uint8_t b) {
  putch(a < b ? '1' : '0'); putch(a <= b ? '1' : '0'); putch(a > b ? '1' : '0');
  putch(a >= b ? '1' : '0'); putch(a == b ? '1' : '0'); putch(a != b ? '1' : '0');
  putch(' ');
}
void t8s(int8_t a, int8_t b) {
  putch(a < b ? '1' : '0'); putch(a <= b ? '1' : '0'); putch(a > b ? '1' : '0');
  putch(a >= b ? '1' : '0'); putch(a == b ? '1' : '0'); putch(a != b ? '1' : '0');
  putch(' ');
}
void t16u(uint16_t a, uint16_t b) {
  putch(a < b ? '1' : '0'); putch(a <= b ? '1' : '0'); putch(a > b ? '1' : '0');
  putch(a >= b ? '1' : '0'); putch(a == b ? '1' : '0'); putch(a != b ? '1' : '0');
  putch(' ');
}
void t16s(int16_t a, int16_t b) {
  putch(a < b ? '1' : '0'); putch(a <= b ? '1' : '0'); putch(a > b ? '1' : '0');
  putch(a >= b ? '1' : '0'); putch(a == b ? '1' : '0'); putch(a != b ? '1' : '0');
  putch(' ');
}
void t32s(int32_t a, int32_t b) {
  putch(a < b ? '1' : '0'); putch(a <= b ? '1' : '0'); putch(a > b ? '1' : '0');
  putch(a >= b ? '1' : '0'); putch(a == b ? '1' : '0'); putch(a != b ? '1' : '0');
  putch(' ');
}
void kc(int16_t a) {
  if (a < 0) putch('n'); else putch('p');
  if (a > 100) putch('g'); else putch('l');
  if (a <= -100) putch('m'); else putch('_');
  if (a == 7) putch('7'); else putch('x');
  if (a != 300) putch('y'); else putch('3');
  if (a >= 300) putch('B'); else putch('s');
  putch(' ');
}
void kc8(uint8_t a) {
  if (a < 10) putch('a');
  if (a > 200) putch('b');
  if (a <= 10) putch('c');
  if (a >= 200) putch('d');
  if (a == 0) putch('z');
  if (!a) putch('Z');
  if (a) putch('N');
  putch(' ');
}

int main(void) {
  t8u(1, 2); t8u(2, 1); t8u(5, 5); t8u(0, 255); t8u(255, 0); nl();
  t8s(1, 2); t8s(2, 1); t8s(-5, -5); t8s(-128, 127); t8s(127, -128); t8s(-1, 0); nl();
  t16u(1, 2); t16u(0x100, 0xff); t16u(5, 5); t16u(0, 65535); t16u(0x1234, 0x1235); nl();
  t16s(1, 2); t16s(-300, 200); t16s(-5, -5); t16s(-32768, 32767); t16s(32767, -32768); t16s(-1, 0); nl();
  t32s(1, 2); t32s(-300000, 200); t32s(-5, -5); t32s(-2147483647L, 2147483647L); t32s(100000, 99999); nl();
  kc(-5); kc(0); kc(7); kc(101); kc(-100); kc(-101); kc(300); kc(301); nl();
  kc8(0); kc8(9); kc8(10); kc8(11); kc8(199); kc8(200); kc8(201); kc8(255); nl();
  return 0;
}
/* EXPECT:
110001 001101 010110 110001 001101 
110001 001101 010110 110001 001101 110001 
110001 001101 010110 110001 110001 
110001 110001 010110 110001 001101 110001 
110001 110001 010110 110001 001101 
nl_xys pl_xys pl_7ys pg_xys nlmxys nlmxys pg_x3B pg_xyB 
aczZ acN cN N N dN bdN bdN 
*/

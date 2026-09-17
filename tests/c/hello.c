#include <8052.h>

void putchar_(char c) {
  SBUF = c;
}

void puts_(const char *s) {
  while (*s) putchar_(*s++);
}

int main(void) {
  puts_("hello\n");
  return 0;
}
/* EXPECT:
hello
*/

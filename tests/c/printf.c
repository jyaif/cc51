#include <8052.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

int putchar(int c) {
  SBUF = c;
  return c;
}

const char *names[] = {"alpha", "beta"};
char ram[16] = "ram-string";

void logmsg(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  printf("[log] ");
  vprintf(fmt, ap);
  va_end(ap);
}

int sum(int n, ...) {
  va_list ap;
  int s = 0;
  va_start(ap, n);
  while (n--) s += va_arg(ap, int);
  va_end(ap);
  return s;
}

int main(void) {
  char buf[32];
  printf("hello %d %u %x %X %o\n", -42, 65000u, 0xbeef, 0xbeef, 8);
  printf("[%5d] [%-5d] [%05d] [%c] [%%]\n", 42, 42, -42, 'z');
  printf("long %ld %lu %lx\n", -100000L, 4000000000UL, 0xdeadbeefUL);
  printf("str %s %s %s\n", names[0], names[1], ram);
  int n = sprintf(buf, "%02x:%s:%d", 10, "abc", 123);
  printf("sprintf -> '%s' (%d, strlen %u)\n", buf, n, strlen(buf));
  logmsg("value=%d name=%s\n", 7, "seven");
  printf("sum=%d\n", sum(4, 1, 2, 3, 4));
  printf("cmp %d %d %d\n", strcmp("abc", "abd") < 0, strcmp("b", "a") > 0, strcmp("x", "x"));
  strcpy(buf, "copy");
  strcat(buf, "-cat");
  printf("%s %d %d %ld\n", buf, atoi("-123"), abs(-5), labs(-70000L));
  memset(buf, 'x', 3);
  buf[3] = 0;
  puts(buf);
  return 0;
}
/* EXPECT:
hello -42 65000 beef BEEF 10
[   42] [42   ] [-0042] [z] [%]
long -100000 4000000000 deadbeef
str alpha beta ram-string
sprintf -> '0a:abc:123' (10, strlen 10)
[log] value=7 name=seven
sum=10
cmp 1 1 0
copy-cat -123 5 70000
xxx
*/

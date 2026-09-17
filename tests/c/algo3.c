#include "test.h"
#include <string.h>

typedef struct {
  char name[8];
  uint8_t age;
  int16_t score;
} rec_t;

rec_t recs[4];

void set_rec(rec_t *r, const char *n, uint8_t age, int16_t score) {
  uint8_t i = 0;
  while (n[i] && i < 7) {
    r->name[i] = n[i];
    i++;
  }
  r->name[i] = 0;
  r->age = age;
  r->score = score;
}

rec_t *best(void) {
  rec_t *b = &recs[0];
  for (uint8_t i = 1; i < 4; i++)
    if (recs[i].score > b->score) b = &recs[i];
  return b;
}

int16_t total(const rec_t *r, uint8_t n) {
  int16_t t = 0;
  for (; n; n--, r++) t += r->score;
  return t;
}

/* 2D array */
uint8_t grid[4][5];

/* state machine parsing digits */
long parse(const char *s) {
  long v = 0;
  int8_t sign = 1;
  enum { START, NUM, DONE } st = START;
  while (st != DONE) {
    char c = *s++;
    switch (st) {
      case START:
        if (c == '-') { sign = -1; st = NUM; }
        else if (c >= '0' && c <= '9') { v = c - '0'; st = NUM; }
        else if (c == 0) st = DONE;
        break;
      case NUM:
        if (c >= '0' && c <= '9') v = v * 10 + (c - '0');
        else st = DONE;
        break;
      default:
        break;
    }
  }
  return v * sign;
}

int main(void) {
  set_rec(&recs[0], "alice", 30, 120);
  set_rec(&recs[1], "bob", 25, -40);
  set_rec(&recs[2], "carolineX", 41, 300);
  set_rec(&recs[3], "dan", 19, 299);
  rec_t *b = best();
  puts_(b->name); putch(' '); put_u16(b->age); putch(' '); put_s16(total(recs, 4)); nl();
  rec_t copy = recs[1];
  copy.score += 1000;
  put_s16(copy.score); putch(' '); put_s16(recs[1].score); putch(' '); puts_(copy.name); nl();
  for (uint8_t y = 0; y < 4; y++)
    for (uint8_t x = 0; x < 5; x++) grid[y][x] = y * 5 + x;
  uint16_t s = 0;
  for (uint8_t y = 0; y < 4; y++) s += grid[y][4 - y];
  put_u16(s); nl();
  put_s32(parse("12345")); putch(' '); put_s32(parse("-987654")); putch(' '); put_s32(parse("x")); nl();
  char buf[16];
  memset(buf, 0, sizeof buf);
  memcpy(buf, "hello", 5);
  memmove(buf + 2, buf, 5);
  puts_(buf); nl();
  put_u16(strlen(buf)); putch(' '); put_u16(sizeof(rec_t)); nl();
  return 0;
}
/* EXPECT:
carolin 41 679
960 -40 bob
40
12345 -987654 0
hehello
7 11
*/

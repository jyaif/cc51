#include "test.h"

typedef struct node {
  uint8_t value;
  struct node *next;
} node_t;

node_t pool[8];
node_t *head;

void push(uint8_t v, uint8_t idx) {
  node_t *n = &pool[idx];
  n->value = v;
  n->next = head;
  head = n;
}

void reverse(void) {
  node_t *prev = 0, *cur = head;
  while (cur) {
    node_t *nx = cur->next;
    cur->next = prev;
    prev = cur;
    cur = nx;
  }
  head = prev;
}

void print_list(void) {
  for (node_t *p = head; p; p = p->next) {
    put_u16(p->value);
    putch(p->next ? '>' : '\n');
  }
}

/* tiny stack machine */
enum { OP_PUSH, OP_ADD, OP_MUL, OP_DUP, OP_SWAP, OP_PRINT, OP_JNZ, OP_DEC, OP_HALT };
const uint8_t prog[] = {
    OP_PUSH, 1,   // acc
    OP_PUSH, 5,   // counter
    // loop (pc=4): acc *= counter
    OP_SWAP, OP_PUSH, 0, OP_ADD,  // just exercise
    OP_SWAP, OP_DUP,              // acc counter counter
    OP_PRINT,                     // prints counter
    OP_DEC,                       // acc counter counter-1 ... simplified
    OP_JNZ, 4,
    OP_HALT,
};

int16_t stack[16];
uint8_t sp;

void run(void) {
  uint8_t pc = 0;
  for (uint8_t steps = 0; steps < 100; steps++) {
    uint8_t op = prog[pc++];
    switch (op) {
      case OP_PUSH: stack[sp++] = prog[pc++]; break;
      case OP_ADD: sp--; stack[sp - 1] += stack[sp]; break;
      case OP_MUL: sp--; stack[sp - 1] *= stack[sp]; break;
      case OP_DUP: stack[sp] = stack[sp - 1]; sp++; break;
      case OP_SWAP: {
        int16_t t = stack[sp - 1];
        stack[sp - 1] = stack[sp - 2];
        stack[sp - 2] = t;
        break;
      }
      case OP_PRINT: sp--; put_s16(stack[sp]); putch(' '); break;
      case OP_DEC: stack[sp - 1]--; break;
      case OP_JNZ: {
        uint8_t target = prog[pc++];
        if (stack[sp - 1]) pc = target;
        break;
      }
      case OP_HALT: nl(); return;
      default: puts_("bad op"); return;
    }
  }
  puts_("steps!");
  nl();
}

/* bit-level ops */
uint8_t reverse_bits(uint8_t b) {
  uint8_t r = 0;
  for (uint8_t i = 0; i < 8; i++) {
    r = (r << 1) | (b & 1);
    b >>= 1;
  }
  return r;
}

uint16_t rotl16(uint16_t v, uint8_t n) { return (v << n) | (v >> (16 - n)); }

int main(void) {
  for (uint8_t i = 0; i < 5; i++) push(i * 10, i);
  print_list();
  reverse();
  print_list();
  run();
  for (uint8_t i = 0; i < 4; i++) {
    put_hex8(reverse_bits(0x01 << i | 0x30));
    putch(' ');
  }
  nl();
  for (uint8_t i = 1; i < 16; i += 5) {
    put_hex16(rotl16(0x8421, i));
    putch(' ');
  }
  nl();
  return 0;
}
/* EXPECT:
40>30>20>10>0
0>10>20>30>40
5 4 3 2 1 
8c 4c 2c 1c 
0843 0861 0c21 
*/

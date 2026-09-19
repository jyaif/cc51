#include "test.h"

/* SDCC local labels (00001$) belong to their asm block: two functions may use the same ones. */
uint8_t count_a(uint8_t n) __naked {
  (void)n;
  __asm
	mov	r7,dpl
	clr	a
00001$:
	add	a,#2
	djnz	r7,00001$
	mov	dpl,a
	ret
  __endasm;
}

uint8_t count_b(uint8_t n) __naked {
  (void)n;
  __asm
	mov	r7,dpl
	clr	a
00001$:
	add	a,#3
	djnz	r7,00001$
	mov	dpl,a
	ret
  __endasm;
}

int main(void) {
  put_u16(count_a(5)); nl();
  put_u16(count_b(5)); nl();
  return 0;
}
/* EXPECT:
10
15
*/

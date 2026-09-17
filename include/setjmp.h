#ifndef __CC51_SETJMP_H
#define __CC51_SETJMP_H
/* Stack pointer and return address of the setjmp call. */
typedef unsigned char jmp_buf[3];
int setjmp(jmp_buf env);
void longjmp(jmp_buf env, int val);
#endif

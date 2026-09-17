#ifndef __CC51_STDIO_H
#define __CC51_STDIO_H
#include <stdarg.h>
#include <stddef.h>
#define EOF (-1)
/* Must be provided by the program. */
int putchar(int c);
int getchar(void);
int printf(const char *fmt, ...);
int sprintf(char *buf, const char *fmt, ...);
int vprintf(const char *fmt, va_list ap);
int vsprintf(char *buf, const char *fmt, va_list ap);
int puts(const char *s);
#endif

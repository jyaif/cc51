#ifndef __CC51_STDLIB_H
#define __CC51_STDLIB_H
#include <stddef.h>
#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
#define RAND_MAX 32767
int abs(int j);
long labs(long j);
int rand(void);
void srand(unsigned int seed);
int atoi(const char *s);
long atol(const char *s);
void *malloc(size_t n);
void free(void *p);
void *calloc(size_t n, size_t size);
void *realloc(void *p, size_t n);
void qsort(void *base, size_t n, size_t size, int (*cmp)(const void *, const void *));
void *bsearch(const void *key, const void *base, size_t n, size_t size, int (*cmp)(const void *, const void *));
char *__itoa(int value, char *buf, unsigned char radix);
char *__uitoa(unsigned int value, char *buf, unsigned char radix);
char *__ltoa(long value, char *buf, unsigned char radix);
char *__ultoa(unsigned long value, char *buf, unsigned char radix);
size_t memalignment(const void *p);
#endif

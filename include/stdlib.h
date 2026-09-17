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
#endif

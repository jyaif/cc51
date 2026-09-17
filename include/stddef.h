#ifndef __CC51_STDDEF_H
#define __CC51_STDDEF_H
typedef unsigned int size_t;
typedef int ptrdiff_t;
typedef unsigned char wchar_t;
typedef unsigned char max_align_t;
#define NULL ((void *)0)
#define offsetof(type, member) __builtin_offsetof(type, member)
#endif

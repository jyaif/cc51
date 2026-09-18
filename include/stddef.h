#ifndef __CC51_STDDEF_H
#define __CC51_STDDEF_H
#ifndef __SIZE_T_DEFINED
#define __SIZE_T_DEFINED
typedef unsigned int size_t;
#endif
typedef int ptrdiff_t;
#ifndef __WCHAR_T_DEFINED
#define __WCHAR_T_DEFINED
typedef unsigned long int wchar_t;
#endif
typedef unsigned char max_align_t;
#define NULL ((void *)0)
#define offsetof(type, member) __builtin_offsetof(type, member)
#endif

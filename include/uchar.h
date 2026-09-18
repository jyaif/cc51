#ifndef __CC51_UCHAR_H
#define __CC51_UCHAR_H

#ifndef __MBSTATE_T_DEFINED
#define __MBSTATE_T_DEFINED
typedef struct { unsigned char c[3]; } mbstate_t;
#endif

#ifndef __SIZE_T_DEFINED
#define __SIZE_T_DEFINED
typedef unsigned int size_t;
#endif

#if __STDC_VERSION__ >= 202311L
#ifndef __CHAR8_T_DEFINED
#define __CHAR8_T_DEFINED
typedef unsigned char char8_t;
#endif
#endif

#ifndef __CHAR16_T_DEFINED
#define __CHAR16_T_DEFINED
typedef unsigned int char16_t;
#endif

#ifndef __CHAR32_T_DEFINED
#define __CHAR32_T_DEFINED
typedef unsigned long int char32_t;
#endif

size_t mbrtoc16(char16_t *restrict pc16, const char *restrict s, size_t n, mbstate_t *restrict ps);
size_t c16rtomb(char *restrict s, char16_t c16, mbstate_t *restrict ps);
size_t mbrtoc32(char32_t *restrict pc32, const char *restrict s, size_t n, mbstate_t *restrict ps);
size_t c32rtomb(char *restrict s, char32_t c32, mbstate_t *restrict ps);

/* SDCC extensions */
size_t __mbstoc16s(char16_t *restrict c16s, const char *restrict s, size_t n);
size_t __c16stombs(char *restrict s, const char16_t *restrict c16s, size_t n);

#endif

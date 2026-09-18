#ifndef __CC51_WCHAR_H
#define __CC51_WCHAR_H

#ifndef __WCHAR_T_DEFINED
#define __WCHAR_T_DEFINED
typedef unsigned long int wchar_t;
#endif

#ifndef __SIZE_T_DEFINED
#define __SIZE_T_DEFINED
typedef unsigned int size_t;
#endif

#ifndef __MBSTATE_T_DEFINED
#define __MBSTATE_T_DEFINED
typedef struct { unsigned char c[3]; } mbstate_t;
#endif

#ifndef __WINT_T_DEFINED
#define __WINT_T_DEFINED
typedef unsigned long int wint_t;
#endif

#ifndef WEOF
#define WEOF 0xfffffffful
#endif

inline int iswblank(wint_t c)
{
  return ((wchar_t)c == L' ' || (wchar_t)c == L'\t');
}

int wcscmp(const wchar_t *s1, const wchar_t *s2);
int wcsncmp(const wchar_t *s1, const wchar_t *s2, size_t count);
size_t wcslen(const wchar_t *s);
size_t wcsnlen(const wchar_t *s, size_t n);

wint_t btowc(int c);
int wctob(wint_t c);

int mbsinit(const mbstate_t *ps);

size_t mbrlen(const char *restrict s, size_t n, mbstate_t *restrict ps);
size_t mbrtowc(wchar_t *restrict pwc, const char *restrict s, size_t n, mbstate_t *restrict ps);
size_t wcrtomb(char *restrict s, wchar_t wc, mbstate_t *restrict ps);

long int wcstol(const wchar_t *restrict nptr, wchar_t **restrict endptr, int base);
unsigned long int wcstoul(const wchar_t *restrict nptr, wchar_t **restrict endptr, int base);
long long int wcstoll(const wchar_t *restrict nptr, wchar_t **restrict endptr, int base);
unsigned long long int wcstoull(const wchar_t *restrict nptr, wchar_t **restrict endptr, int base);

#endif

#undef assert
#ifdef NDEBUG
#define assert(x) ((void)0)
#else
void __assert(const char *expr, const char *file, int line);
#define assert(x) ((x) ? (void)0 : __assert(#x, __FILE__, __LINE__))
#endif
#ifndef __cplusplus
#define static_assert _Static_assert
#endif

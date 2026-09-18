#undef assert
#ifdef NDEBUG
#define assert(...) ((void)0)
#else
void __assert(const char *expr, const char *file, int line);
#define assert(...) ((__VA_ARGS__) ? (void)0 : __assert(#__VA_ARGS__, __FILE__, __LINE__))
#endif
#ifndef __cplusplus
#define static_assert _Static_assert
#endif

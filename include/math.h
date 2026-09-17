#ifndef __CC51_MATH_H
#define __CC51_MATH_H
/* Single precision throughout: double is float on this target. */
#define M_E 2.718281828F
#define M_LOG2E 1.442695041F
#define M_LOG10E 0.434294482F
#define M_LN2 0.693147181F
#define M_LN10 2.302585093F
#define M_PI 3.141592654F
#define M_PI_2 1.570796327F
#define M_PI_4 0.785398163F
#define M_1_PI 0.318309886F
#define M_2_PI 0.636619772F
#define M_2_SQRTPI 1.128379167F
#define M_SQRT2 1.414213562F
#define M_SQRT1_2 0.707106781F
#define HUGE_VALF (__builtin_inff())
#define HUGE_VAL HUGE_VALF
#define INFINITY HUGE_VALF
#define NAN (__builtin_nanf())

#define isnan(x) ((x) != (x))
#define isinf(x) (!isnan(x) && isnan((x) - (x)))
#define isfinite(x) (!isnan((x) - (x)))
#define signbit(x) (__signbitf(x))
#define isnormal(x) (isfinite(x) && (x) != 0.0F)
#define fpclassify(x) (isnan(x) ? 0 : isinf(x) ? 1 : (x) == 0.0F ? 2 : 4)
#define FP_NAN 0
#define FP_INFINITE 1
#define FP_ZERO 2
#define FP_SUBNORMAL 3
#define FP_NORMAL 4

int __signbitf(float x);
float fabsf(float x);
float ldexpf(float x, int e);
float frexpf(float x, int *e);
float truncf(float x);
float floorf(float x);
float ceilf(float x);
float roundf(float x);
float fmodf(float x, float y);
float modff(float x, float *ip);
float sqrtf(float x);
float expf(float x);
float logf(float x);
float log10f(float x);
float sinf(float x);
float cosf(float x);
float tanf(float x);
float asinf(float x);
float acosf(float x);
float atanf(float x);
float atan2f(float y, float x);
float sinhf(float x);
float coshf(float x);
float tanhf(float x);
float powf(float x, float y);
float expm1f(float x);
float log1pf(float x);

/* The double forms are the same functions. */
#define fabs(x) fabsf(x)
#define ldexp(x, e) ldexpf(x, e)
#define frexp(x, e) frexpf(x, e)
#define trunc(x) truncf(x)
#define floor(x) floorf(x)
#define ceil(x) ceilf(x)
#define round(x) roundf(x)
#define fmod(x, y) fmodf(x, y)
#define modf(x, p) modff(x, p)
#define sqrt(x) sqrtf(x)
#define exp(x) expf(x)
#define log(x) logf(x)
#define log10(x) log10f(x)
#define sin(x) sinf(x)
#define cos(x) cosf(x)
#define tan(x) tanf(x)
#define asin(x) asinf(x)
#define acos(x) acosf(x)
#define atan(x) atanf(x)
#define atan2(y, x) atan2f(y, x)
#define sinh(x) sinhf(x)
#define cosh(x) coshf(x)
#define tanh(x) tanhf(x)
#define pow(x, y) powf(x, y)
#endif

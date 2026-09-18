#ifndef __CC51_STDCKDINT_H
#define __CC51_STDCKDINT_H

/* The result is computed in the widest type and checked by converting back. */
#define __CKD_IMPL(T, O) \
  (T *r, signed long long a, signed long long b) { \
    signed long long result = a O b; \
    *r = result; \
    return (*r != result); \
  }

#define __CKD_ULL_IMPL(T, O) \
  (T *r, unsigned long long a, unsigned long long b) { \
    unsigned long long result = a O b; \
    *r = result; \
    return (*r != result); \
  }

inline _Bool __ckd_add_schar __CKD_IMPL(signed char, +)
inline _Bool __ckd_add_uchar __CKD_IMPL(unsigned char, +)
inline _Bool __ckd_add_short __CKD_IMPL(short, +)
inline _Bool __ckd_add_ushort __CKD_IMPL(unsigned short, +)
inline _Bool __ckd_add_int __CKD_IMPL(int, +)
inline _Bool __ckd_add_uint __CKD_IMPL(unsigned int, +)
inline _Bool __ckd_add_long __CKD_IMPL(long, +)
inline _Bool __ckd_add_ulong __CKD_IMPL(unsigned long, +)

inline _Bool __ckd_sub_schar __CKD_IMPL(signed char, -)
inline _Bool __ckd_sub_uchar __CKD_IMPL(unsigned char, -)
inline _Bool __ckd_sub_short __CKD_IMPL(short, -)
inline _Bool __ckd_sub_ushort __CKD_IMPL(unsigned short, -)
inline _Bool __ckd_sub_int __CKD_IMPL(int, -)
inline _Bool __ckd_sub_uint __CKD_IMPL(unsigned int, -)
inline _Bool __ckd_sub_long __CKD_IMPL(long, -)
inline _Bool __ckd_sub_ulong __CKD_IMPL(unsigned long, -)

inline _Bool __ckd_mul_schar __CKD_IMPL(signed char, *)
inline _Bool __ckd_mul_uchar __CKD_IMPL(unsigned char, *)
inline _Bool __ckd_mul_short __CKD_IMPL(short, *)
inline _Bool __ckd_mul_ushort __CKD_IMPL(unsigned short, *)
inline _Bool __ckd_mul_int __CKD_IMPL(int, *)
inline _Bool __ckd_mul_uint __CKD_IMPL(unsigned int, *)
inline _Bool __ckd_mul_long __CKD_IMPL(long, *)
inline _Bool __ckd_mul_ulong __CKD_IMPL(unsigned long, *)

/* unsigned long times unsigned long overflows a signed long long. */
inline _Bool __ckd_mul_ulongull __CKD_ULL_IMPL(unsigned long, *)

#define __ckd_by_result(op, r, a, b) \
  _Generic((r), \
    signed char *: __ckd_##op##_schar((void *)(r), (a), (b)), \
    unsigned char *: __ckd_##op##_uchar((void *)(r), (a), (b)), \
    short *: __ckd_##op##_short((void *)(r), (a), (b)), \
    unsigned short *: __ckd_##op##_ushort((void *)(r), (a), (b)), \
    int *: __ckd_##op##_int((void *)(r), (a), (b)), \
    unsigned int *: __ckd_##op##_uint((void *)(r), (a), (b)), \
    long *: __ckd_##op##_long((void *)(r), (a), (b)), \
    unsigned long *: __ckd_##op##_ulong((void *)(r), (a), (b)))

extern _Bool __ckd_add_unimplemented(void *, unsigned long long, unsigned long long);
extern _Bool __ckd_sub_unimplemented(void *, unsigned long long, unsigned long long);
extern _Bool __ckd_mul_unimplemented(void *, unsigned long long, unsigned long long);

/* long long operands are not supported: there is no wider type to check in. */
#define __ckd_wide(op, r, a, b) \
  _Generic((a), \
    signed long long: __ckd_##op##_unimplemented(r, a, b), \
    unsigned long long: __ckd_##op##_unimplemented(r, a, b), \
    default: _Generic((b), \
      signed long long: __ckd_##op##_unimplemented(r, a, b), \
      unsigned long long: __ckd_##op##_unimplemented(r, a, b), \
      default: __ckd_by_result(op, r, a, b)))

#define ckd_add(r, a, b) __ckd_wide(add, r, a, b)
#define ckd_sub(r, a, b) __ckd_wide(sub, r, a, b)

#define ckd_mul(r, a, b) \
  _Generic((a), \
    signed long long: __ckd_mul_unimplemented(r, a, b), \
    unsigned long long: __ckd_mul_unimplemented(r, a, b), \
    unsigned long: _Generic((b), \
      signed long long: __ckd_mul_unimplemented(r, a, b), \
      unsigned long long: __ckd_mul_unimplemented(r, a, b), \
      unsigned long: __ckd_mul_ulongull((void *)(r), (a), (b)), \
      default: __ckd_by_result(mul, r, a, b)), \
    default: _Generic((b), \
      signed long long: __ckd_mul_unimplemented(r, a, b), \
      unsigned long long: __ckd_mul_unimplemented(r, a, b), \
      default: __ckd_by_result(mul, r, a, b)))

#endif

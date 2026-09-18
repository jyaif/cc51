#ifndef __CC51_STDBIT_H
#define __CC51_STDBIT_H
#define __STDC_VERSION_STDBIT_H__ 202311L
#define __STDC_ENDIAN_LITTLE__ 1234
#define __STDC_ENDIAN_BIG__ 4321
#define __STDC_ENDIAN_NATIVE__ __STDC_ENDIAN_LITTLE__

unsigned char __stdc_ones(unsigned long long v);
unsigned char __stdc_width(unsigned long long v);
unsigned char __stdc_ctz(unsigned long long v, unsigned char bits);

#define __STDC_BITS(x) ((unsigned char)(sizeof(x) * 8))
#define __STDC_MASK(x) (~0ULL >> (64 - __STDC_BITS(x)))
#define __STDC_INV(x) ((unsigned long long)(~(x)) & __STDC_MASK(x))

#define stdc_count_ones(x) ((unsigned int)__stdc_ones((unsigned long long)(x)))
#define stdc_count_zeros(x) ((unsigned int)(__STDC_BITS(x) - __stdc_ones((unsigned long long)(x))))
#define stdc_bit_width(x) ((unsigned int)__stdc_width((unsigned long long)(x)))
#define stdc_count_leading_zeros(x) ((unsigned int)(__STDC_BITS(x) - __stdc_width((unsigned long long)(x))))
#define stdc_count_leading_ones(x) ((unsigned int)(__STDC_BITS(x) - __stdc_width(__STDC_INV(x))))
#define stdc_count_trailing_zeros(x) ((unsigned int)__stdc_ctz((unsigned long long)(x), __STDC_BITS(x)))
#define stdc_count_trailing_ones(x) ((unsigned int)__stdc_ctz(__STDC_INV(x), __STDC_BITS(x)))
#define stdc_first_leading_one(x) ((unsigned int)((x) ? __STDC_BITS(x) - __stdc_width((unsigned long long)(x)) + 1 : 0))
#define stdc_first_leading_zero(x) ((unsigned int)(__STDC_INV(x) ? __STDC_BITS(x) - __stdc_width(__STDC_INV(x)) + 1 : 0))
#define stdc_first_trailing_one(x) ((unsigned int)((x) ? __stdc_ctz((unsigned long long)(x), __STDC_BITS(x)) + 1 : 0))
#define stdc_first_trailing_zero(x) ((unsigned int)(__STDC_INV(x) ? __stdc_ctz(__STDC_INV(x), __STDC_BITS(x)) + 1 : 0))
#define stdc_has_single_bit(x) ((_Bool)((x) && !((unsigned long long)(x) & ((unsigned long long)(x) - 1))))
#define stdc_bit_floor(x) ((x) ? (1ULL << (__stdc_width((unsigned long long)(x)) - 1)) & __STDC_MASK(x) : 0)
#define stdc_bit_ceil(x) ((unsigned long long)(x) <= 1 ? 1 : (1ULL << __stdc_width((unsigned long long)(x) - 1)) & __STDC_MASK(x))
#endif

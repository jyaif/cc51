#ifndef __CC51_STDATOMIC_H
#define __CC51_STDATOMIC_H

typedef struct { unsigned char flag; } atomic_flag;

#define ATOMIC_FLAG_INIT {0}

_Bool atomic_flag_test_and_set(volatile atomic_flag *object);

inline void atomic_flag_clear(volatile atomic_flag *object)
{
  object->flag = 0;
}

#endif

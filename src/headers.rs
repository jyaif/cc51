//! Built-in headers, embedded in the compiler binary.

macro_rules! headers {
    ($($name:expr),* $(,)?) => {
        pub fn get(name: &str) -> Option<&'static str> {
            match name {
                $($name => Some(include_str!(concat!("../include/", $name))),)*
                _ => None,
            }
        }
    };
}

headers!(
    "8051.h", "8052.h", "reg51.h", "reg52.h", "assert.h", "ctype.h", "errno.h", "float.h", "math.h", "setjmp.h", "stdbit.h", "stdckdint.h", "stdatomic.h", "iso646.h", "limits.h", "stdalign.h",
    "stdarg.h", "stdbool.h", "stddef.h", "stdint.h", "stdlib.h", "stdnoreturn.h", "string.h", "stdio.h", "wchar.h", "uchar.h",
);

pub const LIBC: &str = include_str!("../runtime/libc.c");

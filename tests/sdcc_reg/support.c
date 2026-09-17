/* cc51 regression-test support: characters go out through SBUF (captured by sim51). */
__sfr __at(0x99) SBUF;

void _putchar(char c) { SBUF = c; }
void _initEmu(void) {}
void _exitEmu(void) { for (;;) ; }

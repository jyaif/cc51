/* cc51 regression-test support: characters go out through SBUF (captured by sim51). */
__sfr __at(0x99) SBUF;
__sfr __at(0xA8) IE;

void _putchar(char c) { SBUF = c; }
void _initEmu(void) {}
void _exitEmu(void)
{
  IE = 0; /* stop timer interrupts so the simulator sees the halt loop */
  for (;;)
    ;
}

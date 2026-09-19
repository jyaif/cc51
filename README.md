# cc51

A C compiler, assembler and linker for the MCS-51 (8051), written from scratch in Rust with no
dependencies. It reads the dialect SDCC accepts — address-space qualifiers, `__interrupt`, `__at`,
inline assembly — and stands in for `sdcc` in an existing build, but it compiles the whole program
at once and optimizes hard for code size.

On the [minitel-native](https://github.com/jyaif/minitel-native) examples, against SDCC 4.6 for the
same sources and the same board (`nfz330`, bytes of ROM):

| example | SDCC | cc51 | |
|---|---|---|---|
| hello_world | 6130 | 4613 | 75% |
| video_stream | 6405 | 4475 | 70% |
| dino | 4005 | 2610 | 65% |
| dino_game | 9064 | 5642 | 62% |

Much of what is left in those numbers is data: of dino_game's 5642 bytes, 2247 are the splash
screen, a CRC table and sprites, so the code itself is under half of SDCC's.

## Building

```sh
cargo build --release
```

This produces three binaries in `target/release`: `cc51`, and two simulators used for testing,
`sim51` (a plain 8051) and `minitel_sim` (an 8051 with the video chip and keyboard of a Minitel).

## Using it

```sh
cc51 main.c display.c -o firmware.ihx
```

Compilation and linking are one step: every source file named on the command line becomes part of
one program, and all optimization happens with the whole program in view.

```
-o <file>          output (.hex/.ihx: Intel HEX, .bin: binary, with -c: object)
-c                 compile only          -E              preprocess only
-I<dir> -D<n>[=v] -U<n>                  -w              no warnings
-O0 / -O / -O2     optimization level (default -O2)
--code-loc <addr>  --code-size <n>       --iram-size <n>
--xram-loc <addr>  --xram-size <n>
--map <file>       --lst <file>          --size
--float=fast|small soft float: faster, or smaller and slower (same results)
--dump-ir          print the optimized IR and the register allocation
```

### In place of SDCC

`compat/bin` holds `sdcc`, `sdar` and `makebin` as symlinks to the `cc51` binary, which behaves as
whichever name it is invoked under. Put that directory first on `PATH` and a CMake or make build
written for SDCC uses cc51 instead:

```sh
PATH=/path/to/cc51/compat/bin:$PATH cmake /path/to/project && make
```

SDCC's own options (`-mmcs51`, `--std-c23`, `--model-small`, `--opt-code-size`, …) are accepted;
the ones that do not apply are ignored.

## What it compiles

C17 with the C23 additions SDCC has (`bool`, `nullptr`, `auto` as a type specifier, `_BitInt(N)`,
`static_assert`, `typeof`, digraphs and trigraphs, `u8`/`u`/`U`/`L` string and character literals),
plus SDCC's target extensions:

- Address spaces `__data`, `__idata`, `__pdata`, `__xdata`, `__code`, `__near`, `__far`, and
  `__bit`, `__sbit`, `__sfr`, `__sfr16`, `__sfr32` for hardware registers.
- `__at(addr)` on variables and inside declarators, `__interrupt(n)`, `__using(n)`, `__naked`,
  `__critical`, `__reentrant`, `__nonbanked`.
- `__asm ... __endasm` blocks and `__asm__("...")`, assembled by cc51 itself; preprocessor
  conditionals inside a block are evaluated before the assembler sees it.
- `#pragma save/restore/nooverlay`, `#pragma std_c*` and `#pragma std_sdcc*`.
- The library headers in `include/`: the freestanding ones plus `stdio.h` (printf family),
  `stdlib.h`, `string.h`, `math.h`, `ctype.h`, `setjmp.h`, `errno.h`, `wchar.h`, `uchar.h`,
  `stdbit.h`, `stdckdint.h`, `stdatomic.h`, and `8051.h`/`8052.h`/`reg51.h`/`reg52.h`.

## Optimizations

**Whole program.** All translation units are parsed into one program, so there is no point at which
the compiler has to be pessimistic about what another file might do.

- Unreferenced functions and variables never reach code generation.
- A function called once is inlined into its caller whatever its size; elsewhere inlining is
  weighed against the call sequence it would replace, counting the constant arguments it folds.
- Each function gets the calling convention that suits its own parameters — bytes in registers,
  the rest in its frame. A fixed convention (R7–R2 and a shared argument area) is used only where
  the callee cannot be known: address-taken functions, recursive ones and calls through pointers.
  `__naked` functions use SDCC's convention so hand-written callers keep working.
- Clobber sets are computed per function and propagated through the call graph, so a value can stay
  in a register across a call to a function that does not touch that register.
- Address spaces are inferred with a union-find solver: a pointer that only ever addresses internal
  RAM is one byte, `__xdata` or `__code` pointers are two, and the three-byte generic form with
  SDCC's space tag appears only where a pointer really can reach several spaces.

**On the IR** (a non-SSA control-flow graph of byte-width virtual registers):

- Constant folding, algebraic simplification and comparison narrowing.
- Copy and constant propagation, single-use temporary coalescing, dead code elimination.
- Demanded-bit narrowing: arithmetic whose upper bytes are never read is done at the narrower
  width, which on this target is the difference between one instruction and four.
- Control-flow simplification: block merging, jump threading, unreachable block removal.
- Counted loops: `i < K` on the latch becomes `i != K`, because an index stepping by one from a
  smaller constant meets the bound exactly — one `cjne` instead of a `cjne` and a `jc`.
- Induction-variable strength reduction: when every use of an index is the same offset from it, the
  loop counts over that offset, so `a[i] = c` becomes a pointer walk.
- Sinking: a value computed before a branch but used on one side only moves into that successor.

**Code generation.**

- Expression trees are folded so that intermediate values stay in the accumulator instead of being
  materialized into registers.
- Graph-colouring allocation over R0–R7, with byte-position constraints for multi-byte values,
  hints from the calling convention and from copies, and memory or bit slots for what is left.
  There is no stack frame: frames are static and overlaid between functions that cannot be live at
  once, and a recursive call saves only the frame bytes that are live across it.
- Machine-state tracking: the selector knows what the accumulator, B and DPTR already hold and
  skips loads that would reload them.
- A liveness-driven peephole over machine resources (registers, A, B, C, OV, DPTR): dead-store
  elimination, jump threading, tail calls, `add a,#1` to `inc a` where the flags are dead, moves
  folded through dead intermediates, runs of one constant loaded once through the accumulator,
  and `djnz` recognition.
- Procedural abstraction: repeated instruction sequences are factored into shared subroutines as
  long as the calls cost less than the duplication — 54 of them in the dino game.
- Branch relaxation picks the shortest working form (`sjmp`/`ajmp`/`ljmp`, inverted conditional plus
  a jump where the target is out of reach), and absolute sections are placed without pushing
  relocatable code around.

## Differences from SDCC

- **Objects are not compatible.** `-c` writes a file with the expected `.rel` name, but it holds
  preprocessed source, not machine code, because every decision is deferred to the link. cc51
  neither reads nor writes SDCC's `.rel`/`.lib` format, so its output cannot be mixed with objects
  or libraries built by SDCC. `sdar` produces a cc51 library, and the C library is built in.
- **The calling convention is chosen per function**, not SDCC's fixed DPL/DPH/B/ACC plus
  `_PARM_n` globals. Assembly that calls a C function by hand therefore needs that function marked
  `__naked`, which pins it to SDCC's convention.
- **One memory model.** `--model-small`, `--model-large` and friends are accepted and ignored;
  where a variable lives and how wide a pointer is are decided by inference instead. `__reentrant`
  is accepted and inert: recursion works without it.
- **`printf("%f")` prints the number.** SDCC's small-model printf prints `<NO FLOAT>`; this is the
  one deliberate difference in the regression results below. Soft float comes in two forms
  (`--float=fast`, `--float=small`) that agree bit for bit.
- **The library is a weak translation unit.** A program that defines `memcpy`, `printf` or even
  `__fsmul` replaces the library's version, including with a different signature.
- **Listings and maps are cc51's own.** There is no `.asm`, `.rst`, `.sym` or `.mem` output;
  `--lst` and `--map` describe the linked program, in which a function called once no longer
  exists as a separate routine.
- **Not implemented:** `__addressmod` (user-defined address spaces), banked calls, SDCC's assembly
  library routines and their calling convention, and macro bodies that expand to `__asm` text,
  whose instructions stay on one line. Targets other than MCS-51 are out of scope by design.

## Tests

```sh
tests/all.sh                    # everything below except the SDCC suite and the size comparison
tests/run.sh                    # compile tests/c/*.c, simulate, compare with the expected output
tests/float_fuzz.sh             # both soft-float variants against a Python reference model
tests/minitel_all.sh            # build every minitel example with SDCC and cc51, report sizes
SDCC_REG=<sdcc>/support/regression tests/sdcc_reg/run.py     # SDCC's regression suite
```

Current results:

| suite | result |
|---|---|
| `tests/c` | 17 of 17 programs produce their expected output |
| soft float | 6400 operations per variant, bit-exact against the reference model |
| dino differential | video-register writes match the SDCC build on `nfz330`, `nfz400` and `722039m` |
| SDCC regression suite | **4066 pass, 6 expected failures, 0 failures** |

The six expected failures are the features listed above: `addrspace`, `bug3475990` and
`genericnonintrinsicnaddr` need `__addressmod`, `libmullong_type_asm` calls SDCC's assembly
`_mullong` with SDCC's convention, `bug1505956` expands an `__asm` body from a macro, and
`snprintf_type_FLOAT` expects `<NO FLOAT>`. `tests/sdcc_reg/run.py` records `results.txt` and
prints a `REGRESSION` line for any test that passed before and no longer does.

The differential test compiles the dino game with both compilers and runs each in `minitel_sim`,
comparing every write to the video chip; it is what makes size changes safe to trust. It and the
size comparison need `sdcc` on `PATH` and a checkout of minitel-native (`$HOME/Code/minitel-native`,
or `M=<path>`); the regression runner needs the `support/regression` directory of an SDCC source
tree and Python 3.

## Layout

| | |
|---|---|
| `src/pp`, `src/lex.rs` | preprocessor (hidesets, digraphs, trigraphs) and C tokens |
| `src/parse`, `src/ast.rs` | parser and type checker in one pass, address-space inference |
| `src/ir` | lowering to the control-flow graph |
| `src/opt` | the IR passes above |
| `src/cg` | selection, allocation, peephole, outlining, RAM layout |
| `src/asm`, `src/link.rs` | assembler, relaxation, section placement, Intel HEX and binary output |
| `runtime/rt.s`, `runtime/libc.c` | assembly helpers and the C library, parsed last as a weak unit |
| `src/sim.rs`, `src/bin` | the simulators the tests run against |

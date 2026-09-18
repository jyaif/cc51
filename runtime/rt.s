; cc51 runtime library.
; Modules start with ";;; module <name>". A module is linked when one of its labels is referenced.
; Helper calling convention:
;   16-bit: arg0 in r7 (lo) / r6 (hi), arg1 in r3 (lo) / r2 (hi); result in r7 (lo) / r6 (hi)
;   32-bit: arg0 in r7..r4 (lo..hi), arg1 in r3..r0 (lo..hi); result in r7..r4
;   All registers, A, B, DPTR and PSW flags may be clobbered.

;;; module call_dptr
__call_dptr:
	clr	a
	jmp	@a+dptr

;;; module gptr
; A = *(generic pointer DPTR, B = tag)
__gptrget:
	mov	a,b
	jz	__gptrget_x
	jb	acc.7,__gptrget_c
	jb	acc.5,__gptrget_x
	push	ar0
	mov	r0,dpl
	mov	a,@r0
	pop	ar0
	ret
__gptrget_x:
	movx	a,@dptr
	ret
__gptrget_c:
	clr	a
	movc	a,@a+dptr
	ret
; *(generic pointer DPTR, B = tag) = A
__gptrput:
	push	acc
	mov	a,b
	jz	__gptrput_x
	jb	acc.7,__gptrput_c
	jb	acc.5,__gptrput_x
	pop	acc
	push	ar0
	mov	r0,dpl
	mov	@r0,a
	pop	ar0
	ret
__gptrput_x:
	pop	acc
	movx	@dptr,a
	ret
__gptrput_c:
	pop	acc
	ret

;;; module mulint
__mulint:
	mov	a,r7
	mov	b,r3
	mul	ab
	mov	r4,a
	mov	r5,b
	mov	a,r7
	mov	b,r2
	mul	ab
	add	a,r5
	mov	r5,a
	mov	a,r6
	mov	b,r3
	mul	ab
	add	a,r5
	mov	r6,a
	mov	a,r4
	mov	r7,a
	ret

;;; module divuint
__divuint:
	clr	F0
	sjmp	__udiv16
__moduint:
	setb	F0
__udiv16:
	clr	a
	mov	r4,a
	mov	r5,a
	mov	b,#16
__udiv16_loop:
	mov	a,r7
	add	a,r7
	mov	r7,a
	mov	a,r6
	rlc	a
	mov	r6,a
	mov	a,r5
	rlc	a
	mov	r5,a
	mov	a,r4
	rlc	a
	mov	r4,a
	mov	a,r5
	clr	c
	subb	a,r3
	mov	r1,a
	mov	a,r4
	subb	a,r2
	jc	__udiv16_next
	mov	r4,a
	mov	a,r1
	mov	r5,a
	inc	r7
__udiv16_next:
	djnz	b,__udiv16_loop
	jnb	F0,__udiv16_ret
	mov	a,r5
	mov	r7,a
	mov	a,r4
	mov	r6,a
__udiv16_ret:
	ret

;;; module divsint
__divsint:
	mov	a,r6
	xrl	a,r2
	push	acc
	call	__abs_args16
	lcall	__divuint
	pop	acc
	jnb	acc.7,__divsint_ret
	sjmp	__neg16
__divsint_ret:
	ret
__modsint:
	mov	a,r6
	push	acc
	call	__abs_args16
	lcall	__moduint
	pop	acc
	jnb	acc.7,__divsint_ret
__neg16:
	clr	c
	clr	a
	subb	a,r7
	mov	r7,a
	clr	a
	subb	a,r6
	mov	r6,a
	ret
__abs_args16:
	mov	a,r6
	jnb	acc.7,__abs_args16_b
	call	__neg16
__abs_args16_b:
	mov	a,r2
	jnb	acc.7,__abs_args16_ret
	clr	c
	clr	a
	subb	a,r3
	mov	r3,a
	clr	a
	subb	a,r2
	mov	r2,a
__abs_args16_ret:
	ret

;;; module divschar
; A = A / B (signed); A = A % B (signed)
__divschar_ab:
	mov	r7,a
	xrl	a,b
	mov	r6,a
	call	__abs_ab8
	div	ab
	xch	a,r6
	jnb	acc.7,__divschar_pos
	mov	a,r6
	cpl	a
	inc	a
	ret
__divschar_pos:
	mov	a,r6
	ret
__modschar_ab:
	mov	r7,a
	mov	r6,a
	call	__abs_ab8
	div	ab
	mov	a,r6
	jnb	acc.7,__modschar_pos
	mov	a,b
	cpl	a
	inc	a
	ret
__modschar_pos:
	mov	a,b
	ret
__abs_ab8:
	mov	a,r7
	jnb	acc.7,__abs_ab8_b
	cpl	a
	inc	a
__abs_ab8_b:
	xch	a,b
	jnb	acc.7,__abs_ab8_ret
	cpl	a
	inc	a
__abs_ab8_ret:
	xch	a,b
	ret

;;; module mullong
; r7..r4 * r3..r0 -> r7..r4
__mullong:
	clr	a
	mov	__rt_t0,a
	mov	__rt_t1,a
	mov	__rt_t2,a
	mov	__rt_t3,a
	mov	b,#32
__mullong_loop:
	clr	c
	mov	a,r0
	rrc	a
	mov	r0,a
	mov	a,r1
	rrc	a
	mov	r1,a
	mov	a,r2
	rrc	a
	mov	r2,a
	mov	a,r3
	rrc	a
	mov	r3,a
	jnc	__mullong_skip
	mov	a,__rt_t0
	add	a,r7
	mov	__rt_t0,a
	mov	a,__rt_t1
	addc	a,r6
	mov	__rt_t1,a
	mov	a,__rt_t2
	addc	a,r5
	mov	__rt_t2,a
	mov	a,__rt_t3
	addc	a,r4
	mov	__rt_t3,a
__mullong_skip:
	mov	a,r7
	add	a,r7
	mov	r7,a
	mov	a,r6
	rlc	a
	mov	r6,a
	mov	a,r5
	rlc	a
	mov	r5,a
	mov	a,r4
	rlc	a
	mov	r4,a
	djnz	b,__mullong_loop
	mov	r7,__rt_t0
	mov	r6,__rt_t1
	mov	r5,__rt_t2
	mov	r4,__rt_t3
	ret

;;; module divulong
; r7..r4 / r3..r0 -> r7..r4 (remainder in __rt_t0..3)
__divulong:
	clr	F0
	sjmp	__udiv32
__modulong:
	setb	F0
__udiv32:
	clr	a
	mov	__rt_t0,a
	mov	__rt_t1,a
	mov	__rt_t2,a
	mov	__rt_t3,a
	mov	b,#32
__udiv32_loop:
	mov	a,r7
	add	a,r7
	mov	r7,a
	mov	a,r6
	rlc	a
	mov	r6,a
	mov	a,r5
	rlc	a
	mov	r5,a
	mov	a,r4
	rlc	a
	mov	r4,a
	mov	a,__rt_t0
	rlc	a
	mov	__rt_t0,a
	mov	a,__rt_t1
	rlc	a
	mov	__rt_t1,a
	mov	a,__rt_t2
	rlc	a
	mov	__rt_t2,a
	mov	a,__rt_t3
	rlc	a
	mov	__rt_t3,a
	; trial subtraction into __rt_t4..7
	mov	a,__rt_t0
	clr	c
	subb	a,r3
	mov	__rt_t4,a
	mov	a,__rt_t1
	subb	a,r2
	mov	__rt_t5,a
	mov	a,__rt_t2
	subb	a,r1
	mov	__rt_t6,a
	mov	a,__rt_t3
	subb	a,r0
	jc	__udiv32_next
	mov	__rt_t3,a
	mov	__rt_t2,__rt_t6
	mov	__rt_t1,__rt_t5
	mov	__rt_t0,__rt_t4
	inc	r7
__udiv32_next:
	djnz	b,__udiv32_loop
	jnb	F0,__udiv32_ret
	mov	r7,__rt_t0
	mov	r6,__rt_t1
	mov	r5,__rt_t2
	mov	r4,__rt_t3
__udiv32_ret:
	ret

;;; module divslong
__divslong:
	mov	a,r4
	xrl	a,r0
	push	acc
	call	__abs_args32
	lcall	__divulong
	pop	acc
	jnb	acc.7,__divslong_ret
	sjmp	__neg32
__divslong_ret:
	ret
__modslong:
	mov	a,r4
	push	acc
	call	__abs_args32
	lcall	__modulong
	pop	acc
	jnb	acc.7,__divslong_ret
__neg32:
	clr	c
	clr	a
	subb	a,r7
	mov	r7,a
	clr	a
	subb	a,r6
	mov	r6,a
	clr	a
	subb	a,r5
	mov	r5,a
	clr	a
	subb	a,r4
	mov	r4,a
	ret
__abs_args32:
	mov	a,r4
	jnb	acc.7,__abs_args32_b
	call	__neg32
__abs_args32_b:
	mov	a,r0
	jnb	acc.7,__abs_args32_ret
	clr	c
	clr	a
	subb	a,r3
	mov	r3,a
	clr	a
	subb	a,r2
	mov	r2,a
	clr	a
	subb	a,r1
	mov	r1,a
	clr	a
	subb	a,r0
	mov	r0,a
__abs_args32_ret:
	ret

;;; module setjmp
; int setjmp(jmp_buf env): saves SP and the return address; returns 0.
_setjmp:
	mov	r0,ar7
	mov	a,sp
	mov	@r0,a
	add	a,#0xff
	mov	r1,a
	inc	r0
	mov	a,@r1
	mov	@r0,a
	inc	r0
	inc	r1
	mov	a,@r1
	mov	@r0,a
	mov	r7,#0
	mov	r6,#0
	ret
; void longjmp(jmp_buf env, int val): resumes at the matching setjmp, which returns val (or 1).
_longjmp:
	mov	r0,ar7
	mov	b,ie
	clr	ea
	mov	a,@r0
	add	a,#0xfe
	mov	sp,a
	inc	r0
	mov	a,@r0
	push	acc
	inc	r0
	mov	a,@r0
	push	acc
	mov	a,r3
	mov	r7,a
	mov	a,r2
	mov	r6,a
	orl	a,r7
	jnz	_longjmp_ret
	mov	r7,#1
	mov	r6,#0
_longjmp_ret:
	mov	ie,b
	ret

; ---- IEEE-754 single precision. Round to nearest with ties away from zero, denormal inputs read as
; 1.m * 2^-127, underflow to zero (tests/float_model.py is the reference). Modules tagged `fast` or
; `small` exist in two versions (--float=fast|small) that give identical results.

;;; module fsr_pack fast
; Pack mantissa r7..r4 (lo..hi) with value m * 2^(E - 158), E = r2:r3 (hi:lo, signed), sign in B (0x80 or 0).
__fsr_pack:
	mov	a,r4
	jnz	__fsr_pack_bits
	orl	a,r5
	orl	a,r6
	orl	a,r7
	jz	__fsr_zero
__fsr_pack_bytes:
	mov	a,r5
	mov	r4,a
	mov	a,r6
	mov	r5,a
	mov	a,r7
	mov	r6,a
	mov	r7,#0
	mov	a,r3
	add	a,#0xf8
	mov	r3,a
	mov	a,r2
	addc	a,#0xff
	mov	r2,a
	mov	a,r4
	jz	__fsr_pack_bytes
__fsr_pack_bits:
	jb	acc.7,__fsr_pack_round
	mov	a,r7
	add	a,r7
	mov	r7,a
	mov	a,r6
	rlc	a
	mov	r6,a
	mov	a,r5
	rlc	a
	mov	r5,a
	mov	a,r4
	rlc	a
	mov	r4,a
	cjne	r3,#0,__fsr_pack_dec
	dec	r2
__fsr_pack_dec:
	dec	r3
	sjmp	__fsr_pack_bits
__fsr_pack_round:
	jmp	__fsr_round

;;; module fsr_round
; Round and pack a normalized mantissa (r4.7 set); registers as for __fsr_pack.
__fsr_round:
	mov	a,r7
	add	a,#0x80
	clr	a
	addc	a,r6
	mov	r6,a
	clr	a
	addc	a,r5
	mov	r5,a
	clr	a
	addc	a,r4
	mov	r4,a
	jnc	__fsr_pack_exp
	mov	r4,#0x80
	inc	r3
	cjne	r3,#0,__fsr_pack_exp
	inc	r2
__fsr_pack_exp:
	mov	a,r2
	jb	acc.7,__fsr_zero
	jnz	__fsr_inf
	mov	a,r3
	jz	__fsr_zero
	cjne	a,#0xff,__fsr_pack_ok
__fsr_inf:
	mov	a,#0x7f
	orl	a,b
	mov	r4,a
	mov	r5,#0x80
	mov	r6,#0
	mov	r7,#0
	ret
__fsr_pack_ok:
	clr	c
	rrc	a
	orl	a,b
	xch	a,r4
	mov	acc.7,c
	xch	a,r5
	xch	a,r6
	mov	r7,a
	ret
__fsr_zero:
	mov	r4,b
	clr	a
	mov	r5,a
	mov	r6,a
	mov	r7,a
	ret
__fsr_qnan:
	mov	r4,#0x7f
	mov	r5,#0xc0
	mov	r6,#0
	mov	r7,#0
	ret

;;; module fsadd fast
; r7..r4 + r3..r0 -> r7..r4
__fssub:
	mov	a,r0
	xrl	a,#0x80
	mov	r0,a
__fsadd:
	; order the operands so that |a| >= |b|
	mov	a,r7
	clr	c
	subb	a,r3
	mov	a,r6
	subb	a,r2
	mov	a,r5
	subb	a,r1
	mov	a,r0
	anl	a,#0x7f
	mov	b,a
	mov	a,r4
	anl	a,#0x7f
	subb	a,b
	jnc	__fsadd_ordered
	mov	a,r7
	xch	a,r3
	mov	r7,a
	mov	a,r6
	xch	a,r2
	mov	r6,a
	mov	a,r5
	xch	a,r1
	mov	r5,a
	mov	a,r4
	xch	a,r0
	mov	r4,a
__fsadd_ordered:
	mov	a,r5
	rlc	a
	mov	a,r4
	rlc	a
	cjne	a,#0xff,__fsadd_finite
	; a is NaN or infinite
	mov	a,r5
	anl	a,#0x7f
	orl	a,r6
	orl	a,r7
	jnz	__fsadd_nan
	; inf - inf is NaN
	mov	a,r4
	xrl	a,r0
	jnb	acc.7,__fsadd_ret
	mov	a,r0
	orl	a,#0x80
	cjne	a,#0xff,__fsadd_ret
	cjne	r1,#0x80,__fsadd_ret
	mov	a,r2
	orl	a,r3
	jnz	__fsadd_ret
__fsadd_nan:
	jmp	__fsr_qnan
__fsadd_finite:
	mov	dph,a
	mov	a,r0
	anl	a,#0x7f
	orl	a,r1
	orl	a,r2
	orl	a,r3
	jnz	__fsadd_nonzero
	; b is zero: the result is a, or b when both are zero
	mov	a,r4
	anl	a,#0x7f
	orl	a,r5
	orl	a,r6
	orl	a,r7
	jnz	__fsadd_ret
	mov	a,r0
	mov	r4,a
__fsadd_ret:
	ret
__fsadd_nonzero:
	; dpl = exponent difference, B = sign, F0 = subtract
	mov	a,r1
	rlc	a
	mov	a,r0
	rlc	a
	xch	a,dph
	mov	dpl,a
	clr	c
	subb	a,dph
	mov	dph,dpl
	mov	dpl,a
	mov	a,r4
	anl	a,#0x80
	mov	b,a
	xrl	a,r0
	mov	c,acc.7
	mov	F0,c
	; mantissas << 8
	mov	a,r5
	orl	a,#0x80
	mov	r4,a
	mov	a,r6
	mov	r5,a
	mov	a,r7
	mov	r6,a
	mov	r7,#0
	mov	a,r1
	orl	a,#0x80
	mov	r0,a
	mov	a,r2
	mov	r1,a
	mov	a,r3
	mov	r2,a
	mov	r3,#0
	; align b; bits shifted out collect in r3.0
	mov	a,dpl
	add	a,#0xe6
	jnc	__fsadd_align_bytes
	; exponent difference >= 26: b only contributes a sticky bit
	clr	a
	mov	r0,a
	mov	r1,a
	mov	r2,a
	mov	r3,#0x20
	sjmp	__fsadd_aligned
__fsadd_align_bytes:
	mov	a,dpl
	add	a,#0xf8
	jnc	__fsadd_align_bits
	mov	dpl,a
	mov	a,r3
	jz	__fsadd_align_b0
	mov	a,#1
__fsadd_align_b0:
	orl	a,r2
	mov	r3,a
	mov	a,r1
	mov	r2,a
	mov	a,r0
	mov	r1,a
	mov	r0,#0
	sjmp	__fsadd_align_bytes
__fsadd_align_bits:
	mov	a,dpl
	jz	__fsadd_align_done
__fsadd_align_bit:
	clr	c
	mov	a,r0
	rrc	a
	mov	r0,a
	mov	a,r1
	rrc	a
	mov	r1,a
	mov	a,r2
	rrc	a
	mov	r2,a
	mov	a,r3
	rrc	a
	jnc	__fsadd_align_nc
	orl	a,#1
__fsadd_align_nc:
	mov	r3,a
	djnz	dpl,__fsadd_align_bit
__fsadd_align_done:
	; the low 5 bits are below the working precision of the C version: fold them into a sticky bit
	mov	a,r3
	anl	a,#0x1f
	add	a,#0xff
	mov	a,r3
	anl	a,#0xe0
	orl	c,acc.5
	mov	acc.5,c
	mov	r3,a
__fsadd_aligned:
	jb	F0,__fsadd_sub
	mov	a,r7
	add	a,r3
	mov	r7,a
	mov	a,r6
	addc	a,r2
	mov	r6,a
	mov	a,r5
	addc	a,r1
	mov	r5,a
	mov	a,r4
	addc	a,r0
	mov	r4,a
	mov	r3,dph
	mov	r2,#0
	jnc	__fsadd_pack
	; carry out: shift right one place
	mov	a,r4
	rrc	a
	mov	r4,a
	mov	a,r5
	rrc	a
	mov	r5,a
	mov	a,r6
	rrc	a
	mov	r6,a
	mov	a,r7
	rrc	a
	mov	r7,a
	inc	r3
	jmp	__fsr_pack
__fsadd_sub:
	clr	c
	mov	a,r7
	subb	a,r3
	mov	r7,a
	mov	a,r6
	subb	a,r2
	mov	r6,a
	mov	a,r5
	subb	a,r1
	mov	r5,a
	mov	a,r4
	subb	a,r0
	mov	r4,a
	mov	r3,dph
	mov	r2,#0
__fsadd_pack:
	jmp	__fsr_pack

;;; module fsmul fast
; r7..r4 * r3..r0 -> r7..r4
__fsmul:
	mov	a,r4
	xrl	a,r0
	anl	a,#0x80
	push	acc
	mov	a,r5
	rlc	a
	mov	a,r4
	rlc	a
	mov	dpl,a
	mov	a,r1
	rlc	a
	mov	a,r0
	rlc	a
	mov	dph,a
	cjne	a,#0xff,__fsmul_b_fin
	sjmp	__fsmul_special
__fsmul_b_fin:
	mov	a,dpl
	cjne	a,#0xff,__fsmul_finite
__fsmul_special:
	; NaN operand: NaN; infinity times zero: NaN; otherwise infinity
	mov	a,dpl
	cjne	a,#0xff,__fsmul_a_num
	mov	a,r5
	anl	a,#0x7f
	orl	a,r6
	orl	a,r7
	jnz	__fsmul_nan
__fsmul_a_num:
	mov	a,dph
	cjne	a,#0xff,__fsmul_b_num
	mov	a,r1
	anl	a,#0x7f
	orl	a,r2
	orl	a,r3
	jnz	__fsmul_nan
__fsmul_b_num:
	mov	a,r4
	anl	a,#0x7f
	orl	a,r5
	orl	a,r6
	orl	a,r7
	jz	__fsmul_nan
	mov	a,r0
	anl	a,#0x7f
	orl	a,r1
	orl	a,r2
	orl	a,r3
	jz	__fsmul_nan
	pop	b
	jmp	__fsr_inf
__fsmul_nan:
	pop	acc
	jmp	__fsr_qnan
__fsmul_zero:
	pop	b
	jmp	__fsr_zero
__fsmul_finite:
	mov	a,r4
	anl	a,#0x7f
	orl	a,r5
	orl	a,r6
	orl	a,r7
	jz	__fsmul_zero
	mov	a,r0
	anl	a,#0x7f
	orl	a,r1
	orl	a,r2
	orl	a,r3
	jz	__fsmul_zero
	; E = ea + eb - 126 (the product >> 16 has value 2^(ea + eb - 284))
	mov	a,dpl
	add	a,dph
	mov	dpl,a
	clr	a
	rlc	a
	mov	dph,a
	mov	a,dpl
	add	a,#0x82
	push	acc
	mov	a,dph
	addc	a,#0xff
	push	acc
	mov	a,r5
	orl	a,#0x80
	mov	r5,a
	mov	a,r1
	orl	a,#0x80
	mov	r1,a
	; 24x24 product (a2 a1 a0 = r5 r6 r7, b2 b1 b0 = r1 r2 r3) by columns; accumulator r0, r4, dpl
	mov	a,r7
	mov	b,r3
	mul	ab
	mov	r0,b
	; column 1
	mov	a,r7
	mov	b,r2
	mul	ab
	add	a,r0
	mov	r0,a
	clr	a
	addc	a,b
	mov	r4,a
	mov	a,r6
	mov	b,r3
	mul	ab
	add	a,r0
	mov	a,b
	addc	a,r4
	mov	r0,a
	clr	a
	rlc	a
	mov	r4,a
	; column 2
	mov	a,r7
	mov	b,r1
	mul	ab
	add	a,r0
	mov	r0,a
	mov	a,b
	addc	a,r4
	mov	r4,a
	clr	a
	rlc	a
	mov	dpl,a
	mov	a,r6
	mov	b,r2
	mul	ab
	add	a,r0
	mov	r0,a
	mov	a,b
	addc	a,r4
	mov	r4,a
	clr	a
	addc	a,dpl
	mov	dpl,a
	mov	a,r5
	mov	b,r3
	mul	ab
	add	a,r0
	mov	dph,a
	mov	a,b
	addc	a,r4
	mov	r0,a
	clr	a
	addc	a,dpl
	mov	r4,a
	; column 3
	mov	a,r6
	mov	b,r1
	mul	ab
	add	a,r0
	mov	r0,a
	mov	a,b
	addc	a,r4
	mov	r4,a
	clr	a
	rlc	a
	mov	dpl,a
	mov	a,r5
	mov	b,r2
	mul	ab
	add	a,r0
	mov	r3,a
	mov	a,b
	addc	a,r4
	mov	r0,a
	clr	a
	addc	a,dpl
	mov	r4,a
	; column 4
	mov	a,r5
	mov	b,r1
	mul	ab
	add	a,r0
	mov	r5,a
	mov	a,b
	addc	a,r4
	mov	r4,a
	mov	a,r3
	mov	r6,a
	mov	r7,dph
	pop	acc
	mov	r2,a
	pop	acc
	mov	r3,a
	pop	b
	jmp	__fsr_pack

;;; module fsdiv fast
; r7..r4 / r3..r0 -> r7..r4
__fsdiv:
	mov	a,r4
	xrl	a,r0
	anl	a,#0x80
	push	acc
	mov	a,r5
	rlc	a
	mov	a,r4
	rlc	a
	mov	dpl,a
	mov	a,r1
	rlc	a
	mov	a,r0
	rlc	a
	mov	dph,a
	mov	a,dpl
	cjne	a,#0xff,__fsdiv_a_fin
	; a is NaN or infinite: NaN unless a is infinite and b finite
	mov	a,r5
	anl	a,#0x7f
	orl	a,r6
	orl	a,r7
	jnz	__fsdiv_nan
	mov	a,dph
	cjne	a,#0xff,__fsdiv_inf
	sjmp	__fsdiv_nan
__fsdiv_a_fin:
	mov	a,dph
	cjne	a,#0xff,__fsdiv_b_fin
	mov	a,r1
	anl	a,#0x7f
	orl	a,r2
	orl	a,r3
	jnz	__fsdiv_nan
	sjmp	__fsdiv_zero
__fsdiv_b_fin:
	mov	a,r0
	anl	a,#0x7f
	orl	a,r1
	orl	a,r2
	orl	a,r3
	jnz	__fsdiv_b_nz
	mov	a,r4
	anl	a,#0x7f
	orl	a,r5
	orl	a,r6
	orl	a,r7
	jz	__fsdiv_nan
__fsdiv_inf:
	pop	b
	jmp	__fsr_inf
__fsdiv_nan:
	pop	acc
	jmp	__fsr_qnan
__fsdiv_zero:
	pop	b
	jmp	__fsr_zero
__fsdiv_b_nz:
	mov	a,r4
	anl	a,#0x7f
	orl	a,r5
	orl	a,r6
	orl	a,r7
	jz	__fsdiv_zero
	; E = ea - eb + 133 (26 quotient bits)
	mov	a,dpl
	clr	c
	subb	a,dph
	mov	dpl,a
	clr	a
	subb	a,#0
	mov	dph,a
	mov	a,dpl
	add	a,#133
	push	acc
	clr	a
	addc	a,dph
	push	acc
	mov	a,r5
	orl	a,#0x80
	mov	r5,a
	mov	a,r1
	orl	a,#0x80
	mov	r1,a
	; restoring division: remainder r5 r6 r7 (bit 24 in F0), divisor r1 r2 r3; 26 quotient bits in 4 groups
	clr	F0
	mov	b,#2
	call	__fsdiv_bits
	mov	a,r0
	push	acc
	call	__fsdiv_bits8
	mov	a,r0
	push	acc
	call	__fsdiv_bits8
	mov	a,r0
	push	acc
	call	__fsdiv_bits8
	mov	a,r0
	mov	r7,a
	pop	acc
	mov	r6,a
	pop	acc
	mov	r5,a
	pop	acc
	mov	r4,a
	pop	acc
	mov	r2,a
	pop	acc
	mov	r3,a
	pop	b
	jmp	__fsr_pack
; B quotient bits into r0 (trial difference in r4, dpl)
__fsdiv_bits8:
	mov	b,#8
__fsdiv_bits:
	mov	r0,#0
__fsdiv_loop:
	mov	a,r7
	clr	c
	subb	a,r3
	mov	r4,a
	mov	a,r6
	subb	a,r2
	mov	dpl,a
	mov	a,r5
	subb	a,r1
	jb	F0,__fsdiv_take
	jc	__fsdiv_shift
__fsdiv_take:
	mov	r5,a
	mov	r6,dpl
	mov	a,r4
	mov	r7,a
	clr	c
__fsdiv_shift:
	cpl	c
	mov	a,r0
	rlc	a
	mov	r0,a
	mov	a,r7
	add	a,r7
	mov	r7,a
	mov	a,r6
	rlc	a
	mov	r6,a
	mov	a,r5
	rlc	a
	mov	r5,a
	mov	F0,c
	djnz	b,__fsdiv_loop
	ret

;;; module fscmp
; a (r7..r4) < b (r3..r0) -> A; a == b -> A
__fslt:
	call	__fsr_nan2
	jc	__fscmp_false
	mov	a,r4
	orl	a,r0
	anl	a,#0x7f
	orl	a,r5
	orl	a,r6
	orl	a,r7
	orl	a,r1
	orl	a,r2
	orl	a,r3
	jz	__fscmp_ret
	mov	a,r4
	xrl	a,r0
	jnb	acc.7,__fslt_same
	mov	a,r4
	rl	a
	anl	a,#1
	ret
__fslt_same:
	clr	c
	mov	a,r7
	subb	a,r3
	mov	b,a
	mov	a,r6
	subb	a,r2
	orl	b,a
	mov	a,r5
	subb	a,r1
	orl	b,a
	mov	a,r4
	subb	a,r0
	orl	a,b
	jz	__fscmp_ret
	; (|a| < |b|) xor (a negative)
	mov	a,r4
	rl	a
	addc	a,#0
	anl	a,#1
	ret
__fseq:
	call	__fsr_nan2
	jc	__fscmp_false
	mov	a,r7
	xrl	a,r3
	mov	b,a
	mov	a,r6
	xrl	a,r2
	orl	b,a
	mov	a,r5
	xrl	a,r1
	orl	b,a
	mov	a,r4
	xrl	a,r0
	orl	a,b
	jz	__fscmp_true
	; +0 == -0
	mov	a,r4
	orl	a,r0
	anl	a,#0x7f
	orl	a,r5
	orl	a,r6
	orl	a,r7
	orl	a,r1
	orl	a,r2
	orl	a,r3
	jz	__fscmp_true
__fscmp_false:
	clr	a
__fscmp_ret:
	ret
__fscmp_true:
	mov	a,#1
	ret
; C = a or b is NaN
__fsr_nan2:
	mov	a,r4
	anl	a,#0x7f
	mov	b,a
	clr	c
	clr	a
	subb	a,r7
	clr	a
	subb	a,r6
	mov	a,#0x80
	subb	a,r5
	mov	a,#0x7f
	subb	a,b
	jc	__fsr_nan2_ret
	mov	a,r0
	anl	a,#0x7f
	mov	b,a
	clr	c
	clr	a
	subb	a,r3
	clr	a
	subb	a,r2
	mov	a,#0x80
	subb	a,r1
	mov	a,#0x7f
	subb	a,b
__fsr_nan2_ret:
	ret

;;; module fs2l fast
; float r7..r4 -> (unsigned) long r7..r4
__fs2sl:
	mov	a,r4
	anl	a,#0x80
	push	acc
	xrl	a,r4
	mov	r4,a
	call	__fs2ul
	pop	acc
	jnb	acc.7,__fs2l_ret
	clr	c
	clr	a
	subb	a,r7
	mov	r7,a
	clr	a
	subb	a,r6
	mov	r6,a
	clr	a
	subb	a,r5
	mov	r5,a
	clr	a
	subb	a,r4
	mov	r4,a
	ret
__fs2ul:
	mov	a,r4
	jb	acc.7,__fs2l_zero
	mov	a,r5
	rlc	a
	mov	a,r4
	rlc	a
	add	a,#0x81
	jnc	__fs2l_zero
	; A = unbiased exponent e >= 0
	mov	b,a
	add	a,#0xe0
	jc	__fs2l_max
	mov	a,#31
	clr	c
	subb	a,b
	mov	b,a
	; (mantissa << 8) >> (31 - e)
	mov	a,r5
	orl	a,#0x80
	mov	r4,a
	mov	a,r6
	mov	r5,a
	mov	a,r7
	mov	r6,a
	mov	r7,#0
__fs2l_bytes:
	mov	a,b
	add	a,#0xf8
	jnc	__fs2l_bits
	mov	b,a
	mov	a,r6
	mov	r7,a
	mov	a,r5
	mov	r6,a
	mov	a,r4
	mov	r5,a
	mov	r4,#0
	sjmp	__fs2l_bytes
__fs2l_bits:
	mov	a,b
	jz	__fs2l_ret
__fs2l_bit:
	clr	c
	mov	a,r4
	rrc	a
	mov	r4,a
	mov	a,r5
	rrc	a
	mov	r5,a
	mov	a,r6
	rrc	a
	mov	r6,a
	mov	a,r7
	rrc	a
	mov	r7,a
	djnz	b,__fs2l_bit
__fs2l_ret:
	ret
__fs2l_zero:
	clr	a
	sjmp	__fs2l_fill
__fs2l_max:
	mov	a,#0xff
__fs2l_fill:
	mov	r4,a
	mov	r5,a
	mov	r6,a
	mov	r7,a
	ret

;;; module l2fs
; (unsigned) long r7..r4 -> float r7..r4
__sl2fs:
	mov	a,r4
	anl	a,#0x80
	mov	b,a
	jz	__l2fs_pack
	clr	c
	clr	a
	subb	a,r7
	mov	r7,a
	clr	a
	subb	a,r6
	mov	r6,a
	clr	a
	subb	a,r5
	mov	r5,a
	clr	a
	subb	a,r4
	mov	r4,a
	sjmp	__l2fs_pack
__ul2fs:
	mov	b,#0
__l2fs_pack:
	mov	r3,#158
	mov	r2,#0
	jmp	__fsr_pack

;;; module fsr_pack small
; See the fast version.
__fsr_pack:
	mov	a,r4
	orl	a,r5
	orl	a,r6
	orl	a,r7
	jz	__fss_pack_zero
__fss_pack_norm:
	mov	a,r4
	jb	acc.7,__fss_pack_round
	mov	a,r7
	add	a,r7
	mov	r7,a
	mov	a,r6
	rlc	a
	mov	r6,a
	mov	a,r5
	rlc	a
	mov	r5,a
	mov	a,r4
	rlc	a
	mov	r4,a
	cjne	r3,#0,__fss_pack_dec
	dec	r2
__fss_pack_dec:
	dec	r3
	sjmp	__fss_pack_norm
__fss_pack_round:
	jmp	__fsr_round
__fss_pack_zero:
	jmp	__fsr_zero

;;; module fsadd small
; See the fast version.
__fssub:
	mov	a,r0
	xrl	a,#0x80
	mov	r0,a
__fsadd:
	mov	a,r7
	clr	c
	subb	a,r3
	mov	a,r6
	subb	a,r2
	mov	a,r5
	subb	a,r1
	mov	a,r0
	anl	a,#0x7f
	mov	b,a
	mov	a,r4
	anl	a,#0x7f
	subb	a,b
	jnc	__fss_add_ordered
	mov	a,r7
	xch	a,r3
	mov	r7,a
	mov	a,r6
	xch	a,r2
	mov	r6,a
	mov	a,r5
	xch	a,r1
	mov	r5,a
	mov	a,r4
	xch	a,r0
	mov	r4,a
__fss_add_ordered:
	mov	a,r5
	rlc	a
	mov	a,r4
	rlc	a
	cjne	a,#0xff,__fss_add_finite
	mov	a,r5
	anl	a,#0x7f
	orl	a,r6
	orl	a,r7
	jnz	__fss_add_nan
	mov	a,r4
	xrl	a,r0
	jnb	acc.7,__fss_add_ret
	mov	a,r0
	orl	a,#0x80
	cjne	a,#0xff,__fss_add_ret
	cjne	r1,#0x80,__fss_add_ret
	mov	a,r2
	orl	a,r3
	jnz	__fss_add_ret
__fss_add_nan:
	jmp	__fsr_qnan
__fss_add_finite:
	mov	dph,a
	mov	a,r0
	anl	a,#0x7f
	orl	a,r1
	orl	a,r2
	orl	a,r3
	jnz	__fss_add_nonzero
	mov	a,r4
	anl	a,#0x7f
	orl	a,r5
	orl	a,r6
	orl	a,r7
	jnz	__fss_add_ret
	mov	a,r0
	mov	r4,a
__fss_add_ret:
	ret
__fss_add_nonzero:
	mov	a,r1
	rlc	a
	mov	a,r0
	rlc	a
	xch	a,dph
	mov	dpl,a
	clr	c
	subb	a,dph
	mov	dph,dpl
	mov	dpl,a
	mov	a,r4
	anl	a,#0x80
	mov	b,a
	xrl	a,r0
	mov	c,acc.7
	mov	F0,c
	mov	a,r5
	orl	a,#0x80
	mov	r4,a
	mov	a,r6
	mov	r5,a
	mov	a,r7
	mov	r6,a
	mov	r7,#0
	mov	a,r1
	orl	a,#0x80
	mov	r0,a
	mov	a,r2
	mov	r1,a
	mov	a,r3
	mov	r2,a
	mov	r3,#0
	; align b one bit at a time (at most 26 places); bits shifted out collect in r3.0
	mov	a,dpl
	jz	__fss_add_aligned
	add	a,#0xe6
	jnc	__fss_add_align
	mov	dpl,#26
__fss_add_align:
	clr	c
	mov	a,r0
	rrc	a
	mov	r0,a
	mov	a,r1
	rrc	a
	mov	r1,a
	mov	a,r2
	rrc	a
	mov	r2,a
	mov	a,r3
	rrc	a
	jnc	__fss_add_align_nc
	orl	a,#1
__fss_add_align_nc:
	mov	r3,a
	djnz	dpl,__fss_add_align
	mov	a,r3
	anl	a,#0x1f
	add	a,#0xff
	mov	a,r3
	anl	a,#0xe0
	orl	c,acc.5
	mov	acc.5,c
	mov	r3,a
__fss_add_aligned:
	jb	F0,__fss_add_sub
	mov	a,r7
	add	a,r3
	mov	r7,a
	mov	a,r6
	addc	a,r2
	mov	r6,a
	mov	a,r5
	addc	a,r1
	mov	r5,a
	mov	a,r4
	addc	a,r0
	mov	r4,a
	mov	r3,dph
	mov	r2,#0
	jnc	__fss_add_pack
	mov	a,r4
	rrc	a
	mov	r4,a
	mov	a,r5
	rrc	a
	mov	r5,a
	mov	a,r6
	rrc	a
	mov	r6,a
	mov	a,r7
	rrc	a
	mov	r7,a
	inc	r3
	sjmp	__fss_add_pack
__fss_add_sub:
	clr	c
	mov	a,r7
	subb	a,r3
	mov	r7,a
	mov	a,r6
	subb	a,r2
	mov	r6,a
	mov	a,r5
	subb	a,r1
	mov	r5,a
	mov	a,r4
	subb	a,r0
	mov	r4,a
	mov	r3,dph
	mov	r2,#0
__fss_add_pack:
	jmp	__fsr_pack

;;; module fss_prep small
; Common start of __fsmul and __fsdiv. DPTR points to a table giving the result for special operands:
; one byte per class of a, two bits per class of b (class: 0 zero, 1 finite, 2 infinite, 3 NaN;
; result: 0 compute, 1 zero, 2 infinity, 3 NaN). Special results return straight to the caller's caller.
; Otherwise returns B = result sign, r4 = exponent of a, r0 = exponent of b, implicit bits set in r5 and r1.
__fss_prep:
	mov	a,r4
	xrl	a,r0
	anl	a,#0x80
	mov	b,a
	mov	a,r1
	anl	a,#0x7f
	orl	a,r2
	orl	a,r3
	add	a,#0xff
	mov	F0,c
	mov	a,r1
	rlc	a
	mov	a,r0
	rlc	a
	mov	r0,a
	call	__fss_class
	rl	a
	push	acc
	mov	a,r5
	anl	a,#0x7f
	orl	a,r6
	orl	a,r7
	add	a,#0xff
	mov	F0,c
	mov	a,r5
	rlc	a
	mov	a,r4
	rlc	a
	mov	r4,a
	call	__fss_class
	movc	a,@a+dptr
	pop	dpl
	inc	dpl
	sjmp	__fss_prep_next
__fss_prep_shift:
	rr	a
__fss_prep_next:
	djnz	dpl,__fss_prep_shift
	anl	a,#3
	xch	a,r5
	orl	a,#0x80
	xch	a,r5
	xch	a,r1
	orl	a,#0x80
	xch	a,r1
	jz	__fss_prep_ret
	dec	sp
	dec	sp
	dec	a
	jz	__fss_prep_zero
	dec	a
	jz	__fss_prep_inf
	jmp	__fsr_qnan
__fss_prep_zero:
	jmp	__fsr_zero
__fss_prep_inf:
	jmp	__fsr_inf
__fss_prep_ret:
	ret
; pop the exponent and sign pushed by __fsmul / __fsdiv (r7..r4 = mantissa) and pack
__fss_muldiv_pack:
	pop	acc
	mov	r2,a
	pop	acc
	mov	r3,a
	pop	b
	jmp	__fsr_pack
; A = exponent, F0 = mantissa is nonzero -> A = class
__fss_class:
	jz	__fss_class_m
	inc	a
	jz	__fss_class_x
	mov	a,#1
	ret
__fss_class_x:
	mov	a,#2
__fss_class_m:
	mov	c,F0
	addc	a,#0
	ret

;;; module fsmul small
; See the fast version.
__fsmul:
	mov	dptr,#__fss_multab
	call	__fss_prep
	push	b
	; E = ea + eb - 126
	mov	a,r4
	add	a,r0
	mov	r4,a
	clr	a
	rlc	a
	xch	a,r4
	add	a,#0x82
	push	acc
	mov	a,r4
	addc	a,#0xff
	push	acc
	; shift-and-add: product in r0 r4 dpl (high) : r1 r2 r3 (low, initially b)
	clr	a
	mov	r0,a
	mov	r4,a
	mov	dpl,a
	mov	b,#24
__fss_mul_loop:
	mov	a,r3
	rrc	a
	jnc	__fss_mul_shift
	mov	a,dpl
	add	a,r7
	mov	dpl,a
	mov	a,r4
	addc	a,r6
	mov	r4,a
	mov	a,r0
	addc	a,r5
	mov	r0,a
__fss_mul_shift:
	mov	a,r0
	rrc	a
	mov	r0,a
	mov	a,r4
	rrc	a
	mov	r4,a
	mov	a,dpl
	rrc	a
	mov	dpl,a
	mov	a,r1
	rrc	a
	mov	r1,a
	mov	a,r2
	rrc	a
	mov	r2,a
	mov	a,r3
	rrc	a
	mov	r3,a
	djnz	b,__fss_mul_loop
	; product >> 16
	mov	a,r1
	mov	r7,a
	mov	r6,dpl
	mov	a,r4
	mov	r5,a
	mov	a,r0
	mov	r4,a
	jmp	__fss_muldiv_pack
__fss_multab:
	.db	0xf5, 0xe1, 0xeb, 0xff

;;; module fsdiv small
; See the fast version.
__fsdiv:
	mov	dptr,#__fss_divtab
	call	__fss_prep
	push	b
	; E = ea - eb + 133
	mov	a,r4
	clr	c
	subb	a,r0
	mov	r4,a
	subb	a,r4
	mov	r0,a
	mov	a,r4
	add	a,#133
	push	acc
	clr	a
	addc	a,r0
	push	acc
	; restoring division: remainder r5 r6 r7 (bit 24 in F0), divisor r1 r2 r3, quotient dph dpl r4 r0
	clr	a
	mov	r0,a
	mov	r4,a
	mov	dpl,a
	mov	dph,a
	clr	F0
	mov	b,#26
__fss_div_loop:
	mov	a,r7
	clr	c
	subb	a,r3
	mov	r7,a
	mov	a,r6
	subb	a,r2
	mov	r6,a
	mov	a,r5
	subb	a,r1
	mov	r5,a
	jb	F0,__fss_div_take
	jnc	__fss_div_take
	mov	a,r7
	add	a,r3
	mov	r7,a
	mov	a,r6
	addc	a,r2
	mov	r6,a
	mov	a,r5
	addc	a,r1
	mov	r5,a
	clr	c
	sjmp	__fss_div_shift
__fss_div_take:
	setb	c
__fss_div_shift:
	mov	a,r0
	rlc	a
	mov	r0,a
	mov	a,r4
	rlc	a
	mov	r4,a
	mov	a,dpl
	rlc	a
	mov	dpl,a
	mov	a,dph
	rlc	a
	mov	dph,a
	mov	a,r7
	add	a,r7
	mov	r7,a
	mov	a,r6
	rlc	a
	mov	r6,a
	mov	a,r5
	rlc	a
	mov	r5,a
	mov	F0,c
	djnz	b,__fss_div_loop
	mov	a,r0
	mov	r7,a
	mov	a,r4
	mov	r6,a
	mov	r5,dpl
	mov	r4,dph
	jmp	__fss_muldiv_pack
__fss_divtab:
	.db	0xd7, 0xd2, 0xfa, 0xff

;;; module fs2l small
; See the fast version.
__fs2sl:
	mov	a,r4
	anl	a,#0x80
	push	acc
	xrl	a,r4
	mov	r4,a
	call	__fs2ul
	pop	acc
	jnb	acc.7,__fss_f2l_ret
	clr	c
	clr	a
	subb	a,r7
	mov	r7,a
	clr	a
	subb	a,r6
	mov	r6,a
	clr	a
	subb	a,r5
	mov	r5,a
	clr	a
	subb	a,r4
	mov	r4,a
	ret
__fs2ul:
	mov	a,r4
	jb	acc.7,__fss_f2l_zero
	mov	a,r5
	rlc	a
	mov	a,r4
	rlc	a
	add	a,#0x81
	jnc	__fss_f2l_zero
	mov	b,a
	add	a,#0xe0
	jc	__fss_f2l_max
	mov	a,#31
	clr	c
	subb	a,b
	mov	b,a
	mov	a,r5
	orl	a,#0x80
	mov	r4,a
	mov	a,r6
	mov	r5,a
	mov	a,r7
	mov	r6,a
	mov	r7,#0
	inc	b
	sjmp	__fss_f2l_next
__fss_f2l_bit:
	clr	c
	mov	a,r4
	rrc	a
	mov	r4,a
	mov	a,r5
	rrc	a
	mov	r5,a
	mov	a,r6
	rrc	a
	mov	r6,a
	mov	a,r7
	rrc	a
	mov	r7,a
__fss_f2l_next:
	djnz	b,__fss_f2l_bit
__fss_f2l_ret:
	ret
__fss_f2l_zero:
	clr	a
	sjmp	__fss_f2l_fill
__fss_f2l_max:
	mov	a,#0xff
__fss_f2l_fill:
	mov	r4,a
	mov	r5,a
	mov	r6,a
	mov	r7,a
	ret

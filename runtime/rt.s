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

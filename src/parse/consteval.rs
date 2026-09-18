//! Constant folding and constant-expression evaluation.

use super::*;

#[derive(Clone, Debug, PartialEq)]
pub enum ConstVal {
    Int(i64),
    Float(f64),
    Addr(RelocTarget, i64),
}

pub fn bits_of(ty: &Type) -> u32 {
    match ty.int_kind() {
        Some((k, _)) => int_bits(k),
        None => 16,
    }
}

/// Normalize a value to the representation of an integer type (sign- or zero-extended to i64).
pub fn norm_int(v: i64, ty: &Type) -> i64 {
    if ty.is_bool() {
        return (v != 0) as i64;
    }
    let bits = if ty.is_pointer() { 16 } else { bits_of(ty) };
    let signed = ty.is_signed();
    norm_bits(v, bits, signed)
}

pub fn norm_bits(v: i64, bits: u32, signed: bool) -> i64 {
    if bits >= 64 {
        return v;
    }
    let mask = (1u64 << bits) - 1;
    let u = (v as u64) & mask;
    if signed && (u >> (bits - 1)) & 1 == 1 {
        (u | !mask) as i64
    } else {
        u as i64
    }
}

pub fn fold_cast(v: i64, _from: &Type, to: &Type) -> i64 {
    norm_int(v, to)
}

pub fn fold_unary(op: UnOp, v: i64, ty: &Type) -> i64 {
    match op {
        UnOp::Neg => norm_int(v.wrapping_neg(), ty),
        UnOp::BitNot => norm_int(!v, ty),
        UnOp::LogNot => (v == 0) as i64,
    }
}

/// Fold a binary operation whose operands have type `ty` (for comparisons: the operand type).
pub fn fold_binary(op: BinOp, a: i64, b: i64, ty: &Type) -> Option<i64> {
    let signed = ty.is_signed();
    let bits = bits_of(ty);
    let (ua, ub) = (norm_bits(a, bits, false) as u64, norm_bits(b, bits, false) as u64);
    let r = match op {
        BinOp::Add => a.wrapping_add(b),
        BinOp::Sub => a.wrapping_sub(b),
        BinOp::Mul => a.wrapping_mul(b),
        BinOp::Div => {
            if b == 0 {
                return None;
            }
            if signed { a.wrapping_div(b) } else { (ua / ub) as i64 }
        }
        BinOp::Mod => {
            if b == 0 {
                return None;
            }
            if signed { a.wrapping_rem(b) } else { (ua % ub) as i64 }
        }
        BinOp::Shl => {
            if !(0..64).contains(&b) {
                0
            } else {
                a.wrapping_shl(b as u32)
            }
        }
        BinOp::Shr => {
            if !(0..64).contains(&b) {
                if signed && a < 0 { -1 } else { 0 }
            } else if signed {
                a >> b
            } else {
                (ua >> b) as i64
            }
        }
        BinOp::And => a & b,
        BinOp::Or => a | b,
        BinOp::Xor => a ^ b,
        BinOp::Eq => return Some((a == b) as i64),
        BinOp::Ne => return Some((a != b) as i64),
        BinOp::Lt => return Some(if signed { a < b } else { ua < ub } as i64),
        BinOp::Le => return Some(if signed { a <= b } else { ua <= ub } as i64),
        BinOp::Gt => return Some(if signed { a > b } else { ua > ub } as i64),
        BinOp::Ge => return Some(if signed { a >= b } else { ua >= ub } as i64),
        BinOp::LogAnd => return Some((a != 0 && b != 0) as i64),
        BinOp::LogOr => return Some((a != 0 || b != 0) as i64),
    };
    Some(norm_bits(r, bits, signed))
}

impl<'a> Parser<'a> {
    pub(super) fn eval_const(&self, e: &Expr) -> Option<ConstVal> {
        eval_const(self.prog, e)
    }

    pub(super) fn eval_static_init(&mut self, ty: &Type, items: &[super::init::InitItem], loc: Loc) -> Result<InitData> {
        let size = self.prog.sizeof(ty).ok_or_else(|| error(loc, "initializer for object of incomplete type"))?;
        let mut data = InitData { bytes: vec![0; size as usize], relocs: vec![] };
        for it in items {
            match &it.kind {
                super::init::InitKind::Bytes(b) => {
                    let off = it.offset as usize;
                    if off + b.len() > data.bytes.len() && off >= size as usize {
                        data.bytes.resize(off + b.len(), 0);
                    }
                    for (i, x) in b.iter().enumerate() {
                        if off + i < data.bytes.len() {
                            data.bytes[off + i] = *x;
                        }
                    }
                }
                super::init::InitKind::Expr(e) => {
                    let esize = if e.ty.is_bit() { 1 } else { self.prog.size(&e.ty) } as usize;
                    let off = it.offset as usize;
                    if off + esize > data.bytes.len() {
                        // Flexible array member initializer.
                        data.bytes.resize(off + esize, 0);
                    }
                    if e.ty.is_record() {
                        // Copy of a constant aggregate: only compound literals / globals with init.
                        let src = match &e.kind {
                            ExprKind::Global(g) => self.prog.globals[*g].init.clone(),
                            ExprKind::StmtExpr(..) => None,
                            _ => None,
                        };
                        match src {
                            Some(d) => {
                                data.bytes[off..off + esize].copy_from_slice(&d.bytes[..esize]);
                                for r in d.relocs {
                                    data.relocs.push(Reloc { offset: r.offset + off as u32, ..r });
                                }
                            }
                            None => return err(e.loc, "initializer element is not a compile-time constant"),
                        }
                        continue;
                    }
                    let v = self.eval_const(e).ok_or_else(|| error(e.loc, "initializer element is not a compile-time constant"))?;
                    match v {
                        ConstVal::Int(v) => {
                            if let Some((bo, w)) = it.bits {
                                let mask: u64 = if w >= 64 { !0 } else { (1u64 << w) - 1 };
                                let mut cur: u64 = 0;
                                let nbytes = ((bo as usize + w as usize) + 7) / 8;
                                for i in 0..nbytes.min(8) {
                                    cur |= (data.bytes[off + i] as u64) << (8 * i);
                                }
                                cur &= !(mask << bo);
                                cur |= ((v as u64) & mask) << bo;
                                for i in 0..nbytes.min(8) {
                                    data.bytes[off + i] = (cur >> (8 * i)) as u8;
                                }
                            } else {
                                for i in 0..esize {
                                    data.bytes[off + i] = ((v as u64) >> (8 * i.min(7))) as u8;
                                    if i >= 8 {
                                        data.bytes[off + i] = if v < 0 { 0xff } else { 0 };
                                    }
                                }
                            }
                        }
                        ConstVal::Float(f) => {
                            if e.ty.is_float() {
                                let b = (f as f32).to_le_bytes();
                                data.bytes[off..off + 4].copy_from_slice(&b);
                            } else {
                                let v = f as i64;
                                for i in 0..esize {
                                    data.bytes[off + i] = (v >> (8 * i)) as u8;
                                }
                            }
                        }
                        ConstVal::Addr(t, a) => {
                            if esize < 2 {
                                // Low byte of an address.
                                data.relocs.push(Reloc { offset: off as u32, target: t, addend: a, size: 1, space_var: None });
                            } else {
                                data.relocs.push(Reloc {
                                    offset: off as u32,
                                    target: t,
                                    addend: a,
                                    size: esize.min(3) as u8,
                                    space_var: e.ty.space_var(),
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(data)
    }
}

/// The address space of the object whose address `e` computes, if it is known.
fn addr_space(prog: &Program, e: &Expr) -> Option<Space> {
    fn object_space(prog: &Program, e: &Expr) -> Option<Space> {
        match &e.kind {
            ExprKind::Global(g) => Some(match prog.globals[*g].space {
                Space::Sfr | Space::Sbit | Space::Bit => Space::Data,
                s => s,
            }),
            ExprKind::Member(b, _) => object_space(prog, b),
            _ => None,
        }
    }
    match &e.kind {
        ExprKind::AddrOf(i) => object_space(prog, i),
        ExprKind::Cast(i) => addr_space(prog, i),
        _ => None,
    }
}

/// Address of an lvalue as a constant.
fn eval_addr(prog: &Program, e: &Expr) -> Option<ConstVal> {
    match &e.kind {
        ExprKind::Global(_) => Some(ConstVal::Addr(RelocTarget::Global(*match &e.kind {
            ExprKind::Global(g) => g,
            _ => unreachable!(),
        }), 0)),
        ExprKind::Func(f) => Some(ConstVal::Addr(RelocTarget::Func(*f), 0)),
        ExprKind::Member(b, fi) => {
            let TypeKind::Record(r) = b.ty.kind else { return None };
            let off = prog.records[r].fields[*fi].offset as i64;
            match eval_addr(prog, b)? {
                ConstVal::Addr(t, a) => Some(ConstVal::Addr(t, a + off)),
                ConstVal::Int(v) => Some(ConstVal::Int(v + off)),
                _ => None,
            }
        }
        ExprKind::Deref(p) => eval_const(prog, p),
        _ => None,
    }
}

pub fn eval_const(prog: &Program, e: &Expr) -> Option<ConstVal> {
    match &e.kind {
        ExprKind::Int(v) => Some(ConstVal::Int(*v)),
        ExprKind::Float(f) => Some(ConstVal::Float(*f)),
        ExprKind::Func(f) => Some(ConstVal::Addr(RelocTarget::Func(*f), 0)),
        ExprKind::AddrOf(inner) => eval_addr(prog, inner),
        ExprKind::Cast(inner) => {
            let v = eval_const(prog, inner)?;
            if e.ty.is_void() {
                return None;
            }
            match v {
                ConstVal::Int(i) => {
                    if e.ty.is_float() {
                        let f = if inner.ty.is_signed() { i as f64 } else { i as u64 as f64 };
                        Some(ConstVal::Float(f))
                    } else if e.ty.is_pointer()
                        && !e.ty.is_func_ptr()
                        && prog.size(&e.ty) == 3
                        && i & 0xffff != 0
                        && (inner.ty.is_integer() || (inner.ty.is_pointer() && prog.size(&inner.ty) < 3))
                    {
                        // Non-null constant converted to a generic pointer: add the space tag.
                        let tag = if let Some(sp) = addr_space(prog, inner) {
                            Some(sp.gptr_tag())
                        } else if inner.ty.is_pointer() {
                            Some(prog.ptr_space(&inner.ty).map_or(0x40, |s| s.gptr_tag()))
                        } else {
                            // A plain integer keeps whatever it holds in the tag byte (SDCC).
                            e.ty.pointee().and_then(|p| p.q.space).map(|s| s.gptr_tag())
                        };
                        match tag {
                            Some(t) => Some(ConstVal::Int((i & 0xffff) | ((t as i64) << 16))),
                            None => Some(ConstVal::Int(norm_int(i, &e.ty))),
                        }
                    } else {
                        Some(ConstVal::Int(norm_int(i, &e.ty)))
                    }
                }
                ConstVal::Float(f) => {
                    if e.ty.is_float() {
                        Some(ConstVal::Float(f))
                    } else {
                        Some(ConstVal::Int(norm_int(f as i64, &e.ty)))
                    }
                }
                ConstVal::Addr(t, a) => {
                    if e.ty.is_bool() {
                        Some(ConstVal::Int(1))
                    } else if e.ty.is_pointer() || e.ty.is_integer() {
                        // A narrower integer keeps the low bytes of the address.
                        Some(ConstVal::Addr(t, a))
                    } else {
                        None
                    }
                }
            }
        }
        ExprKind::PtrAdd(p, i) => {
            let pv = eval_const(prog, p)?;
            let ConstVal::Int(iv) = eval_const(prog, i)? else { return None };
            match pv {
                ConstVal::Addr(t, a) => Some(ConstVal::Addr(t, a + iv)),
                ConstVal::Int(v) => Some(ConstVal::Int(norm_bits(v + iv, 16, false))),
                _ => None,
            }
        }
        ExprKind::PtrDiff(a, b, sz) => {
            let (av, bv) = (eval_const(prog, a)?, eval_const(prog, b)?);
            match (av, bv) {
                (ConstVal::Addr(t1, a1), ConstVal::Addr(t2, a2)) if t1 == t2 => Some(ConstVal::Int((a1 - a2) / *sz as i64)),
                (ConstVal::Int(x), ConstVal::Int(y)) => Some(ConstVal::Int((x - y) / *sz as i64)),
                _ => None,
            }
        }
        ExprKind::Unary(op, a) => match eval_const(prog, a)? {
            ConstVal::Int(v) => Some(ConstVal::Int(fold_unary(*op, v, &e.ty))),
            ConstVal::Float(f) => match op {
                UnOp::Neg => Some(ConstVal::Float(-f)),
                UnOp::LogNot => Some(ConstVal::Int((f == 0.0) as i64)),
                _ => None,
            },
            ConstVal::Addr(..) => match op {
                UnOp::LogNot => Some(ConstVal::Int(0)),
                _ => None,
            },
        },
        ExprKind::Binary(op, a, b) => {
            if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
                let av = truthy(eval_const(prog, a)?)?;
                if *op == BinOp::LogAnd && !av {
                    return Some(ConstVal::Int(0));
                }
                if *op == BinOp::LogOr && av {
                    return Some(ConstVal::Int(1));
                }
                let bv = truthy(eval_const(prog, b)?)?;
                return Some(ConstVal::Int(bv as i64));
            }
            let av = eval_const(prog, a)?;
            let bv = eval_const(prog, b)?;
            match (av, bv) {
                (ConstVal::Int(x), ConstVal::Int(y)) => {
                    let ty = if op.is_cmp() { &a.ty } else { &e.ty };
                    fold_binary(*op, x, y, ty).map(ConstVal::Int)
                }
                (ConstVal::Float(x), ConstVal::Float(y)) => Some(match op {
                    BinOp::Add => ConstVal::Float(x + y),
                    BinOp::Sub => ConstVal::Float(x - y),
                    BinOp::Mul => ConstVal::Float(x * y),
                    BinOp::Div => ConstVal::Float(x / y),
                    BinOp::Eq => ConstVal::Int((x == y) as i64),
                    BinOp::Ne => ConstVal::Int((x != y) as i64),
                    BinOp::Lt => ConstVal::Int((x < y) as i64),
                    BinOp::Le => ConstVal::Int((x <= y) as i64),
                    BinOp::Gt => ConstVal::Int((x > y) as i64),
                    BinOp::Ge => ConstVal::Int((x >= y) as i64),
                    _ => return None,
                }),
                (ConstVal::Addr(t, x), ConstVal::Int(y)) => match op {
                    BinOp::Add => Some(ConstVal::Addr(t, x + y)),
                    BinOp::Sub => Some(ConstVal::Addr(t, x - y)),
                    _ => None,
                },
                (ConstVal::Int(x), ConstVal::Addr(t, y)) if *op == BinOp::Add => Some(ConstVal::Addr(t, x + y)),
                (ConstVal::Addr(t1, x), ConstVal::Addr(t2, y)) if t1 == t2 => match op {
                    BinOp::Sub => Some(ConstVal::Int(x - y)),
                    BinOp::Eq => Some(ConstVal::Int((x == y) as i64)),
                    BinOp::Ne => Some(ConstVal::Int((x != y) as i64)),
                    _ => None,
                },
                _ => None,
            }
        }
        ExprKind::Cond(c, a, b) => {
            let cv = truthy(eval_const(prog, c)?)?;
            if cv { eval_const(prog, a) } else { eval_const(prog, b) }
        }
        ExprKind::Comma(_, b) => eval_const(prog, b),
        ExprKind::Global(g) => {
            // Array/function designators are handled via AddrOf; reading a variable is not constant.
            let _ = g;
            None
        }
        _ => None,
    }
}

fn truthy(v: ConstVal) -> Option<bool> {
    match v {
        ConstVal::Int(i) => Some(i != 0),
        ConstVal::Float(f) => Some(f != 0.0),
        ConstVal::Addr(..) => Some(true),
    }
}

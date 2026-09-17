//! Local instruction combining, constant folding and width narrowing.

use crate::ir::*;

pub struct Info {
    pub ssa: Vec<bool>,
    pub ndefs: Vec<u32>,
    pub def_at: Vec<(u32, u32)>,
    pub nuses: Vec<u32>,
}

pub fn info(f: &Func) -> Info {
    let n = f.vregs.len();
    let mut i = Info { ssa: super::dataflow::ssa_vregs(f), ndefs: vec![0; n], def_at: vec![(u32::MAX, 0); n], nuses: vec![0; n] };
    for (p, param) in f.params.iter().enumerate() {
        let _ = p;
        if let ParamLoc::Reg(r) = param {
            i.ndefs[*r as usize] += 1;
        }
    }
    for (bi, b) in f.blocks.iter().enumerate() {
        for (ii, ins) in b.insts.iter().enumerate() {
            if let Some(d) = ins.def() {
                i.ndefs[d as usize] += 1;
                i.def_at[d as usize] = (bi as u32, ii as u32);
            }
            for u in ins.uses() {
                i.nuses[u as usize] += 1;
            }
        }
        for u in b.term.uses() {
            i.nuses[u as usize] += 1;
        }
    }
    i
}

impl Info {
    pub fn single_def<'a>(&self, f: &'a Func, v: VReg) -> Option<&'a Inst> {
        if self.ndefs[v as usize] != 1 || !self.ssa[v as usize] {
            return None;
        }
        let (b, i) = self.def_at[v as usize];
        if b == u32::MAX {
            return None;
        }
        f.blocks[b as usize].insts.get(i as usize)
    }
}

fn is_pow2(k: i64) -> Option<u32> {
    if k > 0 && (k & (k - 1)) == 0 { Some(k.trailing_zeros()) } else { None }
}

fn eval_bin(op: BinK, a: i64, b: i64, ty: Ty) -> Option<i64> {
    let (ua, ub) = (ty.norm(a) as u64, ty.norm(b) as u64);
    let (sa, sb) = (ty.sext(a), ty.sext(b));
    let bits = ty.bits() as u64;
    let r: i64 = match op {
        BinK::Add => a.wrapping_add(b),
        BinK::Sub => a.wrapping_sub(b),
        BinK::Mul => a.wrapping_mul(b),
        BinK::DivU => {
            if ub == 0 {
                return None;
            }
            (ua / ub) as i64
        }
        BinK::ModU => {
            if ub == 0 {
                return None;
            }
            (ua % ub) as i64
        }
        BinK::DivS => {
            if sb == 0 {
                return None;
            }
            sa.wrapping_div(sb)
        }
        BinK::ModS => {
            if sb == 0 {
                return None;
            }
            sa.wrapping_rem(sb)
        }
        BinK::And => a & b,
        BinK::Or => a | b,
        BinK::Xor => a ^ b,
        BinK::Shl => {
            if ub >= bits {
                0
            } else {
                ((ua << ub) & ty.mask()) as i64
            }
        }
        BinK::ShrU => {
            if ub >= bits {
                0
            } else {
                (ua >> ub) as i64
            }
        }
        BinK::ShrS => {
            let s = if ub >= bits { bits - 1 } else { ub };
            sa >> s
        }
    };
    Some(ty.norm(r))
}

/// Is the value known to be non-negative when interpreted as signed at its width (i.e. top bit clear)?
fn top_bit_clear(f: &Func, inf: &Info, v: Val, ty: Ty) -> bool {
    match v {
        Val::K(k) => (ty.norm(k) >> (ty.bits() - 1)) & 1 == 0,
        Val::R(r) => match inf.single_def(f, r) {
            Some(Inst::Ext(_, s, false)) => {
                let st = match s {
                    Val::R(x) => f.ty(*x),
                    _ => return true,
                };
                st.bits() < ty.bits()
            }
            Some(Inst::Bin(BinK::And, _, _, Val::K(k))) => (ty.norm(*k) >> (ty.bits() - 1)) & 1 == 0,
            Some(Inst::Bin(BinK::ShrU, _, _, Val::K(k))) => *k > 0,
            Some(Inst::Cmp(..)) => true,
            _ => false,
        },
        Val::Addr(..) => true,
    }
}

/// Simplify a single instruction. Returns a replacement if changed.
fn simplify_inst(f: &Func, inf: &Info, ins: &Inst) -> Option<Inst> {
    match ins {
        Inst::Bin(op, d, a, b) => {
            let ty = f.ty(*d);
            let (op, a, b) = (*op, *a, *b);
            if let (Val::K(x), Val::K(y)) = (a, b) {
                if let Some(r) = eval_bin(op, x, y, ty) {
                    return Some(Inst::Copy(*d, Val::K(r)));
                }
            }
            // Canonicalize constants to the right.
            if op.commutative() && a.is_const() && !b.is_const() {
                return Some(Inst::Bin(op, *d, b, a));
            }
            if let Val::K(k) = b {
                let k = ty.norm(k);
                let all = ty.mask() as i64;
                match op {
                    BinK::Add | BinK::Sub | BinK::Or | BinK::Xor | BinK::Shl | BinK::ShrU | BinK::ShrS if k == 0 => {
                        return Some(Inst::Copy(*d, a));
                    }
                    BinK::Sub if matches!(a, Val::R(_)) => {
                        return Some(Inst::Bin(BinK::Add, *d, a, Val::K(ty.norm(-k))));
                    }
                    BinK::Mul | BinK::DivU | BinK::DivS if k == 1 => return Some(Inst::Copy(*d, a)),
                    BinK::Mul if k == 0 => return Some(Inst::Copy(*d, Val::K(0))),
                    BinK::ModU | BinK::ModS if k == 1 => return Some(Inst::Copy(*d, Val::K(0))),
                    BinK::And if k == 0 => return Some(Inst::Copy(*d, Val::K(0))),
                    BinK::And if k == all => return Some(Inst::Copy(*d, a)),
                    BinK::Or if k == all => return Some(Inst::Copy(*d, Val::K(all))),
                    BinK::Xor if k == all => return Some(Inst::Un(UnK::Not, *d, a)),
                    BinK::Shl | BinK::ShrU if k as u32 >= ty.bits() => return Some(Inst::Copy(*d, Val::K(0))),
                    BinK::ShrS if k as u32 >= ty.bits() => return Some(Inst::Bin(BinK::ShrS, *d, a, Val::K(ty.bits() as i64 - 1))),
                    BinK::Mul => {
                        if let Some(s) = is_pow2(k) {
                            return Some(Inst::Bin(BinK::Shl, *d, a, Val::K(s as i64)));
                        }
                        if k == all {
                            return Some(Inst::Un(UnK::Neg, *d, a));
                        }
                    }
                    BinK::DivU => {
                        if let Some(s) = is_pow2(k) {
                            return Some(Inst::Bin(BinK::ShrU, *d, a, Val::K(s as i64)));
                        }
                    }
                    BinK::ModU => {
                        if is_pow2(k).is_some() {
                            return Some(Inst::Bin(BinK::And, *d, a, Val::K(k - 1)));
                        }
                    }
                    BinK::DivS | BinK::ModS => {
                        if (ty.sext(k) > 0) && top_bit_clear(f, inf, a, ty) {
                            let nop = if op == BinK::DivS { BinK::DivU } else { BinK::ModU };
                            return Some(Inst::Bin(nop, *d, a, Val::K(k)));
                        }
                    }
                    BinK::ShrS => {
                        if top_bit_clear(f, inf, a, ty) {
                            return Some(Inst::Bin(BinK::ShrU, *d, a, Val::K(k)));
                        }
                    }
                    _ => {}
                }
                // Combine constant operations: (x op k1) op k2.
                if let Val::R(s) = a {
                    if inf.nuses[s as usize] == 1 {
                        if let Some(Inst::Bin(op2, _, x, Val::K(k1))) = inf.single_def(f, s) {
                            let x = *x;
                            if x != Val::R(*d) && reg_stable(f, inf, x, s) {
                                let k1 = ty.norm(*k1);
                                let comb = match (op, op2) {
                                    (BinK::Add, BinK::Add) => Some((BinK::Add, ty.norm(k + k1))),
                                    (BinK::And, BinK::And) => Some((BinK::And, k & k1)),
                                    (BinK::Or, BinK::Or) => Some((BinK::Or, k | k1)),
                                    (BinK::Xor, BinK::Xor) => Some((BinK::Xor, k ^ k1)),
                                    (BinK::Shl, BinK::Shl) => Some((BinK::Shl, k + k1)),
                                    (BinK::ShrU, BinK::ShrU) => Some((BinK::ShrU, k + k1)),
                                    _ => None,
                                };
                                if let Some((nop, nk)) = comb {
                                    return Some(Inst::Bin(nop, *d, x, Val::K(nk)));
                                }
                            }
                        }
                    }
                }
            }
            if a == b && matches!(a, Val::R(_)) {
                match op {
                    BinK::Sub | BinK::Xor => return Some(Inst::Copy(*d, Val::K(0))),
                    BinK::And | BinK::Or => return Some(Inst::Copy(*d, a)),
                    _ => {}
                }
            }
            if let Val::K(0) = a {
                match op {
                    BinK::Sub => return Some(Inst::Un(UnK::Neg, *d, b)),
                    BinK::Shl | BinK::ShrU | BinK::ShrS | BinK::DivU | BinK::DivS | BinK::ModU | BinK::ModS => {
                        return Some(Inst::Copy(*d, Val::K(0)));
                    }
                    _ => {}
                }
            }
            None
        }
        Inst::Un(op, d, Val::K(k)) => {
            let ty = f.ty(*d);
            let r = match op {
                UnK::Neg => ty.norm(k.wrapping_neg()),
                UnK::Not => ty.norm(!k),
            };
            Some(Inst::Copy(*d, Val::K(r)))
        }
        Inst::Cmp(c, d, Val::K(a), Val::K(b), ty) => Some(Inst::Copy(*d, Val::K(c.eval(*a, *b, *ty) as i64))),
        Inst::Cmp(c, d, a, b, ty) => {
            if a.is_const() && !b.is_const() {
                return Some(Inst::Cmp(c.swap(), *d, *b, *a, *ty));
            }
            if let Some((nc, na, nb, nty)) = narrow_cmp(f, inf, *c, *a, *b, *ty) {
                return Some(match (na, nb) {
                    (Val::K(x), Val::K(y)) => Inst::Copy(*d, Val::K(nc.eval(x, y, nty) as i64)),
                    _ => Inst::Cmp(nc, *d, na, nb, nty),
                });
            }
            if let Some(r) = trivial_cmp(*c, *b, *ty) {
                return Some(Inst::Copy(*d, Val::K(r as i64)));
            }
            // cmp(ne/eq, boolean, 0)
            if let (Val::R(s), Val::K(0), Cond::Ne | Cond::Eq) = (a, b, c) {
                if let Some(Inst::Cmp(c2, _, x, y, t2)) = inf.single_def(f, *s) {
                    if inf.nuses[*s as usize] == 1 && reg_stable(f, inf, *x, *s) && reg_stable(f, inf, *y, *s) {
                        let nc = if *c == Cond::Ne { *c2 } else { c2.negate() };
                        return Some(Inst::Cmp(nc, *d, *x, *y, *t2));
                    }
                }
                if *c == Cond::Ne && is_boolean(f, inf, *s) {
                    let st = f.ty(*s);
                    let dt = f.ty(*d);
                    return Some(if st == dt {
                        Inst::Copy(*d, *a)
                    } else if dt.bits() > st.bits() {
                        Inst::Ext(*d, *a, false)
                    } else {
                        Inst::Trunc(*d, *a)
                    });
                }
            }
            None
        }
        Inst::Ext(d, a, signed) => {
            let dt = f.ty(*d);
            match a {
                Val::K(k) => {
                    let st = if *signed { Ty::I8 } else { Ty::I64 };
                    let _ = st;
                    // Constants are already normalized to the source width; the source width is unknown here,
                    // so extension of a constant must have been folded at construction.
                    Some(Inst::Copy(*d, Val::K(dt.norm(*k))))
                }
                Val::R(s) => {
                    let st = f.ty(*s);
                    if st == dt {
                        return Some(Inst::Copy(*d, *a));
                    }
                    if st == Ty::Bit {
                        // Bit extension is always a zero extension.
                        if *signed {
                            return Some(Inst::Ext(*d, *a, false));
                        }
                    }
                    if let Some(Inst::Ext(_, x, s2)) = inf.single_def(f, *s) {
                        if (*s2 == *signed || !*s2) && reg_stable(f, inf, *x, *s) {
                            let xt = match x {
                                Val::R(r) => f.ty(*r),
                                _ => return None,
                            };
                            if xt == Ty::Bit {
                                return Some(Inst::Ext(*d, *x, false));
                            }
                            return Some(Inst::Ext(*d, *x, *s2));
                        }
                    }
                    None
                }
                Val::Addr(..) => None,
            }
        }
        Inst::Trunc(d, a) => {
            let dt = f.ty(*d);
            match a {
                Val::K(k) => Some(Inst::Copy(*d, Val::K(dt.norm(*k)))),
                Val::Addr(..) if dt == Ty::I8 => Some(Inst::Copy(*d, *a)),
                Val::R(s) => {
                    let st = f.ty(*s);
                    if st == dt {
                        return Some(Inst::Copy(*d, *a));
                    }
                    match inf.single_def(f, *s) {
                        Some(Inst::Ext(_, x, sg)) if reg_stable(f, inf, *x, *s) => {
                            let xt = match x {
                                Val::R(r) => f.ty(*r),
                                Val::K(k) => return Some(Inst::Copy(*d, Val::K(dt.norm(*k)))),
                                _ => return None,
                            };
                            if xt == dt {
                                Some(Inst::Copy(*d, *x))
                            } else if xt.bits() > dt.bits() {
                                Some(Inst::Trunc(*d, *x))
                            } else {
                                Some(Inst::Ext(*d, *x, *sg))
                            }
                        }
                        Some(Inst::Trunc(_, x)) if reg_stable(f, inf, *x, *s) => Some(Inst::Trunc(*d, *x)),
                        Some(Inst::Copy(_, Val::K(k))) => Some(Inst::Copy(*d, Val::K(dt.norm(*k)))),
                        _ => None,
                    }
                }
                Val::Addr(..) => None,
            }
        }
        Inst::Copy(d, Val::R(s)) if *d == *s => Some(Inst::Nop),
        _ => None,
    }
}

/// Compare against a constant that makes the result trivially known.
fn trivial_cmp(c: Cond, b: Val, ty: Ty) -> Option<bool> {
    let Val::K(k) = b else { return None };
    let u = ty.norm(k) as u64;
    let max = ty.mask();
    let s = ty.sext(k);
    let smin = -(1i64 << (ty.bits() - 1));
    let smax = (1i64 << (ty.bits() - 1)) - 1;
    match c {
        Cond::GeU if u == 0 => Some(true),
        Cond::LtU if u == 0 => Some(false),
        Cond::LeU if u == max => Some(true),
        Cond::GtU if u == max => Some(false),
        Cond::GeS if s == smin => Some(true),
        Cond::LtS if s == smin => Some(false),
        Cond::LeS if s == smax => Some(true),
        Cond::GtS if s == smax => Some(false),
        _ => None,
    }
}

fn is_boolean(f: &Func, inf: &Info, v: VReg) -> bool {
    if f.ty(v) == Ty::Bit {
        return true;
    }
    match inf.single_def(f, v) {
        Some(Inst::Cmp(..)) => true,
        Some(Inst::Ext(_, Val::R(x), false)) => is_boolean(f, inf, *x),
        Some(Inst::Bin(BinK::And, _, _, Val::K(1))) => true,
        _ => false,
    }
}

/// True if `x` (an operand of the def of `s`) still has the same value at every use of `s`.
/// Conservative: constants and single-def registers are stable; multi-def registers only when
/// the def of `s` and all its uses are in the same block with no redefinition of `x` in between.
pub fn reg_stable(f: &Func, inf: &Info, x: Val, s: VReg) -> bool {
    match x {
        Val::R(r) => {
            if inf.ssa[r as usize] && inf.ssa[s as usize] {
                return true;
            }
            if inf.ndefs[r as usize] == 0 {
                return true;
            }
            // Multi-def: check same-block uses.
            let (b, i) = inf.def_at[s as usize];
            if b == u32::MAX {
                return false;
            }
            let blk = &f.blocks[b as usize];
            let mut uses_seen = 0;
            for ins in &blk.insts[i as usize + 1..] {
                let u = ins.uses().iter().filter(|u| **u == s).count() as u32;
                uses_seen += u;
                if uses_seen == inf.nuses[s as usize] {
                    return true;
                }
                if ins.def() == Some(r) || ins.def() == Some(s) {
                    return false;
                }
            }
            uses_seen += blk.term.uses().iter().filter(|u| **u == s).count() as u32;
            uses_seen == inf.nuses[s as usize]
        }
        _ => true,
    }
}

fn is_param_reg(f: &Func, r: VReg) -> bool {
    f.params.iter().any(|p| *p == ParamLoc::Reg(r))
}

/// Narrow a comparison of extended values. Returns (cond, a, b, ty).
fn narrow_cmp(f: &Func, inf: &Info, c: Cond, a: Val, b: Val, ty: Ty) -> Option<(Cond, Val, Val, Ty)> {
    let Val::R(ra) = a else { return None };
    let Some(Inst::Ext(_, xa, sa)) = inf.single_def(f, ra) else { return None };
    if !reg_stable_at_cmp(f, inf, *xa, ra) {
        return None;
    }
    let (xa, sa) = (*xa, *sa);
    let wt = match xa {
        Val::R(r) => f.ty(r),
        _ => return None,
    };
    if wt == Ty::Bit {
        return None;
    }
    let (xb, sb) = match b {
        Val::K(k) => {
            // Does the constant fit in the narrow width?
            let fits_u = (ty.norm(k) as u64) <= wt.mask();
            let sk = ty.sext(k);
            let fits_s = sk >= -(1i64 << (wt.bits() - 1)) && sk < (1i64 << (wt.bits() - 1));
            if !sa {
                if !fits_u {
                    // zext(x) compared with a constant outside [0, max]: result is known.
                    let kk = if c.is_signed() { ty.sext(k) } else { ty.norm(k) };
                    let r = match c {
                        Cond::Eq => false,
                        Cond::Ne => true,
                        Cond::LtS | Cond::LeS | Cond::LtU | Cond::LeU => kk > 0,
                        Cond::GtS | Cond::GeS | Cond::GtU | Cond::GeU => kk < 0,
                    };
                    return Some((Cond::Eq, Val::K(0), Val::K(if r { 0 } else { 1 }), Ty::I8));
                }
                (Val::K(ty.norm(k)), false)
            } else {
                if !fits_s {
                    return None;
                }
                (Val::K(wt.norm(k)), true)
            }
        }
        Val::R(rb) => {
            let Some(Inst::Ext(_, xb, sb)) = inf.single_def(f, rb) else { return None };
            if !reg_stable_at_cmp(f, inf, *xb, rb) {
                return None;
            }
            let bt = match xb {
                Val::R(r) => f.ty(*r),
                Val::K(_) => wt,
                _ => return None,
            };
            if bt != wt {
                return None;
            }
            (*xb, *sb)
        }
        _ => return None,
    };
    if sa != sb {
        return None;
    }
    let nc = if !sa {
        match c {
            Cond::LtS => Cond::LtU,
            Cond::LeS => Cond::LeU,
            Cond::GtS => Cond::GtU,
            Cond::GeS => Cond::GeU,
            c => c,
        }
    } else {
        c
    };
    Some((nc, xa, xb, wt))
}

fn reg_stable_at_cmp(f: &Func, inf: &Info, x: Val, s: VReg) -> bool {
    reg_stable(f, inf, x, s)
}

/// `op(zext(x), k)` where the result fits in x's width: compute narrow, then extend.
fn narrow_ext_ops(f: &mut Func) -> bool {
    let inf = info(f);
    for bi in 0..f.blocks.len() {
        for ii in 0..f.blocks[bi].insts.len() {
            let Inst::Bin(op, d, Val::R(e), Val::K(k)) = f.blocks[bi].insts[ii] else { continue };
            if !matches!(op, BinK::And | BinK::ShrU | BinK::DivU | BinK::ModU) {
                continue;
            }
            let Some(Inst::Ext(_, x @ Val::R(xr), false)) = inf.single_def(f, e) else { continue };
            let (x, xr) = (*x, *xr);
            if !reg_stable(f, &inf, x, e) {
                continue;
            }
            let wt = f.ty(xr);
            if wt == Ty::Bit {
                continue;
            }
            let dt = f.ty(d);
            let ok = match op {
                BinK::And | BinK::DivU => true,
                BinK::ModU => (k as u64) <= wt.mask() + 1 || true,
                BinK::ShrU => true,
                _ => false,
            };
            if !ok {
                continue;
            }
            let nk = match op {
                BinK::And => wt.norm(k),
                BinK::ShrU => {
                    if k as u32 >= wt.bits() {
                        f.blocks[bi].insts[ii] = Inst::Copy(d, Val::K(0));
                        return true;
                    }
                    k
                }
                BinK::DivU | BinK::ModU => {
                    if (k as u64) > wt.mask() {
                        // x / k == 0 ; x % k == x
                        f.blocks[bi].insts[ii] = if op == BinK::DivU { Inst::Copy(d, Val::K(0)) } else { Inst::Copy(d, Val::R(e)) };
                        return true;
                    }
                    k
                }
                _ => unreachable!(),
            };
            let t = f.new_vreg(wt);
            let blk = &mut f.blocks[bi];
            blk.insts[ii] = Inst::Ext(d, Val::R(t), false);
            blk.insts.insert(ii, Inst::Bin(op, t, x, Val::K(nk)));
            let _ = dt;
            return true;
        }
    }
    false
}

/// Demand-driven width narrowing: retype single-use values whose only use truncates them.
fn narrow(f: &mut Func) -> bool {
    if narrow_ext_ops(f) {
        return true;
    }
    let inf = info(f);
    let mut changed = false;
    // Find candidates: Trunc(d, s) where s single-def, single-use.
    let mut cands: Vec<(VReg, VReg)> = Vec::new();
    for b in &f.blocks {
        for ins in &b.insts {
            if let Inst::Trunc(d, Val::R(s)) = ins {
                if inf.ndefs[*s as usize] == 1 && inf.nuses[*s as usize] == 1 && !is_param_reg(f, *s) {
                    cands.push((*d, *s));
                }
            }
        }
    }
    for (d, s) in cands {
        let w = f.ty(d);
        if w == Ty::Bit {
            continue;
        }
        let (bi, ii) = inf.def_at[s as usize];
        if bi == u32::MAX {
            continue;
        }
        let def = f.blocks[bi as usize].insts[ii as usize].clone();
        let st = f.ty(s);
        let mut pre: Vec<Inst> = Vec::new();
        let mut trunc_op = |f: &mut Func, v: Val, pre: &mut Vec<Inst>| -> Val {
            match v {
                Val::K(k) => Val::K(w.norm(k)),
                Val::Addr(..) => {
                    let t = f.new_vreg(w);
                    pre.push(Inst::Trunc(t, v));
                    Val::R(t)
                }
                Val::R(r) => {
                    let t = f.new_vreg(w);
                    pre.push(Inst::Trunc(t, Val::R(r)));
                    Val::R(t)
                }
            }
        };
        let new_def = match def {
            Inst::Bin(op @ (BinK::Add | BinK::Sub | BinK::Mul | BinK::And | BinK::Or | BinK::Xor), _, a, b) => {
                let na = trunc_op(f, a, &mut pre);
                let nb = trunc_op(f, b, &mut pre);
                Some(Inst::Bin(op, s, na, nb))
            }
            Inst::Bin(BinK::Shl, _, a, Val::K(k)) => {
                let na = trunc_op(f, a, &mut pre);
                Some(if k as u32 >= w.bits() { Inst::Copy(s, Val::K(0)) } else { Inst::Bin(BinK::Shl, s, na, Val::K(k)) })
            }
            Inst::Bin(op @ (BinK::ShrU | BinK::ShrS), _, a, Val::K(k)) if (k as u32) < st.bits() => {
                // (a >> k) truncated to w: if a is an extension from <= w bits... only handle zext source.
                let _ = op;
                None.or_else(|| {
                    let _ = (a, k);
                    None
                })
            }
            Inst::Un(op, _, a) => {
                let na = trunc_op(f, a, &mut pre);
                Some(Inst::Un(op, s, na))
            }
            Inst::Copy(_, a) => {
                let na = trunc_op(f, a, &mut pre);
                Some(Inst::Copy(s, na))
            }
            Inst::Ext(_, a, sg) => {
                let at = match a {
                    Val::R(r) => f.ty(r),
                    _ => continue,
                };
                if at.bits() > w.bits() {
                    Some(Inst::Trunc(s, a))
                } else if at == w {
                    Some(Inst::Copy(s, a))
                } else {
                    Some(Inst::Ext(s, a, sg))
                }
            }
            Inst::Trunc(_, a) => Some(Inst::Trunc(s, a)),
            Inst::Load(_, m) if !mem_is_volatile(&m) => Some(Inst::Load(s, m)),
            _ => None,
        };
        let Some(new_def) = new_def else { continue };
        let npre = pre.len();
        f.vregs[s as usize].ty = w;
        let blk = &mut f.blocks[bi as usize];
        blk.insts[ii as usize] = new_def;
        for (k, p) in pre.into_iter().enumerate() {
            blk.insts.insert(ii as usize + k, p);
        }
        // Replace the Trunc use with a Copy.
        for b in &mut f.blocks {
            for ins in &mut b.insts {
                if *ins == Inst::Trunc(d, Val::R(s)) {
                    *ins = Inst::Copy(d, Val::R(s));
                }
            }
        }
        let _ = npre;
        changed = true;
        // Info is now stale (indices shifted): stop and let the caller iterate.
        break;
    }
    changed
}

/// `c = copy x; ...; x = ...; d = load [c]` -> `c = copy x; d = load [x]; ...; x = ...`
/// (typical of `*p++`), so that the copy dies.
fn hoist_post_inc_loads(f: &mut Func) -> bool {
    let inf = info(f);
    for bi in 0..f.blocks.len() {
        let n = f.blocks[bi].insts.len();
        for j in 0..n {
            let (d, c, off, sp) = match &f.blocks[bi].insts[j] {
                Inst::Load(d, Mem::Ptr(Val::R(c), off, sp)) => (*d, *c, *off, *sp),
                _ => continue,
            };
            if inf.nuses[c as usize] != 1 || inf.ndefs[d as usize] != 1 || inf.ndefs[c as usize] != 1 {
                continue;
            }
            let (cb, ci) = inf.def_at[c as usize];
            if cb as usize != bi || ci as usize >= j {
                continue;
            }
            let Inst::Copy(_, Val::R(x)) = f.blocks[bi].insts[ci as usize] else { continue };
            let pure_space = matches!(sp, PSpace::S(crate::types::Space::Code));
            let between = &f.blocks[bi].insts[ci as usize + 1..j];
            if !pure_space && between.iter().any(|i| i.has_side_effects()) {
                continue;
            }
            if between.iter().any(|i| i.uses().contains(&d)) {
                continue;
            }
            if !between.iter().any(|i| i.def() == Some(x)) {
                continue;
            }
            let blk = &mut f.blocks[bi];
            let _ = blk.insts.remove(j);
            blk.insts.insert(ci as usize + 1, Inst::Load(d, Mem::Ptr(Val::R(x), off, sp)));
            return true;
        }
    }
    false
}

/// Fuse comparisons into branches.
fn fuse_branches(f: &mut Func) -> bool {
    let inf = info(f);
    let mut changed = false;
    for bi in 0..f.blocks.len() {
        let term = f.blocks[bi].term.clone();
        let new = match &term {
            Term::Br(Val::R(s), t, e) => match inf.single_def(f, *s) {
                Some(Inst::Cmp(c, _, a, b, ty)) if inf.nuses[*s as usize] == 1 && reg_stable(f, &inf, *a, *s) && reg_stable(f, &inf, *b, *s) => {
                    Some(Term::CmpBr(*c, *a, *b, *ty, *t, *e))
                }
                Some(Inst::Ext(_, x @ Val::R(_), _)) if reg_stable(f, &inf, *x, *s) => Some(Term::Br(*x, *t, *e)),
                Some(Inst::Copy(_, x)) if reg_stable(f, &inf, *x, *s) => Some(Term::Br(*x, *t, *e)),
                Some(Inst::Un(UnK::Neg, _, x)) if reg_stable(f, &inf, *x, *s) => Some(Term::Br(*x, *t, *e)),
                _ => None,
            },
            Term::CmpBr(c @ (Cond::Eq | Cond::Ne), Val::R(s), Val::K(0), _, t, e) => {
                let (t, e) = if *c == Cond::Ne { (*t, *e) } else { (*e, *t) };
                Some(Term::Br(Val::R(*s), t, e))
            }
            Term::CmpBr(c, a, b, ty, t, e) => {
                if a.is_const() && !b.is_const() {
                    Some(Term::CmpBr(c.swap(), *b, *a, *ty, *t, *e))
                } else if let Some((nc, na, nb, nty)) = narrow_cmp(f, &inf, *c, *a, *b, *ty) {
                    Some(match (na, nb) {
                        (Val::K(x), Val::K(y)) => Term::Jmp(if nc.eval(x, y, nty) { *t } else { *e }),
                        _ => Term::CmpBr(nc, na, nb, nty, *t, *e),
                    })
                } else if let Some(r) = trivial_cmp(*c, *b, *ty) {
                    Some(Term::Jmp(if r { *t } else { *e }))
                } else {
                    // Canonicalize <= / > against constants into < / >= (cheaper on the 8051).
                    match (c, b) {
                        (Cond::LeU, Val::K(k)) if (ty.norm(*k) as u64) < ty.mask() => Some(Term::CmpBr(Cond::LtU, *a, Val::K(ty.norm(k + 1)), *ty, *t, *e)),
                        (Cond::GtU, Val::K(k)) if (ty.norm(*k) as u64) < ty.mask() => Some(Term::CmpBr(Cond::GeU, *a, Val::K(ty.norm(k + 1)), *ty, *t, *e)),
                        (Cond::LeS, Val::K(k)) if ty.sext(*k) < (1i64 << (ty.bits() - 1)) - 1 => Some(Term::CmpBr(Cond::LtS, *a, Val::K(ty.norm(k + 1)), *ty, *t, *e)),
                        (Cond::GtS, Val::K(k)) if ty.sext(*k) < (1i64 << (ty.bits() - 1)) - 1 => Some(Term::CmpBr(Cond::GeS, *a, Val::K(ty.norm(k + 1)), *ty, *t, *e)),
                        _ => None,
                    }
                }
            }
            Term::Switch(Val::R(s), ty, cases, d) => match inf.single_def(f, *s) {
                Some(Inst::Ext(_, x @ Val::R(xr), false)) if reg_stable(f, &inf, *x, *s) => {
                    // Switch on a zero-extended value: narrow and drop impossible cases.
                    let nt = f.ty(*xr);
                    if nt == Ty::Bit {
                        None
                    } else {
                        let nc: Vec<(i64, BlockId)> = cases.iter().filter(|c| (c.0 as u64) <= nt.mask()).copied().collect();
                        let _ = ty;
                        Some(Term::Switch(*x, nt, nc, *d))
                    }
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(n) = new {
            if n != term {
                f.blocks[bi].term = n;
                changed = true;
            }
        }
    }
    changed
}

pub fn run(f: &mut Func) -> bool {
    let mut changed = false;
    loop {
        let mut c = false;
        let mut inf = info(f);
        for bi in 0..f.blocks.len() {
            for ii in 0..f.blocks[bi].insts.len() {
                let ins = &f.blocks[bi].insts[ii];
                if let Some(n) = simplify_inst(f, &inf, ins) {
                    if n != *ins {
                        f.blocks[bi].insts[ii] = n;
                        c = true;
                        inf = info(f);
                    }
                }
            }
        }
        for b in &mut f.blocks {
            let n = b.insts.len();
            b.insts.retain(|i| *i != Inst::Nop);
            c |= n != b.insts.len();
        }
        c |= fuse_branches(f);
        if !c {
            c |= hoist_post_inc_loads(f);
        }
        if !c {
            c |= narrow(f);
        }
        if !c {
            break;
        }
        changed = true;
    }
    changed
}

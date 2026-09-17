//! Demanded-bits analysis and width narrowing.

use crate::ir::*;
use crate::types::Space;

fn width_mask(t: Ty) -> u64 {
    t.mask()
}

/// Mask of all bits up to and including the highest set bit of `m`.
fn low_fill(m: u64) -> u64 {
    if m == 0 {
        0
    } else {
        let hb = 63 - m.leading_zeros();
        if hb >= 63 { !0 } else { (1u64 << (hb + 1)) - 1 }
    }
}

fn val_ty(f: &Func, v: &Val, default: Ty) -> Ty {
    match v {
        Val::R(r) => f.ty(*r),
        _ => default,
    }
}

fn ptr_base_demand(sp: &PSpace, bty: Ty) -> u64 {
    match sp {
        PSpace::S(Space::Data | Space::Idata) => 0xff,
        _ => width_mask(bty),
    }
}

pub fn demanded(f: &Func, param_tys: &dyn Fn(&Callee) -> Option<Vec<Ty>>) -> Vec<u64> {
    let n = f.vregs.len();
    let mut dem = vec![0u64; n];
    let mut changed = true;
    let mut iter = 0;
    while changed && iter < 30 {
        changed = false;
        iter += 1;
        let mut add = |dem: &mut Vec<u64>, v: &Val, m: u64| {
            if let Val::R(r) = v {
                let r = *r as usize;
                let m = m & f.vregs[r].ty.mask();
                if dem[r] | m != dem[r] {
                    dem[r] |= m;
                    changed = true;
                }
            }
        };
        for b in &f.blocks {
            for i in &b.insts {
                let dd = |d: &VReg, dem: &Vec<u64>| dem[*d as usize];
                match i {
                    Inst::Copy(d, a) => {
                        let m = dd(d, &dem);
                        add(&mut dem, a, m);
                    }
                    Inst::Bin(op, d, a, bb) => {
                        let dt = f.ty(*d);
                        let m = dd(d, &dem);
                        if m == 0 {
                            continue;
                        }
                        match op {
                            BinK::Add | BinK::Sub | BinK::Mul => {
                                let lf = low_fill(m);
                                add(&mut dem, a, lf);
                                add(&mut dem, bb, lf);
                            }
                            BinK::And => {
                                let ma = if let Val::K(k) = bb { m & (*k as u64) } else { m };
                                let mb = if let Val::K(k) = a { m & (*k as u64) } else { m };
                                add(&mut dem, a, ma);
                                add(&mut dem, bb, mb);
                            }
                            BinK::Or | BinK::Xor => {
                                let ma = if let (BinK::Or, Val::K(k)) = (op, bb) { m & !(*k as u64) } else { m };
                                add(&mut dem, a, ma);
                                add(&mut dem, bb, m);
                            }
                            BinK::Shl => match bb {
                                Val::K(k) => add(&mut dem, a, m >> (*k as u64).min(63)),
                                _ => {
                                    add(&mut dem, a, low_fill(m));
                                    add(&mut dem, bb, 0xff);
                                }
                            },
                            BinK::ShrU => match bb {
                                Val::K(k) => add(&mut dem, a, (m << (*k as u64).min(63)) & width_mask(dt)),
                                _ => {
                                    add(&mut dem, a, width_mask(dt));
                                    add(&mut dem, bb, 0xff);
                                }
                            },
                            BinK::ShrS => match bb {
                                Val::K(k) => {
                                    let k = (*k as u64).min(63);
                                    let mut ma = (m << k) & width_mask(dt);
                                    // Bits shifted in from the sign.
                                    let top_in = if k >= dt.bits() as u64 { !0 } else { !(width_mask(dt) >> k) & width_mask(dt) };
                                    if m & top_in != 0 {
                                        ma |= 1u64 << (dt.bits() - 1);
                                    }
                                    add(&mut dem, a, ma);
                                }
                                _ => {
                                    add(&mut dem, a, width_mask(dt));
                                    add(&mut dem, bb, 0xff);
                                }
                            },
                            _ => {
                                add(&mut dem, a, width_mask(dt));
                                add(&mut dem, bb, width_mask(dt));
                            }
                        }
                    }
                    Inst::Un(op, d, a) => {
                        let m = dd(d, &dem);
                        let m = if *op == UnK::Neg { low_fill(m) } else { m };
                        add(&mut dem, a, m);
                    }
                    Inst::Cmp(_, _, a, bb, ty) => {
                        add(&mut dem, a, width_mask(*ty));
                        add(&mut dem, bb, width_mask(*ty));
                    }
                    Inst::Ext(d, a, signed) => {
                        let m = dd(d, &dem);
                        let at = val_ty(f, a, Ty::I8);
                        let mut ma = m & width_mask(at);
                        if *signed && at != Ty::Bit && (m & !width_mask(at)) != 0 {
                            ma |= 1u64 << (at.bits() - 1);
                        }
                        add(&mut dem, a, ma);
                    }
                    Inst::Trunc(d, a) => {
                        let m = dd(d, &dem);
                        add(&mut dem, a, m);
                    }
                    Inst::Load(_, m) => {
                        if let Mem::Ptr(p, _, sp) = m {
                            let pt = val_ty(f, p, Ty::I16);
                            add(&mut dem, p, ptr_base_demand(sp, pt));
                        }
                    }
                    Inst::Store(m, v, ty) => {
                        if let Mem::Ptr(p, _, sp) = m {
                            let pt = val_ty(f, p, Ty::I16);
                            add(&mut dem, p, ptr_base_demand(sp, pt));
                        }
                        add(&mut dem, v, width_mask(*ty));
                    }
                    Inst::Call(_, c, args) => {
                        if let Callee::Indirect(t) = c {
                            add(&mut dem, t, 0xffff);
                        }
                        let pt = param_tys(c);
                        for (k, a) in args.iter().enumerate() {
                            let m = match pt.as_ref().and_then(|p| p.get(k)) {
                                Some(t) => width_mask(*t),
                                None => !0,
                            };
                            add(&mut dem, a, m);
                        }
                    }
                    Inst::MemCopy(a, bb, _) => {
                        for m in [a, bb] {
                            if let Mem::Ptr(p, _, sp) = m {
                                let pt = val_ty(f, p, Ty::I16);
                                add(&mut dem, p, ptr_base_demand(sp, pt));
                            }
                        }
                    }
                    Inst::MemSet(m, v, _) => {
                        if let Mem::Ptr(p, _, sp) = m {
                            let pt = val_ty(f, p, Ty::I16);
                            add(&mut dem, p, ptr_base_demand(sp, pt));
                        }
                        add(&mut dem, v, 0xff);
                    }
                    Inst::CritExit(v) => add(&mut dem, &Val::R(*v), 1),
                    _ => {}
                }
            }
            match &b.term {
                Term::Br(v, _, _) => {
                    let t = val_ty(f, v, Ty::I16);
                    add(&mut dem, v, width_mask(t));
                }
                Term::CmpBr(_, a, bb, ty, _, _) => {
                    add(&mut dem, a, width_mask(*ty));
                    add(&mut dem, bb, width_mask(*ty));
                }
                Term::Switch(v, ty, _, _) => add(&mut dem, v, width_mask(*ty)),
                Term::Ret(Some(v)) => add(&mut dem, v, f.ret.map(width_mask).unwrap_or(!0)),
                _ => {}
            }
        }
        // Parameters are fully demanded by the caller interface only if used; nothing to do.
    }
    dem
}

fn width_for(m: u64) -> Option<Ty> {
    if m <= 0xff {
        Some(Ty::I8)
    } else if m <= 0xffff {
        Some(Ty::I16)
    } else if m <= 0xff_ffff {
        Some(Ty::I24)
    } else if m <= 0xffff_ffff {
        Some(Ty::I32)
    } else {
        None
    }
}

fn narrowable_def(i: &Inst) -> bool {
    match i {
        Inst::Copy(..) | Inst::Un(..) | Inst::Cmp(..) | Inst::Ext(..) | Inst::Trunc(..) => true,
        Inst::Bin(op, _, _, _) => matches!(op, BinK::Add | BinK::Sub | BinK::Mul | BinK::And | BinK::Or | BinK::Xor | BinK::Shl),
        Inst::Load(_, m) => !mem_is_volatile(m),
        _ => false,
    }
}

/// Narrow vregs whose demanded bits fit a smaller type. Returns true if anything changed.
pub fn run(f: &mut Func, param_tys: &dyn Fn(&Callee) -> Option<Vec<Ty>>) -> bool {
    let dem = demanded(f, param_tys);
    let n = f.vregs.len();
    let is_param: Vec<bool> = {
        let mut v = vec![false; n];
        for p in &f.params {
            if let ParamLoc::Reg(r) = p {
                v[*r as usize] = true;
            }
        }
        v
    };
    // Candidate widths.
    let mut new_ty: Vec<Option<Ty>> = vec![None; n];
    let mut has_def = vec![false; n];
    let mut ok = vec![true; n];
    for b in &f.blocks {
        for i in &b.insts {
            if let Some(d) = i.def() {
                has_def[d as usize] = true;
                if !narrowable_def(i) {
                    ok[d as usize] = false;
                }
                // Ext to a width below its source width after narrowing is handled in fixup.
            }
        }
    }
    let mut any = false;
    for v in 0..n {
        let t = f.vregs[v].ty;
        if t == Ty::Bit || is_param[v] || !has_def[v] || !ok[v] {
            continue;
        }
        let Some(w) = width_for(dem[v]) else { continue };
        if w.bits() < t.bits() {
            new_ty[v] = Some(w);
            any = true;
        }
    }
    if !any {
        return false;
    }
    let old_ty: Vec<Ty> = f.vregs.iter().map(|v| v.ty).collect();
    for v in 0..n {
        if let Some(w) = new_ty[v] {
            f.vregs[v].ty = w;
        }
    }
    // Fix up operand types.
    let nblocks = f.blocks.len();
    for bi in 0..nblocks {
        let insts = std::mem::take(&mut f.blocks[bi].insts);
        let mut out: Vec<Inst> = Vec::with_capacity(insts.len());
        for mut ins in insts {
            fixup_inst(f, &mut ins, &mut out, &old_ty, param_tys);
            out.push(ins);
        }
        let mut term = std::mem::replace(&mut f.blocks[bi].term, Term::Unreachable);
        fixup_term(f, &mut term, &mut out);
        f.blocks[bi].insts = out;
        f.blocks[bi].term = term;
    }
    true
}

/// Convert value `v` to type `want` (inserting instructions into `out`).
fn conv(f: &mut Func, v: Val, want: Ty, out: &mut Vec<Inst>, signed: bool) -> Val {
    match v {
        Val::K(k) => Val::K(want.norm(k)),
        Val::Addr(..) => {
            if want == Ty::I16 {
                v
            } else if want.bits() < 16 {
                let t = f.new_vreg(want);
                out.push(Inst::Trunc(t, v));
                Val::R(t)
            } else {
                let t = f.new_vreg(want);
                out.push(Inst::Ext(t, v, false));
                Val::R(t)
            }
        }
        Val::R(r) => {
            let t = f.ty(r);
            if t == want {
                return v;
            }
            let nv = f.new_vreg(want);
            if want.bits() < t.bits() {
                out.push(Inst::Trunc(nv, v));
            } else {
                out.push(Inst::Ext(nv, v, signed));
            }
            Val::R(nv)
        }
    }
}

fn fixup_mem(f: &mut Func, m: &mut Mem, out: &mut Vec<Inst>) {
    if let Mem::Ptr(p, _, sp) = m {
        if let Val::R(r) = p {
            let t = f.ty(*r);
            let want = match sp {
                PSpace::Generic => Ty::I24,
                _ => Ty::I16,
            };
            let small_ok = matches!(sp, PSpace::S(Space::Data | Space::Idata)) && t == Ty::I8;
            if t != want && !small_ok {
                *p = conv(f, *p, want, out, false);
            }
        }
    }
}

fn fixup_inst(f: &mut Func, ins: &mut Inst, out: &mut Vec<Inst>, old_ty: &[Ty], param_tys: &dyn Fn(&Callee) -> Option<Vec<Ty>>) {
    match ins {
        Inst::Copy(d, a) => {
            let dt = f.ty(*d);
            *a = conv(f, *a, dt, out, false);
        }
        Inst::Bin(op, d, a, b) => {
            let dt = f.ty(*d);
            *a = conv(f, *a, dt, out, *op == BinK::ShrS);
            if matches!(op, BinK::Shl | BinK::ShrU | BinK::ShrS) {
                if let Val::R(_) = b {
                    *b = conv(f, *b, Ty::I8, out, false);
                }
            } else {
                *b = conv(f, *b, dt, out, false);
            }
        }
        Inst::Un(_, d, a) => {
            let dt = f.ty(*d);
            *a = conv(f, *a, dt, out, false);
        }
        Inst::Cmp(_, _, a, b, ty) => {
            let t = *ty;
            *a = conv(f, *a, t, out, false);
            *b = conv(f, *b, t, out, false);
        }
        Inst::Ext(d, a, s) => {
            let dt = f.ty(*d);
            let at = match a {
                Val::R(r) => f.ty(*r),
                Val::K(_) => {
                    // Source width unknown: it was the old source width; keep as is (normalized).
                    *ins = Inst::Copy(*d, Val::K(dt.norm(match a {
                        Val::K(k) => *k,
                        _ => 0,
                    })));
                    return;
                }
                _ => Ty::I16,
            };
            let _ = s;
            let (d, a) = (*d, *a);
            if at == dt {
                *ins = Inst::Copy(d, a);
            } else if at.bits() > dt.bits() {
                *ins = Inst::Trunc(d, a);
            }
        }
        Inst::Trunc(d, a) => {
            let dt = f.ty(*d);
            let at = match a {
                Val::R(r) => f.ty(*r),
                _ => Ty::I64,
            };
            if at == dt {
                *ins = Inst::Copy(*d, *a);
            } else if at.bits() < dt.bits() {
                *ins = Inst::Ext(*d, *a, false);
            }
        }
        Inst::Load(_, m) => fixup_mem(f, m, out),
        Inst::Store(m, v, ty) => {
            let t = *ty;
            fixup_mem(f, m, out);
            *v = conv(f, *v, t, out, false);
        }
        Inst::Call(_, c, args) => {
            if let Callee::Indirect(t) = c {
                *t = conv(f, *t, Ty::I16, out, false);
            }
            let pts = param_tys(c);
            for (k, a) in args.iter_mut().enumerate() {
                if let Val::R(r) = a {
                    let want = pts.as_ref().and_then(|p| p.get(k).copied()).unwrap_or(old_ty.get(*r as usize).copied().unwrap_or(Ty::I16));
                    *a = conv(f, *a, want, out, false);
                }
            }
        }
        Inst::MemCopy(a, b, _) => {
            fixup_mem(f, a, out);
            fixup_mem(f, b, out);
        }
        Inst::MemSet(m, v, _) => {
            fixup_mem(f, m, out);
            *v = conv(f, *v, Ty::I8, out, false);
        }
        _ => {}
    }
}

fn fixup_term(f: &mut Func, t: &mut Term, out: &mut Vec<Inst>) {
    match t {
        Term::CmpBr(_, a, b, ty, _, _) => {
            let tt = *ty;
            *a = conv(f, *a, tt, out, false);
            *b = conv(f, *b, tt, out, false);
        }
        Term::Switch(v, ty, _, _) => {
            let tt = *ty;
            *v = conv(f, *v, tt, out, false);
        }
        Term::Ret(Some(v)) => {
            if let Some(rt) = f.ret {
                *v = conv(f, *v, rt, out, false);
            }
        }
        _ => {}
    }
}

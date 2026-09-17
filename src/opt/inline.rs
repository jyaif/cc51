//! Function inlining.

use crate::ast::FuncId;
use crate::ir::*;
use std::collections::HashMap;

/// Rough code size estimate (bytes) of an IR function.
pub fn cost(f: &Func) -> u32 {
    let mut c = 0;
    for b in &f.blocks {
        for i in &b.insts {
            c += inst_cost(f, i);
        }
        c += match &b.term {
            Term::Jmp(_) => 1,
            Term::Br(..) => 3,
            Term::CmpBr(_, _, _, ty, _, _) => 2 + 3 * ty.bytes(),
            Term::Switch(_, _, cases, _) => 2 + 4 * cases.len() as u32,
            Term::Ret(v) => 1 + if v.is_some() { 2 } else { 0 },
            Term::Unreachable => 0,
        };
    }
    c
}

fn inst_cost(f: &Func, i: &Inst) -> u32 {
    let w = |d: VReg| f.ty(d).bytes();
    match i {
        Inst::Copy(d, _) => 2 * w(*d),
        Inst::Bin(op, d, _, b) => {
            let per = if b.is_const() { 3 } else { 4 };
            match op {
                BinK::Mul | BinK::DivU | BinK::DivS | BinK::ModU | BinK::ModS => 6 + 2 * w(*d),
                BinK::Shl | BinK::ShrU | BinK::ShrS => 3 * w(*d) + 2,
                _ => per * w(*d),
            }
        }
        Inst::Un(_, d, _) => 3 * w(*d),
        Inst::Cmp(_, _, _, _, ty) => 4 + 3 * ty.bytes(),
        Inst::Ext(d, _, _) | Inst::Trunc(d, _) => 2 * w(*d),
        Inst::Load(d, m) | Inst::Store(m, Val::R(d), _) => match m {
            Mem::Sym(..) | Mem::Abs(..) => 2 + 2 * w(*d),
            Mem::Ptr(..) => 4 + 2 * w(*d),
        },
        Inst::Store(m, _, ty) => match m {
            Mem::Sym(..) | Mem::Abs(..) => 3 * ty.bytes(),
            Mem::Ptr(..) => 4 + 2 * ty.bytes(),
        },
        Inst::Call(_, _, args) => 3 + 2 * args.len() as u32,
        Inst::Asm(t) => 3 * t.lines().count() as u32,
        Inst::CritEnter(_) | Inst::CritExit(_) => 5,
        Inst::MemCopy(_, _, n) | Inst::MemSet(_, _, n) => 4 + (*n).min(10),
        Inst::Nop => 0,
    }
}

fn call_cost(nargs: usize) -> u32 {
    3 + 2 * nargs as u32
}

pub fn can_inline(f: &Func) -> bool {
    if f.attrs.interrupt.is_some() || f.attrs.naked || f.variadic || f.ret_obj.is_some() {
        return false;
    }
    if f.params.iter().any(|p| matches!(p, ParamLoc::Frame(_))) {
        return false;
    }
    for b in &f.blocks {
        for i in &b.insts {
            if matches!(i, Inst::Asm(_)) {
                return false;
            }
        }
    }
    true
}

/// Inline `callee` at block `bi`, instruction `ii` of `caller`.
pub fn inline_call(caller: &mut Func, bi: usize, ii: usize, callee: &Func) {
    let Inst::Call(dst, _, args) = caller.blocks[bi].insts[ii].clone() else { panic!("not a call") };
    // Split the block.
    let tail_insts = caller.blocks[bi].insts.split_off(ii + 1);
    caller.blocks[bi].insts.pop();
    let tail_term = std::mem::replace(&mut caller.blocks[bi].term, Term::Unreachable);
    let cont = caller.new_block();
    caller.blocks[cont as usize].insts = tail_insts;
    caller.blocks[cont as usize].term = tail_term;
    // Vreg map.
    let base_v = caller.vregs.len() as u32;
    for v in &callee.vregs {
        caller.vregs.push(v.clone());
    }
    let mapv = |v: VReg| v + base_v;
    // Frame objects.
    let base_o = caller.frame.len() as u32;
    for o in &callee.frame {
        let mut o = o.clone();
        o.param = None;
        caller.frame.push(o);
    }
    let caller_id = caller.id;
    let callee_id = callee.id;
    let map_sym = |s: Sym| -> Sym {
        match s {
            Sym::Frame(f, k) if f == callee_id => Sym::Frame(caller_id, k + base_o),
            s => s,
        }
    };
    let map_val = |v: Val| -> Val {
        match v {
            Val::R(r) => Val::R(mapv(r)),
            Val::Addr(s, o) => Val::Addr(map_sym(s), o),
            k => k,
        }
    };
    // Pointer bases inside memory operands are remapped by for_each_val_mut; only symbols here.
    let map_mem = |m: Mem| -> Mem {
        match m {
            Mem::Sym(s, o) => Mem::Sym(map_sym(s), o),
            a => a,
        }
    };
    // Blocks.
    let base_b = caller.blocks.len() as u32;
    for b in &callee.blocks {
        let mut nb = Block { insts: Vec::with_capacity(b.insts.len()), term: Term::Unreachable };
        for i in &b.insts {
            let mut i = i.clone();
            if let Some(d) = i.def() {
                i.set_def(mapv(d));
            }
            i.for_each_val_mut(|v| *v = map_val(*v));
            match &mut i {
                Inst::Load(_, m) => *m = map_mem(*m),
                Inst::Store(m, _, _) => *m = map_mem(*m),
                Inst::MemCopy(a, b, _) => {
                    *a = map_mem(*a);
                    *b = map_mem(*b);
                }
                Inst::MemSet(a, _, _) => *a = map_mem(*a),
                Inst::CritExit(r) => *r = mapv(*r),
                _ => {}
            }
            nb.insts.push(i);
        }
        let mut t = b.term.clone();
        t.for_each_val_mut(|v| *v = map_val(*v));
        for s in t.succs_mut() {
            *s += base_b;
        }
        if let Term::Ret(v) = &t {
            if let (Some(d), Some(v)) = (dst, v) {
                nb.insts.push(Inst::Copy(d, *v));
            }
            t = Term::Jmp(cont);
        }
        nb.term = t;
        caller.blocks.push(nb);
    }
    // Parameters.
    for (k, p) in callee.params.iter().enumerate() {
        if let ParamLoc::Reg(r) = p {
            let a = args.get(k).copied().unwrap_or(Val::K(0));
            caller.blocks[bi].insts.push(Inst::Copy(mapv(*r), a));
        }
    }
    caller.blocks[bi].term = Term::Jmp(base_b);
}

pub struct InlineStats {
    pub inlined: usize,
}

/// Program-wide inlining pass. `order` is bottom-up (callees first).
pub fn run(funcs: &mut [Option<Func>], order: &[FuncId], keep: &dyn Fn(FuncId) -> bool, is_inline_decl: &dyn Fn(FuncId) -> bool, opt: &dyn Fn(&mut Func)) -> InlineStats {
    let mut stats = InlineStats { inlined: 0 };
    // Count call sites.
    let mut ncalls: HashMap<FuncId, usize> = HashMap::new();
    for f in funcs.iter().flatten() {
        for b in &f.blocks {
            for i in &b.insts {
                if let Inst::Call(_, Callee::Direct(c), _) = i {
                    *ncalls.entry(*c).or_default() += 1;
                }
            }
        }
    }
    for &fid in order {
        let Some(mut f) = funcs[fid].take() else { continue };
        let mut changed = false;
        let mut budget = 200;
        loop {
            let mut site = None;
            'outer: for (bi, b) in f.blocks.iter().enumerate() {
                for (ii, i) in b.insts.iter().enumerate() {
                    if let Inst::Call(_, Callee::Direct(c), args) = i {
                        let c = *c;
                        if c == fid {
                            continue;
                        }
                        let Some(callee) = funcs[c].as_ref() else { continue };
                        if !can_inline(callee) || callee.addr_taken {
                            continue;
                        }
                        let n = ncalls.get(&c).copied().unwrap_or(0);
                        let cc = cost(callee);
                        let single = n == 1 && !keep(c);
                        let consts = args.iter().filter(|a| a.is_const()).count() as u32;
                        let profitable = if single {
                            true
                        } else {
                            // Size model: inlining everywhere vs. one out-of-line copy.
                            let saved_per_site = call_cost(args.len()) + consts;
                            cc <= saved_per_site || (is_inline_decl(c) && cc <= saved_per_site + 2)
                        };
                        if profitable {
                            site = Some((bi, ii, c));
                            break 'outer;
                        }
                    }
                }
            }
            let Some((bi, ii, c)) = site else { break };
            let callee = funcs[c].as_ref().unwrap().clone();
            if let Err(e) = callee.verify() {
                panic!("before inlining (callee): {}", e);
            }
            inline_call(&mut f, bi, ii, &callee);
            if let Err(e) = f.verify() {
                panic!("after inlining {} into {}: {}", callee.name, f.name, e);
            }
            stats.inlined += 1;
            if let Some(n) = ncalls.get_mut(&c) {
                *n = n.saturating_sub(1);
            }
            // Calls inside the inlined body now appear in f.
            for b in &callee.blocks {
                for i in &b.insts {
                    if let Inst::Call(_, Callee::Direct(x), _) = i {
                        *ncalls.entry(*x).or_default() += 1;
                    }
                }
            }
            changed = true;
            budget -= 1;
            if budget == 0 {
                break;
            }
        }
        if changed {
            opt(&mut f);
        }
        funcs[fid] = Some(f);
    }
    stats
}

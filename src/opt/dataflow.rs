//! Dominators, liveness and copy/constant propagation.

use super::bitset::BitSet;
use crate::ir::*;

/// Immediate dominators (entry's idom is itself). Unreachable blocks get u32::MAX.
pub fn dominators(f: &Func) -> Vec<u32> {
    let rpo = f.rpo();
    let n = f.blocks.len();
    let mut order = vec![u32::MAX; n];
    for (i, b) in rpo.iter().enumerate() {
        order[*b as usize] = i as u32;
    }
    let preds = f.preds();
    let mut idom = vec![u32::MAX; n];
    idom[0] = 0;
    let mut changed = true;
    while changed {
        changed = false;
        for &b in rpo.iter().skip(1) {
            let mut new: u32 = u32::MAX;
            for &p in &preds[b as usize] {
                if idom[p as usize] == u32::MAX {
                    continue;
                }
                if new == u32::MAX {
                    new = p;
                } else {
                    // intersect
                    let (mut x, mut y) = (p, new);
                    while x != y {
                        while order[x as usize] > order[y as usize] {
                            x = idom[x as usize];
                        }
                        while order[y as usize] > order[x as usize] {
                            y = idom[y as usize];
                        }
                    }
                    new = x;
                }
            }
            if idom[b as usize] != new {
                idom[b as usize] = new;
                changed = true;
            }
        }
    }
    idom
}

pub fn dominates(idom: &[u32], a: u32, mut b: u32) -> bool {
    loop {
        if a == b {
            return true;
        }
        if b == 0 || idom[b as usize] == u32::MAX {
            return false;
        }
        let p = idom[b as usize];
        if p == b {
            return false;
        }
        b = p;
    }
}

/// For each vreg: true if it has exactly one definition and that definition dominates all uses.
pub fn ssa_vregs(f: &Func) -> Vec<bool> {
    let n = f.vregs.len();
    let idom = dominators(f);
    let mut ndefs = vec![0u32; n];
    // def position: (block, index); params are at (0, -1)
    let mut defpos = vec![(0u32, -1i64); n];
    for p in &f.params {
        if let ParamLoc::Reg(r) = p {
            ndefs[*r as usize] += 1;
        }
    }
    for (bi, b) in f.blocks.iter().enumerate() {
        for (ii, ins) in b.insts.iter().enumerate() {
            if let Some(d) = ins.def() {
                ndefs[d as usize] += 1;
                defpos[d as usize] = (bi as u32, ii as i64);
            }
        }
    }
    let mut ok: Vec<bool> = ndefs.iter().map(|&d| d == 1).collect();
    for (bi, b) in f.blocks.iter().enumerate() {
        if idom[bi] == u32::MAX {
            continue;
        }
        let mut check = |v: VReg, pos: i64, ok: &mut Vec<bool>| {
            let v = v as usize;
            if !ok[v] {
                return;
            }
            let (db, di) = defpos[v];
            if db == bi as u32 {
                if di >= pos {
                    ok[v] = false;
                }
            } else if !dominates(&idom, db, bi as u32) {
                ok[v] = false;
            }
        };
        for (ii, ins) in b.insts.iter().enumerate() {
            for u in ins.uses() {
                check(u, ii as i64, &mut ok);
            }
        }
        for u in b.term.uses() {
            check(u, b.insts.len() as i64, &mut ok);
        }
    }
    ok
}

pub struct Liveness {
    pub live_in: Vec<BitSet>,
    pub live_out: Vec<BitSet>,
}

pub fn liveness(f: &Func) -> Liveness {
    let n = f.vregs.len();
    let nb = f.blocks.len();
    let mut gen_ = vec![BitSet::new(n); nb];
    let mut kill = vec![BitSet::new(n); nb];
    for (bi, b) in f.blocks.iter().enumerate() {
        let (g, k) = (&mut gen_[bi], &mut kill[bi]);
        for ins in &b.insts {
            for u in ins.uses() {
                if !k.contains(u as usize) {
                    g.insert(u as usize);
                }
            }
            if let Some(d) = ins.def() {
                k.insert(d as usize);
            }
        }
        for u in b.term.uses() {
            if !k.contains(u as usize) {
                g.insert(u as usize);
            }
        }
    }
    let mut live_in = vec![BitSet::new(n); nb];
    let mut live_out = vec![BitSet::new(n); nb];
    let rpo = f.rpo();
    let mut changed = true;
    while changed {
        changed = false;
        for &b in rpo.iter().rev() {
            let b = b as usize;
            let mut out = BitSet::new(n);
            for s in f.blocks[b].term.succs() {
                out.union_with(&live_in[s as usize]);
            }
            let mut inn = out.clone();
            inn.subtract(&kill[b]);
            inn.union_with(&gen_[b]);
            if inn != live_in[b] {
                live_in[b] = inn;
                changed = true;
            }
            live_out[b] = out;
        }
    }
    Liveness { live_in, live_out }
}

fn replace_uses(f: &mut Func, from: VReg, to: Val) -> bool {
    let mut changed = false;
    let from_ty = f.ty(from);
    let vregs = f.vregs.clone();
    for b in &mut f.blocks {
        for ins in &mut b.insts {
            // Extension of a constant: fold using the source width.
            if let (Inst::Ext(d, Val::R(r), signed), Val::K(k)) = (&*ins, to) {
                if *r == from {
                    let dt = vregs[*d as usize].ty;
                    let v = if *signed && from_ty != Ty::Bit { dt.norm(from_ty.sext(k)) } else { dt.norm(from_ty.norm(k)) };
                    *ins = Inst::Copy(*d, Val::K(v));
                    changed = true;
                    continue;
                }
            }
            ins.for_each_val_mut(|v| {
                if *v == Val::R(from) {
                    *v = to;
                    changed = true;
                }
            });
            if let Inst::CritExit(r) = ins {
                if *r == from {
                    if let Val::R(t) = to {
                        *r = t;
                        changed = true;
                    }
                }
            }
        }
        b.term.for_each_val_mut(|v| {
            if *v == Val::R(from) {
                *v = to;
                changed = true;
            }
        });
    }
    changed
}

/// Constant and copy propagation.
pub fn propagate(f: &mut Func) -> bool {
    let mut changed = false;
    // 1. SSA-style propagation.
    loop {
        let ssa = ssa_vregs(f);
        let mut work: Vec<(VReg, Val)> = Vec::new();
        for b in &f.blocks {
            for ins in &b.insts {
                if let Inst::Copy(d, v) = ins {
                    if !ssa[*d as usize] {
                        continue;
                    }
                    match v {
                        Val::K(_) | Val::Addr(..) => work.push((*d, *v)),
                        Val::R(x) if ssa[*x as usize] && f.ty(*x) == f.ty(*d) && *x != *d => work.push((*d, *v)),
                        _ => {}
                    }
                }
            }
        }
        if work.is_empty() {
            break;
        }
        let mut did = false;
        for (d, v) in work {
            // A constant used where an address operand is needed (e.g. CritExit) can't be substituted.
            if matches!(v, Val::K(_) | Val::Addr(..)) {
                let used_by_crit = f.blocks.iter().any(|b| b.insts.iter().any(|i| *i == Inst::CritExit(d)));
                if used_by_crit {
                    continue;
                }
            }
            if replace_uses(f, d, v) {
                did = true;
            }
        }
        if !did {
            break;
        }
        changed = true;
        // Constants may need re-normalization for the context width; users normalize when folding.
        break;
    }
    // 2. Local (intra-block) copy propagation for multi-def registers.
    for bi in 0..f.blocks.len() {
        // Active copies: dst -> src
        let mut copies: Vec<(VReg, Val)> = Vec::new();
        let ninst = f.blocks[bi].insts.len();
        for ii in 0..ninst {
            let ins = &mut f.blocks[bi].insts[ii];
            ins.for_each_val_mut(|v| {
                if let Val::R(r) = v {
                    if let Some((_, s)) = copies.iter().find(|c| c.0 == *r) {
                        *v = *s;
                        changed = true;
                    }
                }
            });
            if let Some(d) = ins.def() {
                copies.retain(|(dst, src)| *dst != d && *src != Val::R(d));
                if let Inst::Copy(_, s) = ins {
                    if *s != Val::R(d) {
                        let s = *s;
                        copies.push((d, s));
                    }
                }
            }
            if let Inst::Call(..) | Inst::Asm(..) = ins {
                // Registers aren't clobbered by calls in the IR model; nothing to do.
            }
        }
        let term = &mut f.blocks[bi].term;
        term.for_each_val_mut(|v| {
            if let Val::R(r) = v {
                if let Some((_, s)) = copies.iter().find(|c| c.0 == *r) {
                    *v = *s;
                    changed = true;
                }
            }
        });
    }
    // 3. Coalesce `t = ...; v = copy t` when t is a single-use temp in the same block.
    changed |= coalesce_temp_copies(f);
    changed
}

fn coalesce_temp_copies(f: &mut Func) -> bool {
    let n = f.vregs.len();
    let mut nuses = vec![0u32; n];
    let mut ndefs = vec![0u32; n];
    for p in &f.params {
        if let ParamLoc::Reg(r) = p {
            ndefs[*r as usize] += 1;
        }
    }
    for b in &f.blocks {
        for ins in &b.insts {
            for u in ins.uses() {
                nuses[u as usize] += 1;
            }
            if let Some(d) = ins.def() {
                ndefs[d as usize] += 1;
            }
        }
        for u in b.term.uses() {
            nuses[u as usize] += 1;
        }
    }
    let mut changed = false;
    for b in &mut f.blocks {
        let mut i = 0;
        while i < b.insts.len() {
            if let Inst::Copy(v, Val::R(t)) = b.insts[i] {
                let (v, t) = (v, t);
                if v != t && nuses[t as usize] == 1 && ndefs[t as usize] == 1 && f.vregs[t as usize].ty == f.vregs[v as usize].ty {
                    // Find def of t earlier in this block with no use/def of v in between.
                    let mut j = i;
                    let mut found = None;
                    while j > 0 {
                        j -= 1;
                        let ins = &b.insts[j];
                        if ins.def() == Some(t) {
                            found = Some(j);
                            break;
                        }
                        if ins.def() == Some(v) || ins.uses().contains(&v) {
                            break;
                        }
                    }
                    if let Some(j) = found {
                        // The def itself must not use v (e.g. v = v + 1 is fine: t = v + 1; v = t).
                        b.insts[j].set_def(v);
                        b.insts.remove(i);
                        ndefs[t as usize] = 0;
                        nuses[t as usize] = 0;
                        changed = true;
                        continue;
                    }
                }
            }
            i += 1;
        }
    }
    changed
}

//! Register and frame-slot allocation.

use super::fold::{FoldInfo, FoldKind};
use super::*;
use crate::ir::*;
use crate::opt::bitset::BitSet;
use crate::opt::dataflow;
use crate::types::Space;

pub struct Alloc {
    /// Byte locations per vreg (empty if folded or unused).
    pub locs: Vec<Vec<Loc>>,
    pub nslots: u16,
    pub nbits: u16,
    /// Live vregs after each instruction (non-folded vregs only). live_after[b][i]; index insts.len() = before terminator.
    pub live_after: Vec<Vec<BitSet>>,
    pub live_in: Vec<BitSet>,
    /// Registers assigned to any vreg.
    pub used_regs: RegSet,
}

pub struct AllocCtx<'a> {
    pub f: &'a Func,
    pub fold: &'a FoldInfo,
    pub callee: &'a dyn Fn(&Callee) -> Summary,
    /// Fixed parameter locations (fixed convention), if any.
    pub fixed_params: Option<Vec<Vec<Loc>>>,
    pub ret_locs: Vec<Loc>,
    /// Registers that may not be allocated at all.
    pub reserved: RegSet,
    /// Only give registers to vregs whose weight is at least this.
    pub min_reg_weight: u32,
}

fn loop_depths(f: &Func) -> Vec<u32> {
    let idom = dataflow::dominators(f);
    let n = f.blocks.len();
    let mut depth = vec![0u32; n];
    let preds = f.preds();
    for (b, blk) in f.blocks.iter().enumerate() {
        for s in blk.term.succs() {
            if idom[b] != u32::MAX && dataflow::dominates(&idom, s, b as u32) {
                // Back edge b -> s: natural loop body.
                let mut body = vec![false; n];
                body[s as usize] = true;
                let mut stack = vec![b as u32];
                while let Some(x) = stack.pop() {
                    if body[x as usize] {
                        continue;
                    }
                    body[x as usize] = true;
                    for &p in &preds[x as usize] {
                        stack.push(p);
                    }
                }
                for (i, inb) in body.iter().enumerate() {
                    if *inb {
                        depth[i] += 1;
                    }
                }
            }
        }
    }
    depth
}

/// Clobber sets of instructions that the code generator implements with helpers or scratch registers.
pub fn inst_clobbers(f: &Func, ins: &Inst, callee: &dyn Fn(&Callee) -> Summary) -> RegSet {
    match ins {
        Inst::Call(_, c, _) => callee(c).clobbers,
        Inst::Asm(_) => ALL_REGS,
        Inst::MemCopy(..) | Inst::MemSet(..) => 0x03,
        Inst::Bin(op, d, _, b) => {
            let ty = f.ty(*d);
            match op {
                BinK::Mul if ty != Ty::I8 => helper_clobbers(ty),
                BinK::DivU | BinK::ModU if ty != Ty::I8 => helper_clobbers(ty),
                BinK::DivS | BinK::ModS => helper_clobbers(ty),
                BinK::Shl | BinK::ShrU | BinK::ShrS if ty.bytes() > 2 && !b.is_const() => helper_clobbers(ty),
                _ => 0,
            }
        }
        Inst::Load(_, Mem::Ptr(_, _, PSpace::Generic)) | Inst::Store(Mem::Ptr(_, _, PSpace::Generic), _, _) => 0,
        _ => 0,
    }
}

/// Helper routines take operands in R7..R4 / R3..R0 and return in R7..R4.
pub fn helper_clobbers(_ty: Ty) -> RegSet {
    ALL_REGS
}

pub fn allocate(cx: &AllocCtx) -> Alloc {
    let f = cx.f;
    let n = f.vregs.len();
    let nb = f.blocks.len();
    let folded = |v: VReg| cx.fold.kind[v as usize] != FoldKind::None;

    // --- Liveness at instruction granularity ---
    let live = dataflow::liveness(f);
    let mut live_after: Vec<Vec<BitSet>> = Vec::with_capacity(nb);
    let mut live_in: Vec<BitSet> = Vec::with_capacity(nb);
    let mut interf: Vec<BitSet> = vec![BitSet::new(n); n];
    let mut forbid: Vec<RegSet> = vec![0; n];
    let mut weight: Vec<u32> = vec![0; n];
    let mut ptr_base: Vec<bool> = vec![false; n];
    let mut hints: Vec<Vec<(usize, Loc)>> = vec![Vec::new(); n];
    let mut copy_pairs: Vec<(VReg, VReg)> = Vec::new();
    let depth = loop_depths(f);
    let mut ever_live = BitSet::new(n);

    let add_interf = |interf: &mut Vec<BitSet>, a: usize, b: usize| {
        if a != b {
            interf[a].insert(b);
            interf[b].insert(a);
        }
    };

    for bi in 0..nb {
        let blk = &f.blocks[bi];
        let w = 10u32.pow(depth[bi].min(3)) as u32;
        let mut l = live.live_out[bi].clone();
        // Remove folded vregs from live sets.
        for v in l.clone().iter() {
            if folded(v as VReg) {
                l.remove(v);
            }
        }
        let mut after: Vec<BitSet> = vec![BitSet::new(n); blk.insts.len() + 1];
        after[blk.insts.len()] = l.clone();
        for u in blk.term.uses() {
            weight[u as usize] += w;
            if !folded(u) {
                l.insert(u as usize);
            }
        }
        if let Term::Ret(Some(Val::R(r))) = &blk.term {
            for (k, loc) in cx.ret_locs.iter().enumerate() {
                if loc.is_reg() {
                    hints[*r as usize].push((k, *loc));
                }
            }
        }
        for ii in (0..blk.insts.len()).rev() {
            let ins = &blk.insts[ii];
            after[ii] = l.clone();
            let d = ins.def();
            if let Some(d) = d {
                weight[d as usize] += w;
                if !folded(d) {
                    let except = match ins {
                        Inst::Copy(_, Val::R(s)) => Some(*s as usize),
                        _ => None,
                    };
                    for x in l.iter() {
                        if Some(x) != except {
                            add_interf(&mut interf, d as usize, x);
                        }
                    }
                    l.remove(d as usize);
                    ever_live.insert(d as usize);
                }
            }
            // Clobber constraints for values live across the instruction.
            let clob = inst_clobbers(f, ins, cx.callee);
            if clob != 0 {
                for x in l.iter() {
                    forbid[x] |= clob;
                }
            }
            if let Inst::Call(dst, c, args) = ins {
                let s = (cx.callee)(c);
                for (i, a) in args.iter().enumerate() {
                    if let (Val::R(r), Some(pl)) = (a, s.params.get(i)) {
                        for (k, loc) in pl.iter().enumerate() {
                            if loc.is_reg() {
                                hints[*r as usize].push((k, *loc));
                            }
                        }
                    }
                }
                if let Some(d) = dst {
                    for (k, loc) in s.ret.iter().enumerate() {
                        if loc.is_reg() {
                            hints[*d as usize].push((k, *loc));
                        }
                    }
                }
            }
            if let Inst::Copy(dd, Val::R(s)) = ins {
                copy_pairs.push((*dd, *s));
            }
            // Pointer bases.
            let mut mark_ptr = |m: &Mem| {
                if let Mem::Ptr(Val::R(p), _, PSpace::S(Space::Data | Space::Idata)) = m {
                    ptr_base[*p as usize] = true;
                }
            };
            match ins {
                Inst::Load(_, m) | Inst::Store(m, _, _) => mark_ptr(m),
                Inst::MemCopy(a, b, _) => {
                    mark_ptr(a);
                    mark_ptr(b)
                }
                Inst::MemSet(a, _, _) => mark_ptr(a),
                _ => {}
            }
            for u in ins.uses() {
                weight[u as usize] += w;
                if !folded(u) {
                    l.insert(u as usize);
                    ever_live.insert(u as usize);
                }
            }
        }
        live_in.push(l);
        live_after.push(after);
    }
    // Parameters are defined at entry: they interfere with everything live at entry and each other.
    let param_regs: Vec<VReg> = f.params.iter().filter_map(|p| if let ParamLoc::Reg(r) = p { Some(*r) } else { None }).collect();
    for &p in &param_regs {
        ever_live.insert(p as usize);
        for x in live_in[0].iter() {
            add_interf(&mut interf, p as usize, x);
        }
        for &q in &param_regs {
            add_interf(&mut interf, p as usize, q as usize);
        }
    }
    let mut partners: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in &copy_pairs {
        if !interf[a as usize].contains(b as usize) {
            let (a, b) = (a as usize, b as usize);
            partners[a].push(b);
            partners[b].push(a);
        }
    }

    // --- Assignment ---
    let mut locs: Vec<Vec<Loc>> = vec![Vec::new(); n];
    let mut order: Vec<usize> = (0..n).filter(|&v| !folded(v as VReg) && ever_live.contains(v)).collect();
    order.sort_by(|&a, &b| weight[b].cmp(&weight[a]).then(a.cmp(&b)));

    // Fixed parameters.
    let mut fixed = vec![false; n];
    if let Some(fp) = &cx.fixed_params {
        for (i, p) in f.params.iter().enumerate() {
            if let ParamLoc::Reg(r) = p {
                if let Some(pl) = fp.get(i) {
                    locs[*r as usize] = pl.clone();
                    fixed[*r as usize] = true;
                }
            }
        }
    }

    let mut used_regs: RegSet = 0;
    for &v in &order {
        if fixed[v] {
            for l in &locs[v] {
                if let Loc::R(r) = l {
                    used_regs |= 1 << r;
                }
            }
            continue;
        }
        let ty = f.vregs[v].ty;
        if ty == Ty::Bit {
            continue;
        }
        // Registers taken by interfering vregs.
        let mut taken: RegSet = forbid[v] | cx.reserved;
        for x in interf[v].iter() {
            for l in &locs[x] {
                if let Loc::R(r) = l {
                    taken |= 1 << r;
                }
            }
        }
        if weight[v] < cx.min_reg_weight {
            continue;
        }
        let nbytes = ty.bytes() as usize;
        let mut assigned: Vec<Option<Loc>> = vec![None; nbytes];
        for k in 0..nbytes {
            let mut prefs: Vec<u8> = Vec::new();
            for &p in &partners[v] {
                if let Some(Loc::R(r)) = locs[p].get(k) {
                    prefs.push(*r);
                }
            }
            for (hk, hl) in &hints[v] {
                if let Loc::R(r) = hl {
                    if *hk == k {
                        prefs.push(*r);
                    }
                }
            }
            let default: &[u8] = if ptr_base[v] && k == 0 { &[0, 1, 7, 6, 5, 4, 3, 2] } else { &[7, 6, 5, 4, 3, 2, 1, 0] };
            prefs.extend_from_slice(default);
            for r in prefs {
                if taken & (1 << r) == 0 {
                    assigned[k] = Some(Loc::R(r));
                    taken |= 1 << r;
                    break;
                }
            }
        }
        locs[v] = assigned.iter().map(|a| a.unwrap_or(Loc::Slot(f.id, u16::MAX))).collect();
        for l in &locs[v] {
            if let Loc::R(r) = l {
                used_regs |= 1 << r;
            }
        }
    }

    // --- Memory slots for the remaining bytes ---
    let mut slot_users: Vec<Vec<usize>> = Vec::new();
    let mut bit_users: Vec<Vec<usize>> = Vec::new();
    // Fixed params in memory already carry concrete slots.
    for &v in &order {
        let ty = f.vregs[v].ty;
        if ty == Ty::Bit {
            if fixed[v] {
                continue;
            }
            let mut s = 0usize;
            loop {
                if s == bit_users.len() {
                    bit_users.push(Vec::new());
                }
                if bit_users[s].iter().all(|&u| !interf[v].contains(u)) {
                    bit_users[s].push(v);
                    locs[v] = vec![Loc::BitSlot(f.id, s as u16)];
                    break;
                }
                s += 1;
            }
            continue;
        }
        if locs[v].is_empty() {
            locs[v] = vec![Loc::Slot(f.id, u16::MAX); ty.bytes() as usize];
        }
        for k in 0..locs[v].len() {
            match locs[v][k] {
                Loc::Slot(_, u16::MAX) => {}
                Loc::Slot(_, s) => {
                    while slot_users.len() <= s as usize {
                        slot_users.push(Vec::new());
                    }
                    slot_users[s as usize].push(v);
                    continue;
                }
                _ => continue,
            }
            let mut s = 0usize;
            loop {
                if s == slot_users.len() {
                    slot_users.push(Vec::new());
                }
                let ok = slot_users[s].iter().all(|&u| u != v && !interf[v].contains(u));
                if ok {
                    slot_users[s].push(v);
                    locs[v][k] = Loc::Slot(f.id, s as u16);
                    break;
                }
                s += 1;
            }
        }
    }
    Alloc { locs, nslots: slot_users.len() as u16, nbits: bit_users.len() as u16, live_after, live_in, used_regs }
}

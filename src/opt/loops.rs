//! Loop rewrites.
//!
//! A counted loop whose latch compares the counter with `<` against a constant can compare with
//! `!=` instead: the counter steps by one from a smaller start, so it meets the bound exactly.
//! On this target that turns a `cjne` plus `jc` into a single `cjne`.

use super::dataflow::{dominates, dominators};
use crate::ir::*;

/// Blocks of the natural loop of the back edge `latch -> header`.
fn loop_blocks(f: &Func, preds: &[Vec<BlockId>], header: BlockId, latch: BlockId) -> Vec<BlockId> {
    let mut body = vec![header];
    let mut work = Vec::new();
    if latch != header {
        body.push(latch);
        work.push(latch);
    }
    // Walk back from the latch, stopping at the header.
    while let Some(b) = work.pop() {
        for &p in &preds[b as usize] {
            if !body.contains(&p) {
                body.push(p);
                work.push(p);
            }
        }
    }
    body
}

/// True if `v` counts up by one per iteration from a start below `k`. The increment must sit in
/// the latch itself, so that the comparison sees every value the counter takes.
fn counts_to(f: &Func, body: &[BlockId], latch: BlockId, v: VReg, k: i64, signed: bool) -> bool {
    let ty = f.ty(v);
    let mut steps = 0;
    for (bi, b) in f.blocks.iter().enumerate() {
        let inside = body.contains(&(bi as u32));
        for ins in &b.insts {
            if ins.def() != Some(v) {
                continue;
            }
            if inside {
                // The only definition inside the loop must be `v = v + 1` in the latch.
                if bi as u32 != latch {
                    return false;
                }
                if !matches!(ins, Inst::Bin(BinK::Add, d, Val::R(s), Val::K(1)) if *d == v && *s == v) {
                    return false;
                }
                steps += 1;
            } else {
                // Every definition outside must start the counter below the bound.
                let Inst::Copy(_, Val::K(c)) = ins else { return false };
                let below = if signed { ty.sext(*c) < ty.sext(k) } else { (ty.norm(*c) as u64) < (ty.norm(k) as u64) };
                if !below {
                    return false;
                }
            }
        }
    }
    // A parameter is not a known starting value.
    if f.params.iter().any(|p| matches!(p, ParamLoc::Reg(r) if *r == v)) {
        return false;
    }
    steps == 1
}

pub fn run(f: &mut Func) -> bool {
    let idom = dominators(f);
    let preds = f.preds();
    let mut changed = false;
    for bi in 0..f.blocks.len() {
        let Term::CmpBr(cond, a, b, ty, t, e) = f.blocks[bi].term else { continue };
        let (Val::R(v), Val::K(k)) = (a, b) else { continue };
        let _ = ty;
        // Which edge goes back to a block that dominates this one?
        let (header, signed, taken_loops) = match cond {
            Cond::LtU if dominates(&idom, t, bi as u32) => (t, false, true),
            Cond::LtS if dominates(&idom, t, bi as u32) => (t, true, true),
            Cond::GeU if dominates(&idom, e, bi as u32) => (e, false, false),
            Cond::GeS if dominates(&idom, e, bi as u32) => (e, true, false),
            _ => continue,
        };
        if header == bi as u32 && f.blocks[bi].insts.is_empty() {
            continue;
        }
        let body = loop_blocks(f, &preds, header, bi as u32);
        // A single way in, so the counter always starts from one of its known values.
        if preds[header as usize].iter().filter(|p| !body.contains(p)).count() != 1 {
            continue;
        }
        if !counts_to(f, &body, bi as u32, v, k, signed) {
            continue;
        }
        let new = if taken_loops { Cond::Ne } else { Cond::Eq };
        f.blocks[bi].term = Term::CmpBr(new, a, b, ty, t, e);
        changed = true;
    }
    changed
}

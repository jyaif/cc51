//! Sinking: move a computation into the successor that needs it.
//!
//! A value computed before a branch but used on only one side costs a register (and often a copy)
//! on the other. Moving it into that successor keeps it out of the paths that ignore it.

use super::dataflow::liveness;
use crate::ir::*;

/// Instructions worth moving: pure, cheap and independent of memory.
fn movable(ins: &Inst) -> bool {
    matches!(ins, Inst::Copy(..) | Inst::Bin(..) | Inst::Un(..) | Inst::Ext(..) | Inst::Trunc(..) | Inst::Cmp(..))
}

pub fn run(f: &mut Func) -> bool {
    let preds = f.preds();
    let live = liveness(f);
    let mut changed = false;
    for bi in 0..f.blocks.len() {
        let succs = f.blocks[bi].term.succs();
        if succs.len() < 2 {
            continue;
        }
        let mut moves: Vec<(usize, BlockId)> = Vec::new();
        for ii in (0..f.blocks[bi].insts.len()).rev() {
            let ins = f.blocks[bi].insts[ii].clone();
            if !movable(&ins) {
                continue;
            }
            let Some(d) = ins.def() else { continue };
            // The definition must not be read again in this block, and must reach exactly one
            // successor that no other block enters.
            if f.blocks[bi].insts[ii + 1..].iter().any(|x| x.uses().contains(&d)) || f.blocks[bi].term.uses().contains(&d) {
                continue;
            }
            if moves.iter().any(|(mi, _)| f.blocks[bi].insts[*mi].uses().contains(&d)) {
                continue;
            }
            let wanted: Vec<BlockId> = succs.iter().copied().filter(|s| live.live_in[*s as usize].contains(d as usize)).collect();
            if wanted.len() != 1 {
                continue;
            }
            let s = wanted[0];
            if preds[s as usize].len() != 1 || s == bi as u32 {
                continue;
            }
            // Its operands must still hold the same values where it lands.
            let ops = ins.uses();
            let later = f.blocks[bi].insts[ii + 1..].iter().filter_map(|x| x.def()).collect::<Vec<_>>();
            if ops.iter().any(|u| later.contains(u) || *u == d) {
                continue;
            }
            // The successor must not redefine them before the moved instruction runs: it lands
            // first, so only the instruction itself matters.
            moves.push((ii, s));
        }
        for (ii, s) in moves {
            let ins = f.blocks[bi].insts.remove(ii);
            f.blocks[s as usize].insts.insert(0, ins);
            changed = true;
        }
    }
    changed
}

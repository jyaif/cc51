//! Dead code elimination.

use super::dataflow::liveness;
use crate::ir::*;

pub fn run(f: &mut Func) -> bool {
    let mut changed = false;
    loop {
        let live = liveness(f);
        let mut c = false;
        for (bi, b) in f.blocks.iter_mut().enumerate() {
            let mut l = live.live_out[bi].clone();
            for u in b.term.uses() {
                l.insert(u as usize);
            }
            let mut keep = vec![true; b.insts.len()];
            for (ii, ins) in b.insts.iter().enumerate().rev() {
                let d = ins.def();
                let dead = match d {
                    Some(d) => !l.contains(d as usize),
                    None => false,
                };
                if dead && !ins.has_side_effects() {
                    keep[ii] = false;
                    c = true;
                    continue;
                }
                if matches!(ins, Inst::Nop) {
                    keep[ii] = false;
                    c = true;
                    continue;
                }
                if let Inst::Copy(d, Val::R(s)) = ins {
                    if d == s {
                        keep[ii] = false;
                        c = true;
                        continue;
                    }
                }
                if let Some(d) = d {
                    l.remove(d as usize);
                }
                for u in ins.uses() {
                    l.insert(u as usize);
                }
            }
            let mut it = keep.iter();
            b.insts.retain(|_| *it.next().unwrap());
        }
        // Calls whose result is dead: drop the result.
        let live = liveness(f);
        for (bi, b) in f.blocks.iter_mut().enumerate() {
            let mut l = live.live_out[bi].clone();
            for u in b.term.uses() {
                l.insert(u as usize);
            }
            for ins in b.insts.iter_mut().rev() {
                if let Inst::Call(d @ Some(_), _, _) = ins {
                    if !l.contains(d.unwrap() as usize) {
                        *d = None;
                        c = true;
                    }
                }
                if let Some(d) = ins.def() {
                    l.remove(d as usize);
                }
                for u in ins.uses() {
                    l.insert(u as usize);
                }
            }
        }
        if !c {
            break;
        }
        changed = true;
    }
    changed
}

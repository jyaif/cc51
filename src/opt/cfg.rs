//! Control-flow graph simplification.

use crate::ir::*;

fn fold_terms(f: &mut Func) -> bool {
    let mut changed = false;
    for b in &mut f.blocks {
        let new = match &b.term {
            Term::Br(Val::K(k), t, e) => Some(Term::Jmp(if *k != 0 { *t } else { *e })),
            Term::Br(Val::Addr(..), t, _) => Some(Term::Jmp(*t)),
            Term::Br(_, t, e) if t == e => Some(Term::Jmp(*t)),
            Term::CmpBr(_, _, _, _, t, e) if t == e => Some(Term::Jmp(*t)),
            Term::CmpBr(c, Val::K(a), Val::K(bb), ty, t, e) => Some(Term::Jmp(if c.eval(*a, *bb, *ty) { *t } else { *e })),
            Term::CmpBr(c, a, bb, _, t, e) if a == bb && a.reg().is_some() => {
                let r = matches!(c, Cond::Eq | Cond::LeS | Cond::LeU | Cond::GeS | Cond::GeU);
                Some(Term::Jmp(if r { *t } else { *e }))
            }
            // Unsigned comparisons with trivial constants.
            Term::CmpBr(Cond::GeU, _, Val::K(0), _, t, _) => Some(Term::Jmp(*t)),
            Term::CmpBr(Cond::LtU, _, Val::K(0), _, _, e) => Some(Term::Jmp(*e)),
            Term::Switch(Val::K(k), ty, cases, d) => {
                let k = ty.norm(*k);
                Some(Term::Jmp(cases.iter().find(|c| c.0 == k).map(|c| c.1).unwrap_or(*d)))
            }
            Term::Switch(_, _, cases, d) if cases.iter().all(|c| c.1 == *d) => Some(Term::Jmp(*d)),
            _ => None,
        };
        if let Some(t) = new {
            b.term = t;
            changed = true;
        }
        // Remove switch cases that go to the default.
        if let Term::Switch(_, _, cases, d) = &mut b.term {
            let d = *d;
            let n = cases.len();
            cases.retain(|c| c.1 != d);
            if cases.len() != n {
                changed = true;
            }
        }
    }
    changed
}

/// Remove unreachable blocks and renumber.
pub fn remove_unreachable(f: &mut Func) -> bool {
    let n = f.blocks.len();
    let mut reach = vec![false; n];
    let mut stack = vec![0u32];
    reach[0] = true;
    while let Some(b) = stack.pop() {
        for s in f.blocks[b as usize].term.succs() {
            if !reach[s as usize] {
                reach[s as usize] = true;
                stack.push(s);
            }
        }
    }
    if reach.iter().all(|r| *r) {
        return false;
    }
    let mut map = vec![u32::MAX; n];
    let mut new_blocks = Vec::new();
    for (i, b) in std::mem::take(&mut f.blocks).into_iter().enumerate() {
        if reach[i] {
            map[i] = new_blocks.len() as u32;
            new_blocks.push(b);
        }
    }
    for b in &mut new_blocks {
        for s in b.term.succs_mut() {
            *s = map[*s as usize];
        }
    }
    f.blocks = new_blocks;
    true
}

fn thread_jumps(f: &mut Func) -> bool {
    let n = f.blocks.len();
    // Resolve forwarding targets for empty blocks ending in Jmp.
    let mut fwd: Vec<u32> = (0..n as u32).collect();
    for i in 1..n {
        let b = &f.blocks[i];
        if b.insts.is_empty() {
            if let Term::Jmp(t) = b.term {
                if t as usize != i {
                    fwd[i] = t;
                }
            }
        }
    }
    let resolve = |orig: u32, fwd: &Vec<u32>| -> u32 {
        let mut b = orig;
        let mut steps = 0;
        while fwd[b as usize] != b && steps <= n {
            b = fwd[b as usize];
            steps += 1;
        }
        if steps > n {
            // Cycle of empty blocks (infinite loop): leave as is.
            return orig;
        }
        b
    };
    let mut changed = false;
    for i in 0..n {
        let mut t = f.blocks[i].term.clone();
        for s in t.succs_mut() {
            let r = resolve(*s, &fwd);
            if r != *s {
                *s = r;
                changed = true;
            }
        }
        f.blocks[i].term = t;
    }
    // Jump to a block that only returns: duplicate the return (it's a single instruction).
    for i in 0..n {
        if let Term::Jmp(t) = f.blocks[i].term {
            let tb = &f.blocks[t as usize];
            if tb.insts.is_empty() {
                if let Term::Ret(v) = &tb.term {
                    if f.attrs.interrupt.is_none() && !f.attrs.naked && !f.attrs.critical {
                        f.blocks[i].term = Term::Ret(*v);
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

fn merge_blocks(f: &mut Func) -> bool {
    let mut changed = false;
    loop {
        let preds = f.preds();
        let mut merged = false;
        for i in 0..f.blocks.len() {
            let Term::Jmp(t) = f.blocks[i].term else { continue };
            let t = t as usize;
            if t == i || t == 0 || preds[t].len() != 1 {
                continue;
            }
            let tb = std::mem::replace(&mut f.blocks[t], Block { insts: vec![], term: Term::Unreachable });
            let b = &mut f.blocks[i];
            b.insts.extend(tb.insts);
            b.term = tb.term;
            merged = true;
            changed = true;
            break;
        }
        if !merged {
            break;
        }
        remove_unreachable(f);
    }
    changed
}

/// Thread jumps into blocks that only test values known at the end of the predecessor.
fn thread_known(f: &mut Func) -> bool {
    let mut changed = false;
    for p in 0..f.blocks.len() {
        let Term::Jmp(h) = f.blocks[p].term else { continue };
        let hb = &f.blocks[h as usize];
        if !hb.insts.is_empty() || h as usize == p {
            continue;
        }
        // Constant value of a vreg at the end of block p (last definition in p).
        let known = |v: Val| -> Option<i64> {
            match v {
                Val::K(k) => Some(k),
                Val::R(r) => {
                    for ins in f.blocks[p].insts.iter().rev() {
                        if ins.def() == Some(r) {
                            return match ins {
                                Inst::Copy(_, Val::K(k)) => Some(*k),
                                _ => None,
                            };
                        }
                    }
                    None
                }
                _ => None,
            }
        };
        let target = match &hb.term {
            Term::CmpBr(c, a, b, ty, t, e) => match (known(*a), known(*b)) {
                (Some(x), Some(y)) => Some(if c.eval(x, y, *ty) { *t } else { *e }),
                _ => None,
            },
            Term::Br(v, t, e) => known(*v).map(|x| if x != 0 { *t } else { *e }),
            _ => None,
        };
        if let Some(t) = target {
            if t != h {
                f.blocks[p].term = Term::Jmp(t);
                changed = true;
            }
        }
    }
    changed
}

pub fn simplify(f: &mut Func) -> bool {
    let mut changed = false;
    loop {
        let mut c = fold_terms(f);
        c |= thread_jumps(f);
        c |= thread_known(f);
        c |= remove_unreachable(f);
        c |= merge_blocks(f);
        if !c {
            break;
        }
        changed = true;
    }
    changed
}

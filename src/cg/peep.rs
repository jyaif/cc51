//! Assembly-level peephole optimizations: dead-store elimination on machine resources,
//! jump threading and tail calls.

use crate::asm::{insn_size, Expr, Insn, Item, Mn, Op};
use std::collections::HashMap;
use std::rc::Rc;

pub type Res = u32;
pub const R_A: Res = 1 << 8;
pub const R_B: Res = 1 << 9;
pub const R_C: Res = 1 << 10;
pub const R_DPL: Res = 1 << 11;
pub const R_DPH: Res = 1 << 12;
pub const R_OV: Res = 1 << 13;
pub const R_DPTR: Res = R_DPL | R_DPH;
pub const R_ALL: Res = 0x3fff;

pub fn reg_res(n: u8) -> Res {
    1 << n
}

pub struct PeepCtx<'a> {
    pub bank: u8,
    /// Resources used by a call to the named function (parameters).
    pub call_uses: &'a dyn Fn(&str) -> Res,
    /// Resources the function's return uses.
    pub ret_uses: Res,
    pub is_isr: bool,
}

#[derive(Clone, Copy)]
struct Eff {
    uses: Res,
    defs: Res,
    /// Can be deleted if all defs are dead.
    pure_: bool,
}

fn dir_res(e: &Expr, bank: u8) -> Option<Res> {
    let v = e.const_val()?;
    let b = bank as i64 * 8;
    if v >= b && v < b + 8 {
        return Some(reg_res((v - b) as u8));
    }
    match v {
        0xE0 => Some(R_A),
        0xF0 => Some(R_B),
        0x82 => Some(R_DPL),
        0x83 => Some(R_DPH),
        0xD0 => Some(R_C | R_OV),
        _ => None,
    }
}

fn bit_res(e: &Expr) -> Option<Res> {
    let v = e.const_val()?;
    match v & !7 {
        0xE0 => Some(R_A),
        0xF0 => Some(R_B),
        0xD0 if v == 0xD7 => Some(R_C),
        0xD0 if v == 0xD2 => Some(R_OV),
        _ => None,
    }
}

/// Resource read by an operand used as a source.
fn op_use(o: &Op, bank: u8) -> Res {
    match o {
        Op::A => R_A,
        Op::AB => R_A | R_B,
        Op::C => R_C,
        Op::Dptr => R_DPTR,
        Op::R(n) => reg_res(*n),
        Op::AtR(n) => reg_res(*n),
        Op::AtDptr => R_DPTR,
        Op::AtADptr => R_A | R_DPTR,
        Op::AtAPc => R_A,
        Op::Dir(e) => dir_res(e, bank).unwrap_or(0),
        Op::Bit(e) => bit_res(e).unwrap_or(0),
        _ => 0,
    }
}

/// Destination written by an operand: (resource, is_memory_or_unknown).
fn op_def(o: &Op, bank: u8) -> (Res, bool) {
    match o {
        Op::A => (R_A, false),
        Op::C => (R_C, false),
        Op::Dptr => (R_DPTR, false),
        Op::R(n) => (reg_res(*n), false),
        Op::AtR(_) | Op::AtDptr => (0, true),
        Op::Dir(e) => match dir_res(e, bank) {
            Some(r) => (r, false),
            None => (0, true),
        },
        Op::Bit(e) => match bit_res(e) {
            Some(r) => (r, false),
            None => (0, true),
        },
        _ => (0, true),
    }
}

fn effect(i: &Insn, cx: &PeepCtx) -> Eff {
    let b = cx.bank;
    let o = &i.ops;
    let u = |k: usize| o.get(k).map_or(0, |x| op_use(x, b));
    match i.mn {
        Mn::Mov => {
            let (d, mem) = op_def(&o[0], b);
            let mut uses = u(1);
            if let Op::AtR(n) = o[0] {
                uses |= reg_res(n);
            }
            // Writing one bit of a byte keeps the other bits.
            if matches!(o[0], Op::Bit(_)) {
                uses |= d;
            }
            Eff { uses, defs: d, pure_: !mem }
        }
        Mn::Movc => Eff { uses: u(1), defs: R_A, pure_: true },
        Mn::Movx => {
            if o[0] == Op::A {
                Eff { uses: u(1), defs: R_A, pure_: false }
            } else {
                Eff { uses: u(0) | R_A, defs: 0, pure_: false }
            }
        }
        Mn::Add | Mn::Addc | Mn::Subb => {
            let mut uses = R_A | u(1);
            if i.mn != Mn::Add {
                uses |= R_C;
            }
            Eff { uses, defs: R_A | R_C | R_OV, pure_: true }
        }
        Mn::Anl | Mn::Orl | Mn::Xrl => {
            let (d, mem) = op_def(&o[0], b);
            Eff { uses: op_use(&o[0], b) | u(1), defs: d, pure_: !mem }
        }
        Mn::Inc | Mn::Dec => {
            let (d, mem) = op_def(&o[0], b);
            let mut uses = op_use(&o[0], b);
            if let Op::AtR(n) = o[0] {
                uses |= reg_res(n);
            }
            Eff { uses, defs: d, pure_: !mem }
        }
        Mn::Clr | Mn::Setb => {
            let (d, mem) = op_def(&o[0], b);
            let uses = if matches!(o[0], Op::Bit(_)) { d } else { 0 };
            Eff { uses, defs: d, pure_: !mem }
        }
        Mn::Cpl => {
            let (d, mem) = op_def(&o[0], b);
            Eff { uses: d, defs: d, pure_: !mem }
        }
        Mn::Rl | Mn::Rr | Mn::Swap => Eff { uses: R_A, defs: R_A, pure_: true },
        Mn::Rlc | Mn::Rrc => Eff { uses: R_A | R_C, defs: R_A | R_C, pure_: true },
        Mn::Da => Eff { uses: R_A | R_C, defs: R_A | R_C, pure_: true },
        Mn::Mul | Mn::Div => Eff { uses: R_A | R_B, defs: R_A | R_B | R_C | R_OV, pure_: true },
        Mn::Xch => {
            let (d, mem) = op_def(&o[1], b);
            let mut uses = R_A | op_use(&o[1], b);
            if let Op::AtR(n) = o[1] {
                uses |= reg_res(n);
            }
            Eff { uses, defs: R_A | d, pure_: !mem }
        }
        Mn::Xchd => Eff { uses: R_A | u(1), defs: R_A, pure_: false },
        Mn::Push => Eff { uses: u(0), defs: 0, pure_: false },
        Mn::Pop => {
            let (d, _) = op_def(&o[0], b);
            Eff { uses: 0, defs: d, pure_: false }
        }
        Mn::Nop => Eff { uses: 0, defs: 0, pure_: false },
        Mn::Jz | Mn::Jnz => Eff { uses: R_A, defs: 0, pure_: false },
        Mn::Jc | Mn::Jnc => Eff { uses: R_C, defs: 0, pure_: false },
        Mn::Jb | Mn::Jnb => Eff { uses: u(0), defs: 0, pure_: false },
        Mn::Jbc => {
            let (d, _) = op_def(&o[0], b);
            Eff { uses: u(0), defs: d, pure_: false }
        }
        Mn::Cjne => {
            let mut uses = u(0) | u(1);
            if let Op::AtR(n) = o[0] {
                uses |= reg_res(n);
            }
            Eff { uses, defs: R_C, pure_: false }
        }
        Mn::Djnz => {
            let (d, _) = op_def(&o[0], b);
            Eff { uses: u(0), defs: d, pure_: false }
        }
        Mn::Call | Mn::Acall | Mn::Lcall => {
            let name = match i.target().and_then(|t| t.sym.clone()) {
                Some(s) => s,
                None => return Eff { uses: R_ALL, defs: 0, pure_: false },
            };
            Eff { uses: (cx.call_uses)(&name), defs: 0, pure_: false }
        }
        Mn::Jmp | Mn::Sjmp | Mn::Ajmp | Mn::Ljmp => Eff { uses: if matches!(o.first(), Some(Op::AtADptr)) { R_ALL } else { 0 }, defs: 0, pure_: false },
        Mn::Ret => Eff { uses: cx.ret_uses, defs: 0, pure_: false },
        Mn::Reti => Eff { uses: R_ALL, defs: 0, pure_: false },
    }
}

fn is_uncond_jump(i: &Insn) -> bool {
    matches!(i.mn, Mn::Sjmp | Mn::Ajmp | Mn::Ljmp) || (i.mn == Mn::Jmp && matches!(i.ops.first(), Some(Op::Code(_))))
}

fn is_cond_jump(i: &Insn) -> bool {
    i.mn.is_cond_jump()
}

fn target_label(i: &Insn) -> Option<Rc<str>> {
    let t = i.target()?;
    if t.off != 0 {
        return None;
    }
    t.sym.clone()
}

/// Dead store elimination. Returns true if anything changed.
/// Live machine resources before each item (`live[n]` is the end of the block list).
fn liveness(items: &[Item], cx: &PeepCtx, effs: &[Option<Eff>]) -> Vec<Res> {
    let n = items.len();
    let mut labels: HashMap<Rc<str>, usize> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        if let Item::Label(l) = it {
            labels.insert(l.clone(), i);
        }
    }
    let mut live_in = vec![0 as Res; n + 1];
    // live at end: unknown fall-off (shouldn't happen) -> all.
    live_in[n] = R_ALL;
    let mut changed = true;
    let mut iters = 0;
    while changed && iters < 50 {
        changed = false;
        iters += 1;
        for i in (0..n).rev() {
            let out: Res = match &items[i] {
                Item::Insn(ins) => {
                    let mut out = 0;
                    let fall = !(is_uncond_jump(ins) || matches!(ins.mn, Mn::Ret | Mn::Reti) || (ins.mn == Mn::Jmp && ins.ops.first() == Some(&Op::AtADptr)));
                    if fall {
                        out |= live_in[i + 1];
                    }
                    if is_uncond_jump(ins) || is_cond_jump(ins) {
                        match target_label(ins).and_then(|l| labels.get(&l).copied()) {
                            Some(t) => out |= live_in[t],
                            None => out |= R_ALL,
                        }
                    }
                    out
                }
                Item::Db(_) | Item::Dw(_) | Item::Ds(_) => R_ALL,
                _ => live_in[i + 1],
            };
            let inn = match &effs[i] {
                Some(e) => (out & !e.defs) | e.uses,
                None => out,
            };
            // Conditional defs (djnz/jbc) and bit writes are already modelled as uses.
            if inn != live_in[i] {
                live_in[i] = inn;
                changed = true;
            }
        }
    }
    live_in
}

fn effects(items: &[Item], cx: &PeepCtx) -> Vec<Option<Eff>> {
    items
        .iter()
        .map(|it| match it {
            Item::Insn(i) => Some(effect(i, cx)),
            _ => None,
        })
        .collect()
}

fn dse(items: &mut Vec<Item>, cx: &PeepCtx) -> bool {
    let n = items.len();
    let effs = effects(items, cx);
    let live_in = liveness(items, cx, &effs);
    // Delete dead pure instructions.
    let mut removed = false;
    let mut keep = vec![true; n];
    for i in 0..n {
        if let (Item::Insn(ins), Some(e)) = (&items[i], &effs[i]) {
            if !e.pure_ || e.defs == 0 {
                continue;
            }
            let live_after = if is_uncond_jump(ins) { 0 } else { live_in[i + 1] };
            if e.defs & live_after == 0 {
                keep[i] = false;
                removed = true;
            }
        }
    }
    if removed {
        let mut k = keep.iter();
        items.retain(|_| *k.next().unwrap());
    }
    removed
}

/// Jump-level simplifications.
fn jumps(items: &mut Vec<Item>, cx: &PeepCtx) -> bool {
    let mut changed = false;
    // Map label -> index of the first instruction at or after it.
    let first_insn_after = |items: &Vec<Item>, mut i: usize| -> Option<usize> {
        while i < items.len() {
            if let Item::Insn(_) = items[i] {
                return Some(i);
            }
            if matches!(items[i], Item::Db(_) | Item::Dw(_) | Item::Ds(_)) {
                return None;
            }
            i += 1;
        }
        None
    };
    let label_pos = |items: &Vec<Item>| -> HashMap<Rc<str>, usize> {
        items.iter().enumerate().filter_map(|(i, it)| if let Item::Label(l) = it { Some((l.clone(), i)) } else { None }).collect()
    };
    let lp = label_pos(items);
    let n = items.len();
    for i in 0..n {
        let Item::Insn(ins) = &items[i] else { continue };
        if !(is_uncond_jump(ins) || is_cond_jump(ins)) {
            continue;
        }
        // Thread through unconditional jumps.
        let mut tgt = match target_label(ins) {
            Some(t) => t,
            None => continue,
        };
        let mut hops = 0;
        let mut final_ret = false;
        while let Some(&p) = lp.get(&tgt) {
            let Some(j) = first_insn_after(items, p) else { break };
            let Item::Insn(tj) = &items[j] else { break };
            if is_uncond_jump(tj) {
                if let Some(t2) = target_label(tj) {
                    if t2 == tgt {
                        break;
                    }
                    tgt = t2;
                    hops += 1;
                    if hops > 10 {
                        break;
                    }
                    continue;
                }
            }
            if tj.mn == Mn::Ret && is_uncond_jump(ins) && !cx.is_isr {
                final_ret = true;
            }
            break;
        }
        if final_ret {
            items[i] = Item::Insn(Insn::new(Mn::Ret, vec![]));
            changed = true;
            continue;
        }
        if hops > 0 {
            if let Item::Insn(ins) = &mut items[i] {
                *ins.target_mut().unwrap() = Expr::sym(&tgt);
                changed = true;
            }
        }
    }
    // Jump to the immediately following label.
    let lp = label_pos(items);
    let mut keep = vec![true; items.len()];
    for i in 0..items.len() {
        let Item::Insn(ins) = &items[i] else { continue };
        if !is_uncond_jump(ins) {
            continue;
        }
        let Some(t) = target_label(ins) else { continue };
        let Some(&p) = lp.get(&t) else { continue };
        if p > i && (i + 1..p).all(|k| matches!(items[k], Item::Label(_) | Item::Comment(_)) || !keep[k]) {
            keep[i] = false;
            changed = true;
        }
    }
    // Conditional jump over an unconditional jump: jcc L1; jmp L2; L1:  ->  j!cc L2; L1:
    for i in 0..items.len() {
        if !keep[i] {
            continue;
        }
        let Item::Insn(ins) = &items[i] else { continue };
        let inv = match ins.mn {
            Mn::Jz => Mn::Jnz,
            Mn::Jnz => Mn::Jz,
            Mn::Jc => Mn::Jnc,
            Mn::Jnc => Mn::Jc,
            Mn::Jb => Mn::Jnb,
            Mn::Jnb => Mn::Jb,
            _ => continue,
        };
        let Some(t1) = target_label(ins) else { continue };
        let mut j = i + 1;
        while j < items.len() && !keep[j] {
            j += 1;
        }
        let Some(Item::Insn(jmp)) = items.get(j) else { continue };
        if !is_uncond_jump(jmp) {
            continue;
        }
        let Some(t2) = target_label(jmp) else { continue };
        let Some(&p1) = lp.get(&t1) else { continue };
        if p1 > j && (j + 1..p1).all(|k| matches!(items[k], Item::Label(_) | Item::Comment(_)) || !keep[k]) {
            let mut new = ins.clone();
            new.mn = inv;
            *new.target_mut().unwrap() = Expr::sym(&t2);
            items[i] = Item::Insn(new);
            keep[j] = false;
            changed = true;
        }
    }
    // Tail calls: call f; ret -> jmp f
    if !cx.is_isr {
        for i in 0..items.len() {
            if !keep[i] {
                continue;
            }
            let Item::Insn(ins) = &items[i] else { continue };
            if !matches!(ins.mn, Mn::Call | Mn::Lcall | Mn::Acall) {
                continue;
            }
            let mut j = i + 1;
            while j < items.len() && (!keep[j] || matches!(items[j], Item::Label(_) | Item::Comment(_))) {
                j += 1;
            }
            if let Some(Item::Insn(r)) = items.get(j) {
                if r.mn == Mn::Ret {
                    let t = ins.target().unwrap().clone();
                    items[i] = Item::Insn(Insn::new(Mn::Jmp, vec![Op::Code(t)]));
                    changed = true;
                    // Keep the ret: it may be a jump target.
                }
            }
        }
    }
    // Unreachable instructions after an unconditional transfer until the next label.
    let mut dead = false;
    for i in 0..items.len() {
        match &items[i] {
            Item::Label(_) => dead = false,
            Item::Insn(ins) => {
                if dead && keep[i] {
                    keep[i] = false;
                    changed = true;
                    continue;
                }
                if keep[i] && (is_uncond_jump(ins) || matches!(ins.mn, Mn::Ret | Mn::Reti) || (ins.mn == Mn::Jmp && ins.ops.first() == Some(&Op::AtADptr))) {
                    dead = true;
                }
            }
            Item::Db(_) | Item::Dw(_) | Item::Ds(_) => dead = false,
            _ => {}
        }
    }
    // Unused local labels are harmless; keep them.
    let mut k = keep.iter();
    items.retain(|_| *k.next().unwrap());
    changed
}


/// Short sequences that a shorter one replaces, using liveness to check what may be dropped.
fn combine(items: &mut Vec<Item>, cx: &PeepCtx) -> bool {
    let effs = effects(items, cx);
    let live = liveness(items, cx, &effs);
    // (index, how many items to replace, what to put there); applied after the scan so that every
    // decision reads the liveness of the unmodified list.
    let mut edits: Vec<(usize, usize, Vec<Insn>)> = Vec::new();
    let mut i = 0;
    while i + 1 < items.len() {
        let (Item::Insn(a), Item::Insn(b)) = (&items[i], &items[i + 1]) else {
            i += 1;
            continue;
        };
        // `add a, #1` is `inc a` where the flags it sets are dead.
        if a.mn == Mn::Add && a.ops.first() == Some(&Op::A) && (R_C | R_OV) & live[i + 1] == 0 {
            if let Some(Op::Imm(e)) = a.ops.get(1) {
                let mn = match e.const_val() {
                    Some(1) => Some(Mn::Inc),
                    Some(0xff) => Some(Mn::Dec),
                    _ => None,
                };
                if let Some(mn) = mn {
                    edits.push((i, 1, vec![Insn::new(mn, vec![Op::A])]));
                    i += 1;
                    continue;
                }
            }
        }
        // `dec Rn` then a test against zero is `djnz`.
        if a.mn == Mn::Dec && b.mn == Mn::Cjne && R_C & live[i + 2] == 0 {
            if let (Some(Op::R(n)), Some(Op::R(m)), Some(Op::Imm(k))) = (a.ops.first(), b.ops.first(), b.ops.get(1)) {
                if n == m && k.const_val() == Some(0) {
                    if let Some(t) = b.ops.get(2) {
                        edits.push((i, 2, vec![Insn::new(Mn::Djnz, vec![Op::R(*n), t.clone()])]));
                        i += 2;
                        continue;
                    }
                }
            }
        }
        if a.mn == Mn::Mov && b.mn == Mn::Mov {
            let after = live[i + 2];
            // `mov Rn, <direct>` then `mov a, Rn`, with Rn dead: load A directly.
            if let (Some(Op::R(n)), Some(src @ Op::Dir(_))) = (a.ops.first(), a.ops.get(1)) {
                if b.ops.first() == Some(&Op::A) && b.ops.get(1) == Some(&Op::R(*n)) && reg_res(*n) & after == 0 {
                    edits.push((i, 2, vec![Insn::new(Mn::Mov, vec![Op::A, src.clone()])]));
                    i += 2;
                    continue;
                }
            }
            // `mov a, <direct>` then `mov Rn, a`, with A dead: move it in one step.
            if let (Some(Op::A), Some(src @ Op::Dir(_))) = (a.ops.first(), a.ops.get(1)) {
                if let Some(Op::R(n)) = b.ops.first() {
                    if b.ops.get(1) == Some(&Op::A) && R_A & after == 0 {
                        edits.push((i, 2, vec![Insn::new(Mn::Mov, vec![Op::R(*n), src.clone()])]));
                        i += 2;
                        continue;
                    }
                }
            }
            // A run of moves of one constant: load it into A once and store it from there.
            if R_A & live[i] == 0 {
                if let Some(imm @ Op::Imm(_)) = a.ops.get(1) {
                    let mut n = 0;
                    let mut cost = 0i32;
                    while let Some(Item::Insn(x)) = items.get(i + n) {
                        if x.mn != Mn::Mov || x.ops.get(1) != Some(imm) {
                            break;
                        }
                        match x.ops.first() {
                            Some(Op::R(_)) => cost += 1,
                            Some(Op::Dir(e)) if dir_res(e, cx.bank) != Some(R_A) => cost += 1,
                            _ => break,
                        }
                        n += 1;
                    }
                    let Some(Op::Imm(e)) = a.ops.get(1) else { unreachable!() };
                    let load = if e.const_val() == Some(0) { Insn::new(Mn::Clr, vec![Op::A]) } else { Insn::new(Mn::Mov, vec![Op::A, imm.clone()]) };
                    let setup = insn_size(&load, crate::asm::Reach::Short) as i32;
                    if n >= 2 && cost - setup > 0 {
                        let mut out = vec![load];
                        for k in 0..n {
                            let Item::Insn(x) = &items[i + k] else { unreachable!() };
                            out.push(Insn::new(Mn::Mov, vec![x.ops[0].clone(), Op::A]));
                        }
                        edits.push((i, n, out));
                        i += n;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    let changed = !edits.is_empty();
    for (i, n, ins) in edits.into_iter().rev() {
        items.splice(i..i + n, ins.into_iter().map(Item::Insn));
    }
    changed
}

pub fn optimize(items: &mut Vec<Item>, cx: &PeepCtx, has_asm: bool) {
    for _ in 0..10 {
        let mut changed = jumps(items, cx);
        if !has_asm {
            changed |= dse(items, cx);
            changed |= combine(items, cx);
        }
        if !changed {
            break;
        }
    }
}

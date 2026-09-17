//! Procedural abstraction: factor repeated instruction sequences into shared subroutines.

use crate::asm::{insn_size, Expr, Insn, Item, Mn, Op, Reach};
use crate::link::Section;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

const MAXLEN: usize = 24;

#[derive(Clone, Copy, PartialEq)]
enum EndKind {
    /// Ends with a call to a global symbol: outline as ... + jmp, replace with call.
    Call,
    /// Ends with ret or a jump to a global symbol: replace with jmp.
    Jump,
    /// Ends mid-block: outline with a trailing ret, replace with call.
    Plain,
}

fn movable(i: &Insn, globals: &HashSet<Rc<str>>) -> bool {
    match i.mn {
        Mn::Push | Mn::Pop | Mn::Reti => false,
        Mn::Movc => i.ops.get(1) != Some(&Op::AtAPc),
        Mn::Jmp => {
            if i.ops.first() == Some(&Op::AtADptr) {
                return false;
            }
            is_global_target(i, globals)
        }
        Mn::Call => is_global_target(i, globals),
        Mn::Ret => true,
        m if m.is_cond_jump() => false,
        Mn::Sjmp | Mn::Ajmp | Mn::Ljmp | Mn::Acall | Mn::Lcall => false,
        _ => {
            // Direct writes to SP change the stack; keep them in place.
            !matches!(i.ops.first(), Some(Op::Dir(e)) if e.const_val() == Some(0x81))
        }
    }
}

fn is_global_target(i: &Insn, globals: &HashSet<Rc<str>>) -> bool {
    match i.target() {
        Some(Expr { sym: Some(s), off: 0, .. }) => globals.contains(s),
        _ => false,
    }
}

fn is_transfer(i: &Insn) -> bool {
    matches!(i.mn, Mn::Ret | Mn::Jmp)
}

fn size(i: &Insn) -> u32 {
    match i.mn {
        Mn::Call | Mn::Jmp => 2,
        _ => insn_size(i, Reach::Short),
    }
}

struct Occ {
    sec: usize,
    start: usize,
    len: usize,
}

fn hash_seq(items: &[Item], start: usize, len: usize) -> u64 {
    let mut h = DefaultHasher::new();
    for it in &items[start..start + len] {
        if let Item::Insn(i) = it {
            i.hash(&mut h);
        }
    }
    h.finish()
}

/// Returns the number of outlined routines created.
pub fn run(sections: &mut Vec<Section>, eligible: &[bool], globals: &HashSet<Rc<str>>) -> usize {
    let mut created = 0;
    let mut eligible: Vec<bool> = eligible.to_vec();
    let mut globals = globals.clone();
    for round in 0..300 {
        // Build candidate table: sequences are runs of consecutive instructions (labels allowed only at the start).
        let mut table: HashMap<(u64, usize), Vec<(usize, usize)>> = HashMap::new();
        for (si, sec) in sections.iter().enumerate() {
            if !eligible[si] {
                continue;
            }
            let items = &sec.items;
            for start in 0..items.len() {
                let Item::Insn(first) = &items[start] else { continue };
                if !movable(first, &globals) {
                    continue;
                }
                let mut len = 0;
                let mut j = start;
                while j < items.len() && len < MAXLEN {
                    match &items[j] {
                        Item::Insn(i) => {
                            if !movable(i, &globals) {
                                break;
                            }
                            len += 1;
                            j += 1;
                            if len >= 2 {
                                table.entry((hash_seq(items, start, j - start), j - start)).or_default().push((si, start));
                            }
                            if is_transfer(i) {
                                break;
                            }
                        }
                        Item::Comment(_) => {
                            // Comments break sequences to keep hashing simple.
                            break;
                        }
                        _ => break,
                    }
                }
            }
        }
        // Evaluate candidates.
        let mut best: Option<(i64, Vec<Occ>, EndKind)> = None;
        let mut keys: Vec<(&(u64, usize), &Vec<(usize, usize)>)> = table.iter().collect();
        keys.sort_by_key(|(k, v)| (v[0], k.1, k.0));
        for ((_, span), occs) in keys {
            if occs.len() < 2 {
                continue;
            }
            let span = *span;
            let (s0, p0) = occs[0];
            let seq: Vec<&Insn> = sections[s0].items[p0..p0 + span].iter().filter_map(|it| if let Item::Insn(i) = it { Some(i) } else { None }).collect();
            let bytes: u32 = seq.iter().map(|i| size(i)).sum();
            let last = seq.last().unwrap();
            let kind = match last.mn {
                Mn::Call => EndKind::Call,
                Mn::Ret | Mn::Jmp => EndKind::Jump,
                _ => EndKind::Plain,
            };
            // Non-overlapping, verified occurrences.
            let mut chosen: Vec<Occ> = Vec::new();
            let mut sorted = occs.clone();
            sorted.sort();
            for (si, p) in sorted {
                let same = sections[si].items[p..p + span].iter().zip(sections[s0].items[p0..p0 + span].iter()).all(|(a, b)| a == b);
                if !same {
                    continue;
                }
                if let Some(last) = chosen.last() {
                    if last.sec == si && p < last.start + last.len {
                        continue;
                    }
                }
                chosen.push(Occ { sec: si, start: p, len: span });
            }
            let k = chosen.len() as i64;
            if k < 2 {
                continue;
            }
            let s = bytes as i64;
            let c = 2i64;
            let saving = match kind {
                EndKind::Call => k * s - (s + k * c),
                EndKind::Jump => k * s - (s + k * c),
                EndKind::Plain => k * s - (s + 1 + k * c),
            };
            // Prefer larger savings; break ties toward longer sequences.
            if saving >= 2 && best.as_ref().map_or(true, |b| saving > b.0) {
                best = Some((saving, chosen, kind));
            }
        }
        let Some((_, occs, kind)) = best else { break };
        // Build the outlined routine.
        let name: Rc<str> = format!("__outl{}", round).into();
        let first = &occs[0];
        let mut body: Vec<Item> = vec![Item::Label(name.clone())];
        let seq: Vec<Insn> = sections[first.sec].items[first.start..first.start + first.len]
            .iter()
            .filter_map(|it| if let Item::Insn(i) = it { Some(i.clone()) } else { None })
            .collect();
        let n = seq.len();
        for (idx, ins) in seq.into_iter().enumerate() {
            if idx + 1 == n && kind == EndKind::Call {
                body.push(Item::Insn(Insn::new(Mn::Jmp, ins.ops.clone())));
            } else {
                body.push(Item::Insn(ins));
            }
        }
        if kind == EndKind::Plain {
            body.push(Item::Insn(Insn::new(Mn::Ret, vec![])));
        }
        // Replace occurrences (from the back so indices stay valid).
        let mut by_sec: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        for o in &occs {
            by_sec.entry(o.sec).or_default().push((o.start, o.len));
        }
        for (si, mut list) in by_sec {
            list.sort();
            for (start, len) in list.into_iter().rev() {
                // Labels at the start are kept; the span has only instructions besides that.
                let repl = match kind {
                    EndKind::Jump => Insn::new(Mn::Jmp, vec![Op::label(&name)]),
                    _ => Insn::new(Mn::Call, vec![Op::label(&name)]),
                };
                sections[si].items.splice(start..start + len, std::iter::once(Item::Insn(repl)));
            }
        }
        sections.push(Section { name: name.clone(), items: body, org: None, absolute: false });
        eligible.push(true);
        globals.insert(name);
        created += 1;
    }
    created
}

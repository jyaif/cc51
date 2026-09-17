//! Instruction selection with machine-state tracking.

use super::alloc::Alloc;
use super::fold::{FoldInfo, FoldKind};
use super::*;
use crate::asm::parse::{AsmParser, SymRes};
use crate::asm::{Item, Insn, Mn, Part, A_DIR, B_DIR, DPH_DIR, DPL_DIR, EA_BIT};
use crate::ir::*;
use crate::types::Space;
use std::collections::HashMap;

pub struct GenCtx<'a> {
    pub callee: &'a dyn Fn(&Callee) -> Summary,
    pub callee_sym: &'a dyn Fn(&Callee) -> Rc<str>,
    pub sym_space: &'a dyn Fn(&Sym) -> Space,
    pub sym_expr: &'a dyn Fn(&Sym) -> Rc<str>,
    pub asm_resolve: &'a dyn Fn(&str) -> Option<SymRes>,
}

#[derive(Clone, Debug, PartialEq)]
enum Cont {
    K(u8),
    E(Expr),
    L(Op),
}

#[derive(Clone, Debug, PartialEq)]
enum Dp {
    E(Expr),
    L(Op, Op),
}

#[derive(Clone, Debug, PartialEq)]
enum CBit {
    K(bool),
    L(Op),
}

#[derive(Clone, Default, Debug)]
struct State {
    a: Option<Cont>,
    b: Option<Cont>,
    dptr: Option<Dp>,
    c: Option<CBit>,
    mem: HashMap<Op, Cont>,
}

/// A source byte.
#[derive(Clone, Debug, PartialEq)]
pub enum Src {
    K(u8),
    E(Expr),
    L(Loc),
    /// Folded 8-bit (or bit) expression computed into A (or C).
    Tree(VReg),
    /// The current value of A (used during parallel moves).
    Acc,
}

pub struct FnCode {
    pub items: Vec<Item>,
    pub clobbers: RegSet,
    pub uses_b: bool,
    pub uses_dptr: bool,
}

pub struct Gen<'a> {
    cx: &'a GenCtx<'a>,
    f: &'a Func,
    al: &'a Alloc,
    fold: &'a FoldInfo,
    ret_locs: Vec<Loc>,
    bank: u8,
    items: Vec<Item>,
    st: State,
    nlabel: u32,
    scratch: RegSet,
    cur: (usize, usize),
    pub uses_b: bool,
    pub uses_dptr: bool,
    is_isr: bool,
}

fn sfr_track(a: i64) -> bool {
    a < 0x80
}

impl<'a> Gen<'a> {
    pub fn new(cx: &'a GenCtx<'a>, f: &'a Func, al: &'a Alloc, fold: &'a FoldInfo, ret_locs: Vec<Loc>, bank: u8) -> Self {
        Gen {
            cx,
            f,
            al,
            fold,
            ret_locs,
            bank,
            items: Vec::new(),
            st: State::default(),
            nlabel: 0,
            scratch: 0,
            cur: (0, 0),
            uses_b: false,
            uses_dptr: false,
            is_isr: f.attrs.interrupt.is_some(),
        }
    }

    // ------------------------------------------------------------------
    // Emission and state tracking

    fn key(&self, op: &Op) -> Option<Op> {
        match op {
            Op::R(_) => Some(op.clone()),
            Op::Dir(e) => {
                if let Some(v) = e.const_val() {
                    let base = self.bank as i64 * 8;
                    if v >= base && v < base + 8 {
                        return Some(Op::R((v - base) as u8));
                    }
                    if !sfr_track(v) {
                        return None;
                    }
                }
                Some(op.clone())
            }
            Op::Bit(e) => {
                if let Some(v) = e.const_val() {
                    if v >= 0x80 {
                        return None;
                    }
                }
                Some(op.clone())
            }
            _ => None,
        }
    }

    fn content_of(&self, op: &Op) -> Option<Cont> {
        match op {
            Op::Imm(e) => Some(match e.const_val() {
                Some(v) => Cont::K(v as u8),
                None => Cont::E(e.clone()),
            }),
            Op::A => self.st.a.clone(),
            _ => {
                let k = self.key(op)?;
                Some(self.st.mem.get(&k).cloned().unwrap_or(Cont::L(k)))
            }
        }
    }

    fn invalidate_refs(&mut self, k: &Op) {
        let r = Cont::L(k.clone());
        if self.st.a.as_ref() == Some(&r) {
            self.st.a = None;
        }
        if self.st.b.as_ref() == Some(&r) {
            self.st.b = None;
        }
        if let Some(Dp::L(lo, hi)) = &self.st.dptr {
            if lo == k || hi == k {
                self.st.dptr = None;
            }
        }
        if self.st.c == Some(CBit::L(k.clone())) {
            self.st.c = None;
        }
        self.st.mem.retain(|_, v| *v != r);
    }

    fn write(&mut self, dst: &Op, c: Option<Cont>) {
        if let Op::Dir(e) = dst {
            match e.const_val() {
                Some(0xE0) => {
                    self.st.a = c;
                    return;
                }
                Some(0xF0) => {
                    self.st.b = c;
                    return;
                }
                Some(0x82) | Some(0x83) => {
                    self.st.dptr = None;
                    return;
                }
                Some(0xD0) => {
                    self.st.c = None;
                    return;
                }
                _ => {}
            }
        }
        let Some(k) = self.key(dst) else { return };
        if c.as_ref() == Some(&Cont::L(k.clone())) {
            return;
        }
        self.invalidate_refs(&k);
        match c {
            Some(c) => {
                self.st.mem.insert(k, c);
            }
            None => {
                self.st.mem.remove(&k);
            }
        }
    }

    fn clobber_mem(&mut self) {
        // Unknown indirect write into internal RAM.
        self.st.mem.retain(|k, _| matches!(k, Op::R(_)));
        let keep = |c: &Option<Cont>| match c {
            Some(Cont::L(Op::R(_))) | Some(Cont::K(_)) | Some(Cont::E(_)) | None => true,
            _ => false,
        };
        if !keep(&self.st.a) {
            self.st.a = None;
        }
        if !keep(&self.st.b) {
            self.st.b = None;
        }
        self.st.mem.retain(|_, v| !matches!(v, Cont::L(Op::Dir(_))));
        if !matches!(self.st.dptr, Some(Dp::E(_)) | None) {
            self.st.dptr = None;
        }
        if matches!(self.st.c, Some(CBit::L(_))) {
            self.st.c = None;
        }
    }

    fn transfer(&mut self, i: &Insn) {
        let o = &i.ops;
        let kfold = |c: &Option<Cont>, f: &dyn Fn(u8) -> u8| -> Option<Cont> {
            match c {
                Some(Cont::K(k)) => Some(Cont::K(f(*k))),
                _ => None,
            }
        };
        match i.mn {
            Mn::Mov => match (&o[0], &o[1]) {
                (Op::A, src) => {
                    self.st.a = self.content_of(src);
                    if self.st.a.is_none() {
                        // e.g. SFR: A is unknown
                    }
                }
                (Op::Dptr, Op::Imm(e)) => self.st.dptr = Some(Dp::E(e.clone())),
                (Op::C, Op::Bit(_)) => {
                    self.st.c = self.key(&o[1]).map(|k| match self.st.mem.get(&k) {
                        Some(Cont::K(v)) => CBit::K(*v != 0),
                        _ => CBit::L(k),
                    });
                }
                (Op::Bit(_), Op::C) => {
                    let c = match &self.st.c {
                        Some(CBit::K(b)) => Some(Cont::K(*b as u8)),
                        Some(CBit::L(k)) => Some(Cont::L(k.clone())),
                        None => None,
                    };
                    let c = if c.is_none() { None } else { c };
                    self.write(&o[0], c);
                    if self.st.c.is_none() {
                        if let Some(k) = self.key(&o[0]) {
                            self.st.c = Some(CBit::L(k));
                        }
                    }
                }
                (Op::AtR(_), _) => self.clobber_mem(),
                (dst, Op::A) => {
                    let c = self.st.a.clone();
                    self.write(dst, c.clone());
                    if c.is_none() {
                        self.st.a = self.key(dst).map(Cont::L);
                    }
                }
                (dst, src) => {
                    let c = self.content_of(src);
                    self.write(dst, c);
                }
            },
            Mn::Movx => {
                if o[0] == Op::A {
                    self.st.a = None;
                } else if let Op::AtR(_) = o[0] {
                    // external RAM write: no internal state change
                }
            }
            Mn::Movc => self.st.a = None,
            Mn::Inc | Mn::Dec => {
                let d: i16 = if i.mn == Mn::Inc { 1 } else { -1 };
                match &o[0] {
                    Op::A => self.st.a = kfold(&self.st.a, &|k| (k as i16 + d) as u8),
                    Op::Dptr => {
                        self.st.dptr = match &self.st.dptr {
                            Some(Dp::E(e)) if e.part == Part::Val => Some(Dp::E(e.clone().add(1))),
                            _ => None,
                        }
                    }
                    Op::AtR(_) => self.clobber_mem(),
                    dst => {
                        let c = self.content_of(dst);
                        let nc = match c {
                            Some(Cont::K(k)) => Some(Cont::K((k as i16 + d) as u8)),
                            _ => None,
                        };
                        self.write(dst, nc);
                    }
                }
                if i.mn == Mn::Inc && o[0] != Op::Dptr {
                    // INC does not affect C.
                }
            }
            Mn::Add | Mn::Addc | Mn::Subb => {
                self.st.a = None;
                self.st.c = None;
            }
            Mn::Anl | Mn::Orl | Mn::Xrl => match &o[0] {
                Op::A => {
                    self.st.a = match (&self.st.a, &o[1]) {
                        (Some(Cont::K(a)), Op::Imm(e)) if e.const_val().is_some() => {
                            let b = e.const_val().unwrap() as u8;
                            Some(Cont::K(match i.mn {
                                Mn::Anl => a & b,
                                Mn::Orl => a | b,
                                _ => a ^ b,
                            }))
                        }
                        _ => None,
                    }
                }
                Op::C => self.st.c = None,
                dst => {
                    let dst = dst.clone();
                    self.write(&dst, None)
                }
            },
            Mn::Clr | Mn::Setb | Mn::Cpl => {
                let val = match i.mn {
                    Mn::Clr => Some(false),
                    Mn::Setb => Some(true),
                    _ => None,
                };
                match &o[0] {
                    Op::A => {
                        self.st.a = match i.mn {
                            Mn::Clr => Some(Cont::K(0)),
                            _ => kfold(&self.st.a, &|k| !k),
                        }
                    }
                    Op::C => {
                        self.st.c = match (val, &self.st.c) {
                            (Some(v), _) => Some(CBit::K(v)),
                            (None, Some(CBit::K(v))) => Some(CBit::K(!v)),
                            _ => None,
                        }
                    }
                    b => {
                        let b = b.clone();
                        self.write(&b, val.map(|v| Cont::K(v as u8)));
                    }
                }
            }
            Mn::Rl | Mn::Rr | Mn::Swap | Mn::Da => self.st.a = None,
            Mn::Rlc | Mn::Rrc => {
                self.st.a = None;
                self.st.c = None;
            }
            Mn::Mul | Mn::Div => {
                self.st.a = None;
                self.st.b = None;
                self.st.c = Some(CBit::K(false));
            }
            Mn::Xch => {
                let other = o[1].clone();
                match other {
                    Op::AtR(_) => {
                        self.clobber_mem();
                        self.st.a = None;
                    }
                    _ => {
                        let oc = self.content_of(&other);
                        let ac = self.st.a.clone();
                        self.write(&other, ac);
                        self.st.a = oc;
                    }
                }
            }
            Mn::Xchd => {
                self.st.a = None;
                self.clobber_mem();
            }
            Mn::Pop => {
                let d = o[0].clone();
                self.write(&d, None);
            }
            Mn::Djnz => {
                let d = o[0].clone();
                self.write(&d, None);
            }
            Mn::Cjne => self.st.c = None,
            Mn::Jbc => {
                let d = o[0].clone();
                self.write(&d, Some(Cont::K(0)));
            }
            Mn::Acall | Mn::Lcall | Mn::Call => self.st = State::default(),
            _ => {}
        }
    }

    fn emit(&mut self, mn: Mn, ops: Vec<Op>) {
        let i = Insn::new(mn, ops);
        for op in &i.ops {
            match op {
                Op::R(r) | Op::AtR(r) => {
                    let _ = r;
                }
                Op::AB => self.uses_b = true,
                Op::Dptr | Op::AtDptr | Op::AtADptr => self.uses_dptr = true,
                Op::Dir(e) => match e.const_val() {
                    Some(0xF0) => self.uses_b = true,
                    Some(0x82) | Some(0x83) => self.uses_dptr = true,
                    _ => {}
                },
                _ => {}
            }
        }
        // Record register writes.
        let writes_reg = match (i.mn, i.ops.first()) {
            (Mn::Mov | Mn::Inc | Mn::Dec | Mn::Djnz | Mn::Xch | Mn::Pop | Mn::Anl | Mn::Orl | Mn::Xrl, Some(Op::R(r))) => Some(*r),
            (Mn::Xch, _) => match i.ops.get(1) {
                Some(Op::R(r)) => Some(*r),
                _ => None,
            },
            (Mn::Mov | Mn::Inc | Mn::Dec | Mn::Djnz | Mn::Pop | Mn::Anl | Mn::Orl | Mn::Xrl, Some(Op::Dir(e))) => match e.const_val() {
                Some(v) if v >= self.bank as i64 * 8 && v < self.bank as i64 * 8 + 8 => Some((v - self.bank as i64 * 8) as u8),
                _ => None,
            },
            _ => None,
        };
        if let Some(r) = writes_reg {
            self.scratch |= 1 << r;
        }
        self.transfer(&i);
        self.items.push(Item::Insn(i));
    }

    fn e0(&mut self, mn: Mn) {
        self.emit(mn, vec![]);
    }
    fn e1(&mut self, mn: Mn, a: Op) {
        self.emit(mn, vec![a]);
    }
    fn e2(&mut self, mn: Mn, a: Op, b: Op) {
        self.emit(mn, vec![a, b]);
    }

    fn new_label(&mut self) -> Rc<str> {
        self.nlabel += 1;
        format!("L{}_x{}", self.f.id, self.nlabel).into()
    }

    fn block_label(&self, b: BlockId) -> Rc<str> {
        format!("L{}_{}", self.f.id, b).into()
    }

    fn place_label(&mut self, l: Rc<str>, keep_state: bool) {
        self.items.push(Item::Label(l));
        if !keep_state {
            self.st = State::default();
        }
    }

    fn jmp(&mut self, l: &Rc<str>) {
        self.e1(Mn::Jmp, Op::label(l));
    }

    // ------------------------------------------------------------------
    // Values and sources

    fn locs(&self, v: VReg) -> &[Loc] {
        &self.al.locs[v as usize]
    }

    fn vty(&self, v: Val, default: Ty) -> Ty {
        match v {
            Val::R(r) => self.f.ty(r),
            _ => default,
        }
    }

    fn sym_name(&self, s: &Sym) -> Rc<str> {
        (self.cx.sym_expr)(s)
    }

    fn addr_expr(&self, s: &Sym, off: i32) -> Expr {
        Expr::sym_off(&self.sym_name(s), off as i64)
    }

    fn src(&self, v: Val, k: u32) -> Src {
        match v {
            Val::K(x) => Src::K(((x as u64) >> (8 * k.min(7))) as u8),
            Val::Addr(s, o) => {
                let e = self.addr_expr(&s, o);
                Src::E(match k {
                    0 => e.lo(),
                    1 => e.hi(),
                    2 => Expr::num(0x00),
                    _ => Expr::num(0),
                })
            }
            Val::R(r) => match self.fold.kind[r as usize] {
                FoldKind::Acc => {
                    if k == 0 {
                        Src::Tree(r)
                    } else {
                        Src::K(0)
                    }
                }
                FoldKind::Leaf => {
                    let Inst::Load(_, m) = self.fold.def(self.f, r) else { unreachable!() };
                    Src::L(self.direct_loc(m, k as i32))
                }
                _ => {
                    let l = self.locs(r);
                    match l.get(k as usize) {
                        Some(l) => Src::L(*l),
                        None => Src::K(0),
                    }
                }
            },
        }
    }

    /// Location of a directly addressable memory byte.
    fn direct_loc(&self, m: &Mem, k: i32) -> Loc {
        match m {
            Mem::Sym(Sym::Global(g), o) => Loc::Glob(*g, (o + k) as u16),
            Mem::Sym(Sym::Frame(f, n), o) => Loc::Obj(*f, *n, (o + k) as u16),
            Mem::Abs(_, a) => Loc::Dir((*a as i64 + k as i64) as u8),
            _ => panic!("not a direct location: {:?}", m),
        }
    }

    fn src_op(&self, s: &Src) -> Option<Op> {
        match s {
            Src::K(k) => Some(Op::imm(*k as i64)),
            Src::E(e) => Some(Op::Imm(e.clone())),
            Src::L(l) => Some(loc_op(*l)),
            _ => None,
        }
    }

    /// Operand usable where a direct address is required (registers via their direct address).
    fn src_dir_op(&self, s: &Src) -> Option<Op> {
        match s {
            Src::L(l) => Some(loc_dir(*l, self.bank)),
            _ => None,
        }
    }

    fn a_holds(&self, s: &Src) -> bool {
        match s {
            Src::Acc => true,
            Src::Tree(_) => false,
            _ => {
                let op = self.src_op(s).unwrap();
                match (&self.st.a, self.content_of(&op)) {
                    (Some(a), Some(c)) => *a == c,
                    _ => false,
                }
            }
        }
    }

    fn load_a(&mut self, s: &Src) {
        if self.a_holds(s) {
            return;
        }
        match s {
            Src::K(0) => self.e1(Mn::Clr, Op::A),
            Src::Tree(r) => self.gen_tree(*r),
            Src::Acc => {}
            Src::L(l) if l.is_bit() => {
                self.e2(Mn::Mov, Op::C, loc_op(*l));
                self.e1(Mn::Clr, Op::A);
                self.e1(Mn::Rlc, Op::A);
            }
            _ => {
                // Prefer a register/imm source known to hold the same value (shorter).
                let op = self.src_op(s).unwrap();
                if let Some(Cont::K(k)) = self.content_of(&op) {
                    if k == 0 {
                        self.e1(Mn::Clr, Op::A);
                        return;
                    }
                }
                self.e2(Mn::Mov, Op::A, op);
            }
        }
    }

    fn store_a(&mut self, d: Loc) {
        if d.is_bit() {
            self.e1(Mn::Rrc, Op::A);
            self.e2(Mn::Mov, loc_op(d), Op::C);
            return;
        }
        let op = loc_op(d);
        if let (Some(a), Some(c)) = (&self.st.a, self.content_of(&op)) {
            if *a == c {
                return;
            }
        }
        self.e2(Mn::Mov, op, Op::A);
    }

    fn move_byte(&mut self, d: Loc, s: &Src) {
        if d.is_bit() {
            self.load_c_src(s);
            self.store_c(d);
            return;
        }
        let dop = loc_op(d);
        if let Src::L(l) = s {
            if *l == d {
                return;
            }
            if l.is_bit() {
                self.load_c_src(s);
                self.e1(Mn::Clr, Op::A);
                self.e1(Mn::Rlc, Op::A);
                self.store_a(d);
                return;
            }
        }
        // Already there?
        if let (Some(sop), Some(dc)) = (self.src_op(s).or_else(|| if matches!(s, Src::Acc) { Some(Op::A) } else { None }), self.content_of(&dop)) {
            if let Some(sc) = self.content_of(&sop) {
                if sc == dc {
                    return;
                }
            }
        }
        match s {
            Src::Tree(_) | Src::Acc => {
                self.load_a(s);
                self.store_a(d);
            }
            _ => {
                if self.a_holds(s) {
                    self.store_a(d);
                    return;
                }
                let sop = self.src_op(s).unwrap();
                // A register known to hold the value is a cheaper source for direct destinations.
                if !d.is_reg() {
                    if let Some(Cont::K(k)) = self.content_of(&sop) {
                        for r in 0..8u8 {
                            if self.st.mem.get(&Op::R(r)) == Some(&Cont::K(k)) {
                                self.e2(Mn::Mov, dop.clone(), Op::R(r));
                                return;
                            }
                        }
                    }
                }
                match (&dop, &sop) {
                    (Op::R(_), Op::R(_)) => {
                        let sd = self.src_dir_op(s).unwrap();
                        self.e2(Mn::Mov, dop, sd);
                    }
                    _ => self.e2(Mn::Mov, dop, sop),
                }
            }
        }
    }

    fn load_c_src(&mut self, s: &Src) {
        match s {
            Src::K(k) => {
                if self.st.c != Some(CBit::K(*k & 1 != 0)) {
                    self.e1(if *k & 1 != 0 { Mn::Setb } else { Mn::Clr }, Op::C);
                }
            }
            Src::Tree(r) => self.gen_tree_c(*r),
            Src::L(l) if l.is_bit() => {
                let op = loc_op(*l);
                if let Some(k) = self.key(&op) {
                    if self.st.c == Some(CBit::L(k)) {
                        return;
                    }
                }
                self.e2(Mn::Mov, Op::C, op);
            }
            Src::Acc => self.e1(Mn::Rrc, Op::A),
            _ => {
                self.load_a(s);
                self.e1(Mn::Rrc, Op::A);
            }
        }
    }

    fn store_c(&mut self, d: Loc) {
        if d.is_bit() {
            self.e2(Mn::Mov, loc_op(d), Op::C);
        } else {
            self.e1(Mn::Clr, Op::A);
            self.e1(Mn::Rlc, Op::A);
            self.store_a(d);
        }
    }

    /// Parallel move. Sources that are trees must be single moves.
    fn par_move(&mut self, moves: Vec<(Loc, Src)>) {
        let mut pending: Vec<(Loc, Src)> = moves.into_iter().filter(|(d, s)| *s != Src::L(*d)).collect();
        // Moves into ACC must be done last.
        let mut acc_moves: Vec<(Loc, Src)> = Vec::new();
        pending.retain(|m| {
            if m.0 == ACC {
                acc_moves.push(m.clone());
                false
            } else {
                true
            }
        });
        // Trees first (they only clobber A).
        let trees: Vec<(Loc, Src)> = pending.iter().filter(|m| matches!(m.1, Src::Tree(_))).cloned().collect();
        pending.retain(|m| !matches!(m.1, Src::Tree(_)));
        for (d, s) in trees {
            self.move_byte(d, &s);
        }
        let mut acc_holds: Option<Loc> = None;
        while !pending.is_empty() {
            let is_src = |l: Loc, pending: &Vec<(Loc, Src)>, skip: usize| pending.iter().enumerate().any(|(i, m)| i != skip && m.1 == Src::L(l));
            if let Some(i) = (0..pending.len()).find(|&i| !is_src(pending[i].0, &pending, i)) {
                let (d, s) = pending.remove(i);
                if s == Src::Acc {
                    self.store_a(d);
                } else {
                    // Don't disturb A if it holds a pending value.
                    if acc_holds.is_some() && !(d.is_reg() || matches!(s, Src::L(_) | Src::K(_) | Src::E(_))) {
                        panic!("par_move: A busy");
                    }
                    if acc_holds.is_some() {
                        // Only direct moves (no A) allowed.
                        let dop = loc_op(d);
                        match &s {
                            Src::L(l) if l.is_reg() && d.is_reg() => {
                                let sd = loc_dir(*l, self.bank);
                                self.e2(Mn::Mov, dop, sd);
                            }
                            _ => {
                                let sop = self.src_op(&s).unwrap();
                                self.e2(Mn::Mov, dop, sop);
                            }
                        }
                    } else {
                        self.move_byte(d, &s);
                    }
                }
                continue;
            }
            // Cycle: save the destination of the first move into A.
            let (d, _) = pending[0].clone();
            assert!(acc_holds.is_none(), "par_move: nested cycle");
            self.load_a(&Src::L(d));
            acc_holds = Some(d);
            for m in pending.iter_mut() {
                if m.1 == Src::L(d) {
                    m.1 = Src::Acc;
                }
            }
        }
        for (d, s) in acc_moves {
            let _ = d;
            self.load_a(&s);
        }
    }

    // ------------------------------------------------------------------
    // Scratch registers

    fn busy_regs(&self, extra: &[Val]) -> RegSet {
        let (b, i) = self.cur;
        let mut busy: RegSet = 0;
        let live = &self.al.live_after[b][i];
        for v in live.iter() {
            for l in &self.al.locs[v] {
                if let Loc::R(r) = l {
                    busy |= 1 << r;
                }
            }
        }
        for v in extra {
            if let Val::R(r) = v {
                for l in self.al.locs.get(*r as usize).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if let Loc::R(x) = l {
                        busy |= 1 << x;
                    }
                }
            }
        }
        busy
    }

    fn pick_ptr_reg(&self, busy: RegSet) -> Option<u8> {
        [1u8, 0].into_iter().find(|&r| busy & (1 << r) == 0)
    }

    // ------------------------------------------------------------------
    // Trees

    fn gen_tree(&mut self, r: VReg) {
        let ins = self.fold.def(self.f, r).clone();
        let ty = self.f.ty(r);
        if ty == Ty::Bit {
            self.gen_tree_c(r);
            self.e1(Mn::Clr, Op::A);
            self.e1(Mn::Rlc, Op::A);
            return;
        }
        match ins {
            Inst::Bin(op, _, a, b) => self.bin8_to_a(op, a, b),
            Inst::Un(op, _, a) => {
                self.load_a(&self.src(a, 0));
                match op {
                    UnK::Not => self.e1(Mn::Cpl, Op::A),
                    UnK::Neg => {
                        self.e1(Mn::Cpl, Op::A);
                        self.e1(Mn::Inc, Op::A);
                    }
                }
            }
            Inst::Load(_, m) => self.load_mem_a(&m, 0),
            Inst::Trunc(_, a) => {
                let at = self.vty(a, Ty::I16);
                if at == Ty::Bit {
                    let s = self.src(a, 0);
                    self.load_c_src(&s);
                    self.e1(Mn::Clr, Op::A);
                    self.e1(Mn::Rlc, Op::A);
                } else {
                    let s = self.src(a, 0);
                    self.load_a(&s);
                }
            }
            Inst::Ext(_, a, _) => {
                // bit -> byte
                let s = self.src(a, 0);
                self.load_c_src(&s);
                self.e1(Mn::Clr, Op::A);
                self.e1(Mn::Rlc, Op::A);
            }
            Inst::Cmp(c, _, a, b, cty) => {
                self.cmp_to_a(c, a, b, cty);
            }
            Inst::Call(d, c, args) => {
                let rt = d.map(|d| self.f.ty(d));
                let s = self.gen_call_ty(None, &c, &args, rt);
                if let Some(r) = s.ret.first() {
                    if *r == CARRY {
                        self.e1(Mn::Clr, Op::A);
                        self.e1(Mn::Rlc, Op::A);
                    } else if *r != ACC {
                        self.load_a(&Src::L(*r));
                    }
                }
            }
            other => panic!("unsupported tree node {:?}", other),
        }
    }

    fn gen_tree_c(&mut self, r: VReg) {
        let ins = self.fold.def(self.f, r).clone();
        match ins {
            Inst::Cmp(c, _, a, b, ty) => {
                let inv = self.cmp_to_c(c, a, b, ty);
                if inv {
                    self.e1(Mn::Cpl, Op::C);
                }
            }
            Inst::Load(_, m) => self.load_bit_c(&m),
            Inst::Trunc(_, a) => {
                let s = self.src(a, 0);
                self.load_a(&s);
                self.e1(Mn::Rrc, Op::A);
            }
            Inst::Bin(op, _, a, b) => {
                let sa = self.src(a, 0);
                self.load_c_src(&sa);
                let sb = self.src(b, 0);
                match (op, &sb) {
                    (BinK::And, Src::K(k)) => {
                        if k & 1 == 0 {
                            self.e1(Mn::Clr, Op::C);
                        }
                    }
                    (BinK::Or, Src::K(k)) => {
                        if k & 1 != 0 {
                            self.e1(Mn::Setb, Op::C);
                        }
                    }
                    (BinK::Xor, Src::K(k)) => {
                        if k & 1 != 0 {
                            self.e1(Mn::Cpl, Op::C);
                        }
                    }
                    (BinK::And, Src::L(l)) if l.is_bit() => self.e2(Mn::Anl, Op::C, loc_op(*l)),
                    (BinK::Or, Src::L(l)) if l.is_bit() => self.e2(Mn::Orl, Op::C, loc_op(*l)),
                    (BinK::Xor, Src::L(l)) if l.is_bit() => {
                        let skip = self.new_label();
                        self.e2(Mn::Jnb, loc_op(*l), Op::label(&skip));
                        self.e1(Mn::Cpl, Op::C);
                        self.place_label(skip, true);
                        self.st.c = None;
                    }
                    _ => panic!("unsupported bit op"),
                }
            }
            Inst::Call(d, c, args) => {
                let rt = d.map(|d| self.f.ty(d));
                let s = self.gen_call_ty(None, &c, &args, rt);
                match s.ret.first() {
                    Some(r) if *r == CARRY => {}
                    Some(r) if *r == ACC => self.e1(Mn::Rrc, Op::A),
                    Some(r) => {
                        let l = *r;
                        self.load_c_src(&Src::L(l));
                    }
                    None => {}
                }
            }
            Inst::Ext(..) | Inst::Un(..) => {
                self.gen_tree(r);
                self.e1(Mn::Rrc, Op::A);
            }
            other => panic!("unsupported bit tree node {:?}", other),
        }
    }

    /// 8-bit binary operation with the result left in A.
    fn bin8_to_a(&mut self, op: BinK, a: Val, b: Val) {
        let sa = self.src(a, 0);
        let sb = self.src(b, 0);
        match op {
            BinK::Add | BinK::Sub | BinK::And | BinK::Or | BinK::Xor => {
                // Reversed operand order if a is simple and b is a tree (commutative).
                if matches!(sb, Src::Tree(_)) {
                    assert!(op != BinK::Sub);
                    self.load_a(&sb);
                    self.alu(op, &sa);
                } else {
                    self.load_a(&sa);
                    self.alu(op, &sb);
                }
            }
            BinK::Mul => {
                self.load_b_then_a(&sb, &sa);
                self.e1(Mn::Mul, Op::AB);
            }
            BinK::DivU | BinK::ModU => {
                self.load_b_then_a(&sb, &sa);
                self.e1(Mn::Div, Op::AB);
                if op == BinK::ModU {
                    self.e2(Mn::Xch, Op::A, Op::dir(B_DIR));
                }
            }
            BinK::DivS | BinK::ModS => {
                // Helper: __divschar / __modschar: operands in R7 (a) and R3 (b)... use A/B based helper.
                self.load_b_then_a(&sb, &sa);
                let name: Rc<str> = if op == BinK::DivS { "__divschar_ab".into() } else { "__modschar_ab".into() };
                self.e1(Mn::Call, Op::label(&name));
                self.st = State::default();
            }
            BinK::Shl | BinK::ShrU | BinK::ShrS => {
                match sb {
                    Src::K(k) => {
                        self.load_a(&sa);
                        self.shift8_const(op, k as u32);
                    }
                    _ => {
                        // Count into B, value into A.
                        self.load_b_then_a(&sb, &sa);
                        self.shift_loop_a(op);
                    }
                }
            }
        }
    }

    fn load_b_then_a(&mut self, sb: &Src, sa: &Src) {
        // If a is a tree, compute it first and keep it safe while loading B.
        match (sa, sb) {
            (Src::Tree(_), Src::Tree(_)) => panic!("two trees"),
            (_, Src::Tree(_)) => {
                self.load_a(sb);
                self.e2(Mn::Mov, Op::dir(B_DIR), Op::A);
                self.load_a(sa);
            }
            (Src::Tree(_), _) => {
                self.load_a(sa);
                self.load_b(sb);
            }
            _ => {
                self.load_b(sb);
                self.load_a(sa);
            }
        }
        self.uses_b = true;
    }

    fn load_b(&mut self, s: &Src) {
        let op = self.src_op(s).unwrap();
        if let (Some(b), Some(c)) = (&self.st.b, self.content_of(&op)) {
            if *b == c {
                return;
            }
        }
        if self.a_holds(s) {
            self.e2(Mn::Mov, Op::dir(B_DIR), Op::A);
            return;
        }
        let sop = match (&op, s) {
            (Op::R(_), Src::L(l)) => loc_dir(*l, self.bank),
            _ => op,
        };
        self.e2(Mn::Mov, Op::dir(B_DIR), sop);
    }

    /// A = A op src
    fn alu(&mut self, op: BinK, s: &Src) {
        let mn = match op {
            BinK::Add => Mn::Add,
            BinK::Sub => Mn::Subb,
            BinK::And => Mn::Anl,
            BinK::Or => Mn::Orl,
            BinK::Xor => Mn::Xrl,
            _ => unreachable!(),
        };
        if let Src::K(k) = s {
            match (op, k) {
                (BinK::Add | BinK::Sub | BinK::Or | BinK::Xor, 0) => return,
                (BinK::And, 0xff) => return,
                (BinK::Add, 1) => return self.e1(Mn::Inc, Op::A),
                (BinK::Add, 0xff) => return self.e1(Mn::Dec, Op::A),
                (BinK::And, 0) => return self.e1(Mn::Clr, Op::A),
                (BinK::Xor, 0xff) => return self.e1(Mn::Cpl, Op::A),
                _ => {}
            }
        }
        if op == BinK::Sub {
            if let Src::K(k) = s {
                self.e2(Mn::Add, Op::A, Op::imm((-(*k as i64)) & 0xff));
                return;
            }
            self.e1(Mn::Clr, Op::C);
        }
        let sop = self.src_op(s).expect("alu: complex operand");
        self.e2(mn, Op::A, sop);
    }

    fn shift8_const(&mut self, op: BinK, k: u32) {
        if k == 0 {
            return;
        }
        if k >= 8 {
            match op {
                BinK::ShrS => {
                    self.e1(Mn::Rlc, Op::A);
                    self.e2(Mn::Subb, Op::A, Op::dir(A_DIR));
                }
                _ => self.e1(Mn::Clr, Op::A),
            }
            return;
        }
        match op {
            BinK::Shl => {
                if k == 1 {
                    self.e2(Mn::Add, Op::A, Op::dir(A_DIR));
                } else if k <= 3 {
                    for _ in 0..k {
                        self.e1(Mn::Rl, Op::A);
                    }
                    self.e2(Mn::Anl, Op::A, Op::imm(((0xffu32 << k) & 0xff) as i64));
                } else if k == 4 {
                    self.e1(Mn::Swap, Op::A);
                    self.e2(Mn::Anl, Op::A, Op::imm(0xf0));
                } else {
                    // rotate right (8-k) then mask
                    for _ in 0..(8 - k) {
                        self.e1(Mn::Rr, Op::A);
                    }
                    self.e2(Mn::Anl, Op::A, Op::imm(((0xffu32 << k) & 0xff) as i64));
                }
            }
            BinK::ShrU => {
                if k <= 3 {
                    for _ in 0..k {
                        self.e1(Mn::Rr, Op::A);
                    }
                } else if k == 4 {
                    self.e1(Mn::Swap, Op::A);
                } else {
                    for _ in 0..(8 - k) {
                        self.e1(Mn::Rl, Op::A);
                    }
                }
                if k != 0 {
                    self.e2(Mn::Anl, Op::A, Op::imm((0xffu32 >> k) as i64));
                }
            }
            BinK::ShrS => {
                if k == 7 {
                    self.e1(Mn::Rlc, Op::A);
                    self.e2(Mn::Subb, Op::A, Op::dir(A_DIR));
                    return;
                }
                for _ in 0..k {
                    self.e2(Mn::Mov, Op::C, Op::bit(0xE7));
                    self.e1(Mn::Rrc, Op::A);
                }
            }
            _ => unreachable!(),
        }
    }

    /// Shift A by B (counter), B is clobbered.
    fn shift_loop_a(&mut self, op: BinK) {
        let top = self.new_label();
        let test = self.new_label();
        self.e1(Mn::Inc, Op::dir(B_DIR));
        self.jmp(&test);
        self.place_label(top.clone(), false);
        match op {
            BinK::Shl => self.e2(Mn::Add, Op::A, Op::dir(A_DIR)),
            BinK::ShrU => {
                self.e1(Mn::Clr, Op::C);
                self.e1(Mn::Rrc, Op::A);
            }
            _ => {
                self.e2(Mn::Mov, Op::C, Op::bit(0xE7));
                self.e1(Mn::Rrc, Op::A);
            }
        }
        self.place_label(test, false);
        self.e2(Mn::Djnz, Op::dir(B_DIR), Op::label(&top));
        self.st = State::default();
    }

    // ------------------------------------------------------------------
    // Memory access

    fn space_of(&self, m: &Mem) -> PSpace {
        match m {
            Mem::Sym(s, _) => PSpace::S((self.cx.sym_space)(s)),
            Mem::Abs(sp, _) => PSpace::S(*sp),
            Mem::Ptr(_, _, sp) => *sp,
        }
    }

    fn is_direct(&self, m: &Mem) -> bool {
        match m {
            Mem::Sym(s, _) => matches!((self.cx.sym_space)(s), Space::Data | Space::Idata | Space::Sfr),
            Mem::Abs(Space::Data | Space::Sfr | Space::Idata, a) => *a < 0x100 && !(matches!(m, Mem::Abs(Space::Idata, a) if *a >= 0x80)),
            _ => false,
        }
    }

    fn set_dptr_expr(&mut self, e: Expr) {
        if let Some(Dp::E(cur)) = &self.st.dptr {
            if *cur == e {
                return;
            }
            if cur.sym == e.sym && cur.part == Part::Val && e.part == Part::Val {
                let d = e.off - cur.off;
                if (1..=2).contains(&d) {
                    for _ in 0..d {
                        self.e1(Mn::Inc, Op::Dptr);
                    }
                    return;
                }
            }
        }
        self.e2(Mn::Mov, Op::Dptr, Op::Imm(e));
    }

    /// Point DPTR at a memory location plus offset `k`. Returns an extra offset to apply via A (for code
    /// space loads) — always 0 here.
    fn set_dptr_mem(&mut self, m: &Mem, k: i32) {
        match m {
            Mem::Sym(s, o) => {
                let e = self.addr_expr(s, o + k);
                self.set_dptr_expr(e);
            }
            Mem::Abs(_, a) => self.set_dptr_expr(Expr::num((*a as i64 + k as i64) & 0xffff)),
            Mem::Ptr(p, o, _) => {
                let off = o + k;
                match p {
                    Val::K(x) => self.set_dptr_expr(Expr::num((*x + off as i64) & 0xffff)),
                    Val::Addr(s, ao) => {
                        let e = self.addr_expr(s, ao + off);
                        self.set_dptr_expr(e)
                    }
                    Val::R(r) if self.fold.kind[*r as usize] == FoldKind::PtrIdx => {
                        self.set_dptr_idx(*r, off);
                    }
                    Val::R(_) => {
                        let lo = self.src(*p, 0);
                        let hi = self.src(*p, 1);
                        let (Src::L(llo), Src::L(lhi)) = (&lo, &hi) else { panic!("pointer not in a location") };
                        let (olo, ohi) = (loc_op(*llo), loc_op(*lhi));
                        let (klo, khi) = (self.key(&olo), self.key(&ohi));
                        let base_known = matches!(&self.st.dptr, Some(Dp::L(a, b)) if Some(a) == klo.as_ref() && Some(b) == khi.as_ref());
                        // DPTR may already hold base + something; handle simple increments.
                        if !base_known {
                            let dlo = loc_dir(*llo, self.bank);
                            let dhi = loc_dir(*lhi, self.bank);
                            self.e2(Mn::Mov, Op::dir(DPL_DIR), dlo);
                            self.e2(Mn::Mov, Op::dir(DPH_DIR), dhi);
                            if let (Some(a), Some(b)) = (klo, khi) {
                                self.st.dptr = Some(Dp::L(a, b));
                            }
                        }
                        if off > 0 && off <= 3 {
                            for _ in 0..off {
                                self.e1(Mn::Inc, Op::Dptr);
                            }
                        } else if off != 0 {
                            self.e2(Mn::Mov, Op::A, Op::dir(DPL_DIR));
                            self.e2(Mn::Add, Op::A, Op::imm((off & 0xff) as i64));
                            self.e2(Mn::Mov, Op::dir(DPL_DIR), Op::A);
                            self.e2(Mn::Mov, Op::A, Op::dir(DPH_DIR));
                            self.e2(Mn::Addc, Op::A, Op::imm(((off >> 8) & 0xff) as i64));
                            self.e2(Mn::Mov, Op::dir(DPH_DIR), Op::A);
                        }
                    }
                }
            }
        }
        self.uses_dptr = true;
    }

    /// DPTR = base + zext(idx) + off (idx is a folded index).
    fn set_dptr_idx(&mut self, p: VReg, off: i32) {
        let Inst::Bin(BinK::Add, _, base, Val::R(e)) = self.fold.def(self.f, p).clone() else { unreachable!() };
        let Inst::Ext(_, x, _) = self.fold.def(self.f, e).clone() else { unreachable!() };
        let sx = self.src(x, 0);
        match base {
            Val::K(_) | Val::Addr(..) => {
                let (lo, hi) = (self.src(base, 0), self.src(base, 1));
                let blo = self.src_op(&lo).unwrap();
                let bhi = self.src_op(&hi).unwrap();
                let bo = |e: Op, k: i32, part: Part| -> Op {
                    let _ = part;
                    let _ = k;
                    e
                };
                let _ = bo;
                self.load_a(&sx);
                if off != 0 {
                    // Adjust the constant base by the offset.
                    let e = match base {
                        Val::K(k) => Expr::num(k + off as i64),
                        Val::Addr(s, o) => self.addr_expr(&s, o + off),
                        _ => unreachable!(),
                    };
                    self.e2(Mn::Add, Op::A, Op::Imm(e.clone().lo()));
                    self.e2(Mn::Mov, Op::dir(DPL_DIR), Op::A);
                    self.e1(Mn::Clr, Op::A);
                    self.e2(Mn::Addc, Op::A, Op::Imm(e.hi()));
                } else {
                    self.e2(Mn::Add, Op::A, blo);
                    self.e2(Mn::Mov, Op::dir(DPL_DIR), Op::A);
                    self.e1(Mn::Clr, Op::A);
                    self.e2(Mn::Addc, Op::A, bhi);
                }
                self.e2(Mn::Mov, Op::dir(DPH_DIR), Op::A);
            }
            Val::R(_) => {
                let (lo, hi) = (self.src(base, 0), self.src(base, 1));
                self.load_a(&sx);
                let blo = self.src_op(&lo).unwrap();
                let bhi = self.src_op(&hi).unwrap();
                self.e2(Mn::Add, Op::A, blo);
                self.e2(Mn::Mov, Op::dir(DPL_DIR), Op::A);
                self.e1(Mn::Clr, Op::A);
                self.e2(Mn::Addc, Op::A, bhi);
                self.e2(Mn::Mov, Op::dir(DPH_DIR), Op::A);
                if off != 0 {
                    for _ in 0..off.min(3) {
                        self.e1(Mn::Inc, Op::Dptr);
                    }
                    assert!(off <= 3);
                }
            }
        }
        self.st.dptr = None;
    }

    /// Point Rn (R0 or R1) at an internal-RAM location. Returns the register.
    fn set_rptr(&mut self, m: &Mem, k: i32, extra_busy: &[Val]) -> u8 {
        let Mem::Ptr(p, o, _) = m else {
            // Direct symbol in idata above 0x80 etc.
            let busy = self.busy_regs(extra_busy);
            let r = self.pick_ptr_reg(busy).expect("no free pointer register");
            let e = match m {
                Mem::Sym(s, so) => self.addr_expr(s, so + k),
                Mem::Abs(_, a) => Expr::num(*a as i64 + k as i64),
                _ => unreachable!(),
            };
            self.set_reg_imm(r, e);
            self.scratch |= 1 << r;
            return r;
        };
        let off = o + k;
        let mut busy = self.busy_regs(extra_busy);
        // Base pointer already in R0/R1?
        if let Val::R(pr) = p {
            if self.fold.kind[*pr as usize] == FoldKind::None {
                if let Some(Loc::R(r)) = self.locs(*pr).first().copied() {
                    if r <= 1 {
                        let live_after = self.al.live_after[self.cur.0][self.cur.1].contains(*pr as usize);
                        if off == 0 {
                            return r;
                        }
                        if !live_after && busy & (1 << r) == 0 && (1..=3).contains(&off) {
                            for _ in 0..off {
                                self.e1(Mn::Inc, Op::R(r));
                            }
                            return r;
                        }
                        busy |= 1 << r;
                    }
                }
            }
        }
        let r = match self.pick_ptr_reg(busy) {
            Some(r) => r,
            None => panic!("no free pointer register in {}", self.f.name),
        };
        self.scratch |= 1 << r;
        match p {
            Val::K(x) => self.set_reg_imm(r, Expr::num((x + off as i64) & 0xff)),
            Val::Addr(s, ao) => {
                let e = self.addr_expr(s, ao + off);
                self.set_reg_imm(r, e.lo());
            }
            Val::R(pr) if self.fold.kind[*pr as usize] == FoldKind::PtrIdx => {
                let Inst::Bin(BinK::Add, _, base, Val::R(e)) = self.fold.def(self.f, *pr).clone() else { unreachable!() };
                let Inst::Ext(_, x, _) = self.fold.def(self.f, e).clone() else { unreachable!() };
                let sx = self.src(x, 0);
                self.load_a(&sx);
                let sb = self.src(base, 0);
                match sb {
                    Src::K(kk) => self.alu(BinK::Add, &Src::K(((kk as i64 + off as i64) & 0xff) as u8)),
                    Src::E(e) => {
                        let e = if off != 0 {
                            match base {
                                Val::Addr(s, o) => self.addr_expr(&s, o + off).lo(),
                                _ => e,
                            }
                        } else {
                            e
                        };
                        self.e2(Mn::Add, Op::A, Op::Imm(e));
                    }
                    other => {
                        self.alu(BinK::Add, &other);
                        if off != 0 {
                            self.alu(BinK::Add, &Src::K((off & 0xff) as u8));
                        }
                    }
                }
                self.e2(Mn::Mov, Op::R(r), Op::A);
            }
            Val::R(_) => {
                let s = self.src(*p, 0);
                if off == 0 {
                    self.move_byte(Loc::R(r), &s);
                } else if (1..=2).contains(&off) {
                    self.move_byte(Loc::R(r), &s);
                    for _ in 0..off {
                        self.e1(Mn::Inc, Op::R(r));
                    }
                } else {
                    self.load_a(&s);
                    self.alu(BinK::Add, &Src::K((off & 0xff) as u8));
                    self.e2(Mn::Mov, Op::R(r), Op::A);
                }
            }
        }
        r
    }

    fn set_reg_imm(&mut self, r: u8, e: Expr) {
        let c = match e.const_val() {
            Some(v) => Cont::K(v as u8),
            None => Cont::E(e.clone()),
        };
        if self.st.mem.get(&Op::R(r)) == Some(&c) {
            return;
        }
        self.e2(Mn::Mov, Op::R(r), Op::Imm(e));
    }

    /// Load byte `k` of memory `m` into A.
    fn load_mem_a(&mut self, m: &Mem, k: i32) {
        if self.is_direct(m) {
            let l = self.direct_loc(m, k);
            let s = Src::L(l);
            self.load_a(&s);
            if mem_is_volatile(m) {
                self.st.a = None;
            }
            return;
        }
        match self.space_of(m) {
            PSpace::S(Space::Code) => {
                if let Mem::Ptr(Val::R(pr), 0, _) = m {
                    if self.fold.kind[*pr as usize] == FoldKind::PtrIdx && k == 0 {
                        // DPTR = base, A = idx
                        let Inst::Bin(BinK::Add, _, base, Val::R(e)) = self.fold.def(self.f, *pr).clone() else { unreachable!() };
                        let Inst::Ext(_, x, _) = self.fold.def(self.f, e).clone() else { unreachable!() };
                        let sx = self.src(x, 0);
                        let tree_x = matches!(sx, Src::Tree(_));
                        if tree_x {
                            self.load_a(&sx);
                        }
                        let bm = match base {
                            Val::Addr(s, o) => Mem::Sym(s, o),
                            Val::K(kk) => Mem::Abs(Space::Code, kk as u32),
                            v => Mem::Ptr(v, 0, PSpace::S(Space::Code)),
                        };
                        let saved_a = self.st.a.clone();
                        self.set_dptr_mem(&bm, 0);
                        if tree_x {
                            // set_dptr_mem doesn't clobber A for these forms.
                            self.st.a = saved_a;
                        } else {
                            self.load_a(&sx);
                        }
                        self.e2(Mn::Movc, Op::A, Op::AtADptr);
                        return;
                    }
                    if self.fold.kind[*pr as usize] == FoldKind::PtrIdx {
                        let Inst::Bin(BinK::Add, _, base, Val::R(e)) = self.fold.def(self.f, *pr).clone() else { unreachable!() };
                        let Inst::Ext(_, x, _) = self.fold.def(self.f, e).clone() else { unreachable!() };
                        let bm = match base {
                            Val::Addr(s, o) => Mem::Sym(s, o),
                            Val::K(kk) => Mem::Abs(Space::Code, kk as u32),
                            v => Mem::Ptr(v, 0, PSpace::S(Space::Code)),
                        };
                        self.set_dptr_mem(&bm, k);
                        let sx = self.src(x, 0);
                        self.load_a(&sx);
                        self.e2(Mn::Movc, Op::A, Op::AtADptr);
                        return;
                    }
                }
                self.set_dptr_mem(m, k);
                self.e1(Mn::Clr, Op::A);
                self.e2(Mn::Movc, Op::A, Op::AtADptr);
            }
            PSpace::S(Space::Xdata) => {
                self.set_dptr_mem(m, k);
                self.e2(Mn::Movx, Op::A, Op::AtDptr);
            }
            PSpace::S(Space::Pdata) => {
                let busy = self.busy_regs(&[]);
                let _ = busy;
                self.set_dptr_mem(m, k);
                self.e2(Mn::Movx, Op::A, Op::AtDptr);
            }
            PSpace::S(Space::Data | Space::Idata) => {
                let r = self.set_rptr(m, k, &[]);
                self.e2(Mn::Mov, Op::A, Op::AtR(r));
            }
            PSpace::Generic => {
                self.set_gptr(m, k);
                self.e1(Mn::Call, Op::label(&"__gptrget".into()));
                self.st.a = None;
                self.st.c = None;
            }
            PSpace::S(s) => panic!("load from space {:?}", s),
        }
    }

    fn set_gptr(&mut self, m: &Mem, k: i32) {
        let Mem::Ptr(p, o, _) = m else { panic!("generic mem") };
        let off = o + k;
        let lo = self.src(*p, 0);
        let hi = self.src(*p, 1);
        let tag = self.src(*p, 2);
        match (&lo, &hi) {
            (Src::L(a), Src::L(b)) => {
                let (a, b) = (loc_dir(*a, self.bank), loc_dir(*b, self.bank));
                self.e2(Mn::Mov, Op::dir(DPL_DIR), a);
                self.e2(Mn::Mov, Op::dir(DPH_DIR), b);
            }
            _ => {
                let e = match p {
                    Val::K(x) => Expr::num(x & 0xffff),
                    Val::Addr(s, ao) => self.addr_expr(s, *ao),
                    _ => panic!("generic pointer source"),
                };
                self.e2(Mn::Mov, Op::Dptr, Op::Imm(e));
            }
        }
        for _ in 0..off.max(0) {
            self.e1(Mn::Inc, Op::Dptr);
        }
        let tagop = match tag {
            Src::L(l) => loc_dir(l, self.bank),
            s => self.src_op(&s).unwrap(),
        };
        self.e2(Mn::Mov, Op::dir(B_DIR), tagop);
        self.uses_b = true;
        self.uses_dptr = true;
    }

    fn load_bit_c(&mut self, m: &Mem) {
        match m {
            Mem::Abs(Space::Sbit, a) => self.e2(Mn::Mov, Op::C, Op::bit(*a as i64)),
            Mem::Sym(Sym::Global(g), _) => self.e2(Mn::Mov, Op::C, loc_op(Loc::GBit(*g))),
            _ => {
                // A byte holding a bool.
                self.load_mem_a(m, 0);
                self.e1(Mn::Rrc, Op::A);
            }
        }
        if mem_is_volatile(m) {
            self.st.c = None;
        }
    }

    fn gen_load(&mut self, d: VReg, m: &Mem) {
        let ty = self.f.ty(d);
        let dl = self.locs(d).to_vec();
        if dl.is_empty() {
            return;
        }
        if ty == Ty::Bit {
            self.load_bit_c(m);
            self.store_c(dl[0]);
            return;
        }
        let n = ty.bytes() as i32;
        if self.is_direct(m) {
            let mut moves = Vec::new();
            for k in 0..n {
                moves.push((dl[k as usize], Src::L(self.direct_loc(m, k))));
            }
            if mem_is_volatile(m) {
                for (d, s) in moves {
                    self.move_byte(d, &s);
                    self.invalidate_volatile();
                }
            } else {
                self.par_move(moves);
            }
            return;
        }
        match self.space_of(m) {
            PSpace::S(Space::Data | Space::Idata) if n > 1 => {
                // Sequential bytes through @Ri.
                let busy_extra: Vec<Val> = vec![];
                let r = self.set_rptr(m, 0, &busy_extra);
                let base_is_dest = dl.iter().any(|l| *l == Loc::R(r));
                assert!(!base_is_dest, "pointer register overlaps destination");
                for k in 0..n {
                    if k > 0 {
                        self.e1(Mn::Inc, Op::R(r));
                    }
                    let dk = dl[k as usize];
                    if dk.is_reg() {
                        self.e2(Mn::Mov, Op::A, Op::AtR(r));
                        self.store_a(dk);
                    } else {
                        self.e2(Mn::Mov, loc_op(dk), Op::AtR(r));
                    }
                }
                self.restore_rptr(m, r, n - 1);
            }
            PSpace::S(Space::Xdata | Space::Pdata) if n > 1 => {
                for k in 0..n {
                    self.set_dptr_mem(m, k);
                    self.e2(Mn::Movx, Op::A, Op::AtDptr);
                    self.store_a(dl[k as usize]);
                }
            }
            PSpace::S(Space::Code) if n > 1 => {
                self.set_dptr_mem(m, 0);
                for k in 0..n {
                    if k == 0 {
                        self.e1(Mn::Clr, Op::A);
                    } else {
                        self.e2(Mn::Mov, Op::A, Op::imm(k as i64));
                    }
                    self.e2(Mn::Movc, Op::A, Op::AtADptr);
                    self.store_a(dl[k as usize]);
                }
            }
            _ => {
                for k in 0..n {
                    self.load_mem_a(m, k);
                    self.store_a(dl[k as usize]);
                }
            }
        }
    }

    /// If the pointer register is also the base vreg's location, undo increments.
    fn restore_rptr(&mut self, m: &Mem, r: u8, incs: i32) {
        if let Mem::Ptr(Val::R(pr), _, _) = m {
            if self.fold.kind[*pr as usize] == FoldKind::None && self.locs(*pr).first() == Some(&Loc::R(r)) {
                for _ in 0..incs {
                    self.e1(Mn::Dec, Op::R(r));
                }
            }
        }
    }

    fn invalidate_volatile(&mut self) {
        self.st.a = None;
    }

    fn gen_store(&mut self, m: &Mem, v: Val, ty: Ty) {
        if ty == Ty::Bit {
            let bitop = match m {
                Mem::Abs(Space::Sbit, a) => Some(Op::bit(*a as i64)),
                Mem::Sym(Sym::Global(g), _) if (self.cx.sym_space)(&Sym::Global(*g)) == Space::Bit => Some(loc_op(Loc::GBit(*g))),
                _ => None,
            };
            if let Some(b) = bitop {
                match v {
                    Val::K(k) => {
                        let key = self.key(&b);
                        let known = key.as_ref().and_then(|k| self.st.mem.get(k)).cloned();
                        if known != Some(Cont::K((k & 1) as u8)) {
                            self.e1(if k & 1 != 0 { Mn::Setb } else { Mn::Clr }, b);
                        }
                    }
                    _ => {
                        let s = self.src(v, 0);
                        self.load_c_src(&s);
                        self.e2(Mn::Mov, b, Op::C);
                    }
                }
                return;
            }
            // bool byte storage
            let s = self.src(v, 0);
            self.load_c_src(&s);
            self.e1(Mn::Clr, Op::A);
            self.e1(Mn::Rlc, Op::A);
            self.store_mem_from_a(m, 0, &[]);
            return;
        }
        let n = ty.bytes() as i32;
        if self.is_direct(m) {
            let moves: Vec<(Loc, Src)> = (0..n).map(|k| (self.direct_loc(m, k), self.src(v, k as u32))).collect();
            if mem_is_volatile(m) {
                for (d, s) in moves {
                    self.move_byte(d, &s);
                }
            } else {
                self.par_move(moves);
            }
            return;
        }
        let vals = [v];
        match self.space_of(m) {
            PSpace::S(Space::Data | Space::Idata) => {
                let s0 = self.src(v, 0);
                let tree = matches!(s0, Src::Tree(_));
                if tree {
                    self.load_a(&s0);
                    // Pointer setup must not clobber A.
                    let saved = self.st.a.clone();
                    let needs_a = matches!(m, Mem::Ptr(Val::R(pr), _, _) if self.fold.kind[*pr as usize] == FoldKind::PtrIdx);
                    if needs_a {
                        self.e2(Mn::Mov, Op::dir(B_DIR), Op::A);
                        self.uses_b = true;
                    }
                    let r = self.set_rptr(m, 0, &vals);
                    if needs_a {
                        self.e2(Mn::Mov, Op::A, Op::dir(B_DIR));
                    } else {
                        self.st.a = saved;
                    }
                    self.e2(Mn::Mov, Op::AtR(r), Op::A);
                    self.clobber_mem();
                    return;
                }
                let r = self.set_rptr(m, 0, &vals);
                for k in 0..n {
                    if k > 0 {
                        self.e1(Mn::Inc, Op::R(r));
                    }
                    let s = self.src(v, k as u32);
                    match &s {
                        Src::K(_) | Src::E(_) => {
                            let op = self.src_op(&s).unwrap();
                            if self.a_holds(&s) {
                                self.e2(Mn::Mov, Op::AtR(r), Op::A);
                            } else {
                                self.e2(Mn::Mov, Op::AtR(r), op);
                            }
                        }
                        Src::L(l) if !l.is_reg() => {
                            let op = loc_op(*l);
                            self.e2(Mn::Mov, Op::AtR(r), op);
                        }
                        _ => {
                            self.load_a(&s);
                            self.e2(Mn::Mov, Op::AtR(r), Op::A);
                        }
                    }
                    self.clobber_mem();
                }
                self.restore_rptr(m, r, n - 1);
            }
            PSpace::S(Space::Xdata | Space::Pdata) => {
                for k in 0..n {
                    let s = self.src(v, k as u32);
                    if let Src::Tree(_) = s {
                        self.load_a(&s);
                        let saved = self.st.a.clone();
                        let needs_a = self.dptr_setup_needs_a(m);
                        if needs_a {
                            self.e2(Mn::Mov, Op::dir(B_DIR), Op::A);
                            self.uses_b = true;
                        }
                        self.set_dptr_mem(m, k);
                        if needs_a {
                            self.e2(Mn::Mov, Op::A, Op::dir(B_DIR));
                        } else {
                            self.st.a = saved;
                        }
                    } else {
                        self.set_dptr_mem(m, k);
                        self.load_a(&s);
                    }
                    self.e2(Mn::Movx, Op::AtDptr, Op::A);
                }
            }
            PSpace::Generic => {
                for k in 0..n {
                    let s = self.src(v, k as u32);
                    self.load_a(&s);
                    // set_gptr doesn't use A.
                    let saved = self.st.a.clone();
                    self.set_gptr(m, k);
                    self.st.a = saved;
                    self.e1(Mn::Call, Op::label(&"__gptrput".into()));
                    self.st = State::default();
                }
            }
            PSpace::S(s) => panic!("store to space {:?} in {}", s, self.f.name),
        }
    }

    fn dptr_setup_needs_a(&self, m: &Mem) -> bool {
        match m {
            Mem::Ptr(Val::R(pr), off, _) => self.fold.kind[*pr as usize] == FoldKind::PtrIdx || *off > 3,
            _ => false,
        }
    }

    fn store_mem_from_a(&mut self, m: &Mem, k: i32, extra: &[Val]) {
        if self.is_direct(m) {
            let l = self.direct_loc(m, k);
            self.store_a(l);
            return;
        }
        match self.space_of(m) {
            PSpace::S(Space::Data | Space::Idata) => {
                let saved = self.st.a.clone();
                self.e2(Mn::Mov, Op::dir(B_DIR), Op::A);
                let r = self.set_rptr(m, k, extra);
                self.e2(Mn::Mov, Op::A, Op::dir(B_DIR));
                let _ = saved;
                self.e2(Mn::Mov, Op::AtR(r), Op::A);
                self.clobber_mem();
            }
            PSpace::S(Space::Xdata | Space::Pdata) => {
                self.e2(Mn::Mov, Op::dir(B_DIR), Op::A);
                self.set_dptr_mem(m, k);
                self.e2(Mn::Mov, Op::A, Op::dir(B_DIR));
                self.e2(Mn::Movx, Op::AtDptr, Op::A);
            }
            _ => panic!("store_mem_from_a: bad space"),
        }
    }

    // ------------------------------------------------------------------
    // Comparisons

    /// Compute condition into C. Returns true if C holds the *negated* condition.
    fn cmp_to_c(&mut self, c: Cond, a: Val, b: Val, ty: Ty) -> bool {
        let n = ty.bytes();
        if ty == Ty::Bit {
            // Compare bits: eq/ne via xor.
            let sa = self.src(a, 0);
            let sb = self.src(b, 0);
            self.load_c_src(&sa);
            match sb {
                Src::K(k) => {
                    if k & 1 != 0 {
                        self.e1(Mn::Cpl, Op::C);
                    }
                }
                Src::L(l) => {
                    let skip = self.new_label();
                    self.e2(Mn::Jnb, loc_op(l), Op::label(&skip));
                    self.e1(Mn::Cpl, Op::C);
                    self.place_label(skip, true);
                    self.st.c = None;
                }
                _ => panic!("bit compare"),
            }
            // C = a xor b = ne
            return match c {
                Cond::Ne => false,
                Cond::Eq => true,
                _ => panic!("ordered bit compare"),
            };
        }
        match c {
            Cond::Eq | Cond::Ne => {
                // C = (a != b) using subtraction per byte ORed... use A: a xor b, then test.
                let mut first = true;
                let done = self.new_label();
                for k in 0..n {
                    let sa = self.src(a, k);
                    let sb = self.src(b, k);
                    self.load_a(&sa);
                    if let Src::Tree(_) = sb {
                        panic!("tree as second compare operand");
                    }
                    let sbo = self.src_op(&sb).unwrap();
                    let sbo = if let (Op::R(_), Src::L(l)) = (&sbo, &sb) { loc_dir(*l, self.bank) } else { sbo };
                    // cjne sets C = (A < op) but we need C = (A != op): cjne then setb c
                    if first {
                        first = false;
                    }
                    if k + 1 < n {
                        let lbl = done.clone();
                        self.cjne_a(sbo, &lbl);
                    } else {
                        self.cjne_a(sbo, &done);
                    }
                }
                // Equal: C = 0 path falls through; not equal jumps to `ne`.
                let ne = done;
                let end = self.new_label();
                self.e1(Mn::Clr, Op::C);
                self.jmp(&end);
                self.place_label(ne, false);
                self.e1(Mn::Setb, Op::C);
                self.place_label(end, false);
                c == Cond::Eq
            }
            _ => {
                let (c, a, b) = match c {
                    Cond::LeU => (Cond::GeU, b, a),
                    Cond::GtU => (Cond::LtU, b, a),
                    Cond::LeS => (Cond::GeS, b, a),
                    Cond::GtS => (Cond::LtS, b, a),
                    c => (c, a, b),
                };
                // Now c is LtU/GeU/LtS/GeS: compute C = (a < b).
                self.lt_to_c(a, b, ty, c.is_signed());
                matches!(c, Cond::GeU | Cond::GeS)
            }
        }
    }

    fn cjne_a(&mut self, op: Op, l: &Rc<str>) {
        let op = match op {
            Op::R(r) => Op::dir((self.bank * 8 + r) as i64),
            o => o,
        };
        self.e2(Mn::Cjne, Op::A, op);
        // fix: cjne needs 3 operands
        if let Some(Item::Insn(i)) = self.items.last_mut() {
            i.ops.push(Op::label(l));
        }
    }

    /// C = (a < b) (signed or unsigned), for any width.
    fn lt_to_c(&mut self, a: Val, b: Val, ty: Ty, signed: bool) {
        let n = ty.bytes();
        // Constant b: use addition of the negated constant (C = a >= b), then complement.
        if let Val::K(k) = b {
            let k = ty.norm(k);
            if signed && k == 0 {
                // sign bit of a
                let s = self.src(a, n - 1);
                self.load_a(&s);
                self.e1(Mn::Rlc, Op::A);
                return;
            }
            let kk = if signed { ty.norm(k ^ (1i64 << (ty.bits() - 1))) } else { k };
            if kk == 0 {
                // a' >= 0 always: C = 0 (a < 0 false)
                self.e1(Mn::Clr, Op::C);
                return;
            }
            let neg = ty.norm(-kk) as u64;
            let mut started = false;
            for i in 0..n {
                let nb = ((neg >> (8 * i)) & 0xff) as u8;
                if !started && nb == 0 {
                    continue;
                }
                let s = self.src(a, i);
                self.load_a(&s);
                if signed && i == n - 1 {
                    self.e2(Mn::Xrl, Op::A, Op::imm(0x80));
                }
                if !started {
                    self.e2(Mn::Add, Op::A, Op::imm(nb as i64));
                    started = true;
                } else {
                    self.e2(Mn::Addc, Op::A, Op::imm(nb as i64));
                }
            }
            if !started {
                // All bytes of -k are zero only when k == 0 (handled).
                self.e1(Mn::Setb, Op::C);
            }
            // C = (a >= k); complement to get a < k.
            self.e1(Mn::Cpl, Op::C);
            return;
        }
        if signed {
            // Compare with xor 0x80 on the top bytes using B for b's top byte.
            let bt = self.src(b, n - 1);
            let at = self.src(a, n - 1);
            let bt_tree = matches!(bt, Src::Tree(_));
            let at_tree = matches!(at, Src::Tree(_));
            if n == 1 && at_tree && !bt_tree {
                self.load_a(&at);
                self.e2(Mn::Xrl, Op::A, Op::imm(0x80));
                self.e2(Mn::Mov, Op::dir(B_DIR), Op::A);
                self.load_a(&bt);
                self.e2(Mn::Xrl, Op::A, Op::imm(0x80));
                // C = b' < a' ... we need a' < b': compute b' - a' borrow means b' < a'. Use cjne a,b: C = A < B = b' < a'.
                // So swap: compute A = a', B = b' instead.
                self.e2(Mn::Xch, Op::A, Op::dir(B_DIR));
                self.e2(Mn::Cjne, Op::A, Op::dir(B_DIR));
                let l = self.new_label();
                if let Some(Item::Insn(i)) = self.items.last_mut() {
                    i.ops.push(Op::label(&l));
                }
                self.place_label(l, false);
                self.uses_b = true;
                return;
            }
            self.load_a(&bt);
            self.e2(Mn::Xrl, Op::A, Op::imm(0x80));
            self.e2(Mn::Mov, Op::dir(B_DIR), Op::A);
            self.uses_b = true;
            for i in 0..n {
                let s = self.src(a, i);
                self.load_a(&s);
                if i == 0 {
                    self.e1(Mn::Clr, Op::C);
                }
                if i == n - 1 {
                    self.e2(Mn::Xrl, Op::A, Op::imm(0x80));
                    self.e2(Mn::Subb, Op::A, Op::dir(B_DIR));
                } else {
                    let sb = self.src(b, i);
                    let op = self.src_op(&sb).unwrap();
                    self.e2(Mn::Subb, Op::A, op);
                }
            }
            return;
        }
        for i in 0..n {
            let s = self.src(a, i);
            self.load_a(&s);
            let sb = self.src(b, i);
            let op = self.src_op(&sb).expect("tree as second operand");
            if n == 1 {
                // cjne gives C = A < op directly.
                let op = match (&op, &sb) {
                    (Op::R(_), Src::L(l)) => loc_dir(*l, self.bank),
                    _ => op,
                };
                let l = self.new_label();
                self.e2(Mn::Cjne, Op::A, op);
                if let Some(Item::Insn(ins)) = self.items.last_mut() {
                    ins.ops.push(Op::label(&l));
                }
                self.place_label(l, true);
                self.st.c = None;
                return;
            }
            if i == 0 {
                self.e1(Mn::Clr, Op::C);
            }
            self.e2(Mn::Subb, Op::A, op);
        }
    }

    fn cmp_to_a(&mut self, c: Cond, a: Val, b: Val, ty: Ty) {
        let inv = self.cmp_to_c(c, a, b, ty);
        if inv {
            self.e1(Mn::Cpl, Op::C);
        }
        self.e1(Mn::Clr, Op::A);
        self.e1(Mn::Rlc, Op::A);
    }

    // ------------------------------------------------------------------
    // Instructions

    fn gen_inst(&mut self, ins: &Inst) {
        if let Some(d) = ins.def() {
            if self.fold.kind[d as usize] != FoldKind::None {
                return;
            }
            if self.al.locs[d as usize].is_empty() && !matches!(ins, Inst::Call(..)) && !ins.has_side_effects() {
                return;
            }
        }
        match ins {
            Inst::Copy(d, v) => {
                let dl = self.locs(*d).to_vec();
                if self.f.ty(*d) == Ty::Bit {
                    let s = self.src(*v, 0);
                    match s {
                        Src::K(k) => {
                            let op = loc_op(dl[0]);
                            self.e1(if k & 1 != 0 { Mn::Setb } else { Mn::Clr }, op);
                        }
                        _ => {
                            self.load_c_src(&s);
                            self.store_c(dl[0]);
                        }
                    }
                    return;
                }
                let moves: Vec<(Loc, Src)> = dl.iter().enumerate().map(|(k, l)| (*l, self.src(*v, k as u32))).collect();
                if moves.len() == 1 {
                    self.move_byte(moves[0].0, &moves[0].1);
                } else {
                    self.par_move(moves);
                }
            }
            Inst::Trunc(d, v) | Inst::Ext(d, v, _) => {
                let dt = self.f.ty(*d);
                let st = self.vty(*v, Ty::I16);
                let dl = self.locs(*d).to_vec();
                if dt == Ty::Bit {
                    let s = self.src(*v, 0);
                    if st == Ty::Bit {
                        self.load_c_src(&s);
                    } else {
                        self.load_a(&s);
                        self.e1(Mn::Rrc, Op::A);
                    }
                    self.store_c(dl[0]);
                    return;
                }
                if st == Ty::Bit {
                    let s = self.src(*v, 0);
                    self.load_c_src(&s);
                    self.e1(Mn::Clr, Op::A);
                    self.e1(Mn::Rlc, Op::A);
                    self.store_a(dl[0]);
                    for k in 1..dl.len() {
                        self.e1(Mn::Clr, Op::A);
                        self.store_a(dl[k]);
                    }
                    return;
                }
                let signed = matches!(ins, Inst::Ext(_, _, true));
                let sn = st.bytes() as usize;
                if dt.bytes() as usize <= sn || !signed {
                    let mut moves = Vec::new();
                    for k in 0..dl.len() {
                        let s = if k < sn { self.src(*v, k as u32) } else { Src::K(0) };
                        moves.push((dl[k], s));
                    }
                    // Zero bytes last (they may use A = 0).
                    if moves.len() == 1 {
                        self.move_byte(moves[0].0, &moves[0].1);
                    } else {
                        let (nz, z): (Vec<_>, Vec<_>) = moves.into_iter().partition(|m| m.1 != Src::K(0));
                        self.par_move(nz);
                        for (d, s) in z {
                            self.move_byte(d, &s);
                        }
                    }
                    return;
                }
                // Sign extension.
                let mut moves = Vec::new();
                for k in 0..sn {
                    moves.push((dl[k], self.src(*v, k as u32)));
                }
                let top = self.src(*v, sn as u32 - 1);
                if moves.len() == 1 {
                    self.move_byte(moves[0].0, &moves[0].1);
                } else {
                    self.par_move(moves);
                }
                let top = match top {
                    Src::Tree(_) => Src::L(dl[sn - 1]),
                    t => t,
                };
                self.load_a(&top);
                self.e1(Mn::Rlc, Op::A);
                self.e2(Mn::Subb, Op::A, Op::dir(A_DIR));
                for k in sn..dl.len() {
                    self.store_a(dl[k]);
                }
            }
            Inst::Bin(op, d, a, b) => self.gen_bin(*op, *d, *a, *b),
            Inst::Un(op, d, a) => {
                let dl = self.locs(*d).to_vec();
                let n = dl.len() as u32;
                match op {
                    UnK::Not => {
                        if n == 1 && dl[0].is_bit() {
                            let s = self.src(*a, 0);
                            self.load_c_src(&s);
                            self.e1(Mn::Cpl, Op::C);
                            self.store_c(dl[0]);
                            return;
                        }
                        for k in 0..n {
                            let s = self.src(*a, k);
                            if s == Src::L(dl[k as usize]) && !dl[k as usize].is_reg() {
                                // cpl in place is not available for bytes; use xrl dir,#0xff
                                let op = loc_op(dl[k as usize]);
                                self.e2(Mn::Xrl, op, Op::imm(0xff));
                                continue;
                            }
                            self.load_a(&s);
                            self.e1(Mn::Cpl, Op::A);
                            self.store_a(dl[k as usize]);
                        }
                    }
                    UnK::Neg => {
                        if n == 1 {
                            let s = self.src(*a, 0);
                            self.load_a(&s);
                            self.e1(Mn::Cpl, Op::A);
                            self.e1(Mn::Inc, Op::A);
                            self.store_a(dl[0]);
                            return;
                        }
                        // 0 - a
                        for k in 0..n {
                            if k == 0 {
                                self.e1(Mn::Clr, Op::C);
                            }
                            self.e1(Mn::Clr, Op::A);
                            let s = self.src(*a, k);
                            let op = self.src_op(&s).unwrap();
                            self.e2(Mn::Subb, Op::A, op);
                            self.store_a(dl[k as usize]);
                        }
                    }
                }
            }
            Inst::Cmp(c, d, a, b, ty) => {
                let dl = self.locs(*d).to_vec();
                if dl.is_empty() {
                    return;
                }
                let inv = self.cmp_to_c(*c, *a, *b, *ty);
                if inv {
                    self.e1(Mn::Cpl, Op::C);
                }
                if dl[0].is_bit() {
                    self.store_c(dl[0]);
                } else {
                    self.e1(Mn::Clr, Op::A);
                    self.e1(Mn::Rlc, Op::A);
                    self.store_a(dl[0]);
                    for k in 1..dl.len() {
                        self.e1(Mn::Clr, Op::A);
                        self.store_a(dl[k]);
                    }
                }
            }
            Inst::Load(d, m) => self.gen_load(*d, m),
            Inst::Store(m, v, ty) => self.gen_store(m, *v, *ty),
            Inst::Call(d, c, args) => {
                let dl = d.map(|d| self.locs(d).to_vec());
                let rt = d.map(|d| self.f.ty(d));
                self.gen_call_ty(dl, c, args, rt);
            }
            Inst::Asm(text) => self.gen_asm(text),
            Inst::CritEnter(v) => {
                let dl = self.locs(*v).to_vec();
                let l = self.new_label();
                self.e1(Mn::Setb, Op::C);
                self.e2(Mn::Jbc, Op::bit(EA_BIT), Op::label(&l));
                self.e1(Mn::Clr, Op::C);
                self.place_label(l, true);
                self.st.c = None;
                if let Some(d) = dl.first() {
                    self.store_c(*d);
                }
            }
            Inst::CritExit(v) => {
                let s = self.src(Val::R(*v), 0);
                self.load_c_src(&s);
                self.e2(Mn::Mov, Op::bit(EA_BIT), Op::C);
            }
            Inst::MemCopy(d, s, n) => self.gen_memcopy(d, s, *n),
            Inst::MemSet(d, v, n) => self.gen_memset(d, *v, *n),
            Inst::Nop => {}
        }
    }

    fn gen_bin(&mut self, op: BinK, d: VReg, a: Val, b: Val) {
        let dl = self.locs(d).to_vec();
        if dl.is_empty() {
            return;
        }
        let ty = self.f.ty(d);
        let n = ty.bytes();
        if ty == Ty::Bit {
            let s = Src::Tree(d);
            let _ = s;
            // Evaluate like a tree.
            let sa = self.src(a, 0);
            self.load_c_src(&sa);
            let sb = self.src(b, 0);
            match (op, &sb) {
                (BinK::And, Src::L(l)) if l.is_bit() => self.e2(Mn::Anl, Op::C, loc_op(*l)),
                (BinK::Or, Src::L(l)) if l.is_bit() => self.e2(Mn::Orl, Op::C, loc_op(*l)),
                (BinK::Xor, Src::L(l)) if l.is_bit() => {
                    let skip = self.new_label();
                    self.e2(Mn::Jnb, loc_op(*l), Op::label(&skip));
                    self.e1(Mn::Cpl, Op::C);
                    self.place_label(skip, true);
                    self.st.c = None;
                }
                (BinK::And, Src::K(k)) => {
                    if k & 1 == 0 {
                        self.e1(Mn::Clr, Op::C)
                    }
                }
                (BinK::Or, Src::K(k)) => {
                    if k & 1 != 0 {
                        self.e1(Mn::Setb, Op::C)
                    }
                }
                (BinK::Xor, Src::K(k)) => {
                    if k & 1 != 0 {
                        self.e1(Mn::Cpl, Op::C)
                    }
                }
                _ => panic!("unsupported bit operation {:?} {:?}", op, sb),
            }
            self.store_c(dl[0]);
            return;
        }
        if n == 1 {
            let d0 = dl[0];
            let sa = self.src(a, 0);
            let sb = self.src(b, 0);
            // In-place operations.
            if sa == Src::L(d0) {
                match (op, &sb) {
                    (BinK::Add, Src::K(k)) if *k == 1 || *k == 2 || (*k == 0xfe) || *k == 0xff => {
                        let (mn, cnt) = match k {
                            1 => (Mn::Inc, 1),
                            2 => (Mn::Inc, 2),
                            0xff => (Mn::Dec, 1),
                            _ => (Mn::Dec, 2),
                        };
                        if self.a_holds(&sa) && cnt == 1 && !d0.is_reg() {
                            // A holds the value: inc a; mov d, a is 3 bytes vs inc dir 2 bytes.
                        }
                        for _ in 0..cnt {
                            self.e1(mn, loc_op(d0));
                        }
                        return;
                    }
                    (BinK::And | BinK::Or | BinK::Xor, Src::K(k)) => {
                        if self.a_holds(&sa) {
                            // fall through to the A path
                        } else {
                            let mn = match op {
                                BinK::And => Mn::Anl,
                                BinK::Or => Mn::Orl,
                                _ => Mn::Xrl,
                            };
                            let dd = loc_dir(d0, self.bank);
                            self.e2(mn, dd, Op::imm(*k as i64));
                            return;
                        }
                    }
                    _ => {}
                }
            }
            match op {
                BinK::Mul if false => {}
                BinK::Mul => {
                    self.load_b_then_a(&sb, &sa);
                    self.e1(Mn::Mul, Op::AB);
                    self.store_a(d0);
                }
                _ => {
                    self.bin8_to_a(op, a, b);
                    self.store_a(d0);
                }
            }
            return;
        }
        // Multi-byte operations.
        match op {
            BinK::Add | BinK::Sub => self.gen_addsub(op == BinK::Sub, &dl, a, b, ty),
            BinK::And | BinK::Or | BinK::Xor => {
                for k in 0..n {
                    let sa = self.src(a, k);
                    let sb = self.src(b, k);
                    let dk = dl[k as usize];
                    // Constant shortcuts.
                    if let Src::K(kv) = sb {
                        let res = match (op, kv) {
                            (BinK::And, 0) => Some(Src::K(0)),
                            (BinK::And, 0xff) | (BinK::Or, 0) | (BinK::Xor, 0) => Some(sa.clone()),
                            (BinK::Or, 0xff) => Some(Src::K(0xff)),
                            _ => None,
                        };
                        if let Some(r) = res {
                            self.move_byte(dk, &r);
                            continue;
                        }
                        if sa == Src::L(dk) && !self.a_holds(&sa) {
                            let mn = match op {
                                BinK::And => Mn::Anl,
                                BinK::Or => Mn::Orl,
                                _ => Mn::Xrl,
                            };
                            let dd = loc_dir(dk, self.bank);
                            self.e2(mn, dd, Op::imm(kv as i64));
                            continue;
                        }
                    }
                    self.load_a(&sa);
                    self.alu(op, &sb);
                    self.store_a(dk);
                }
            }
            BinK::Shl | BinK::ShrU | BinK::ShrS => match b {
                Val::K(k) => self.gen_shift_const(op, &dl, a, k as u32, ty),
                _ => self.gen_shift_var(op, &dl, a, b, ty),
            },
            BinK::Mul | BinK::DivU | BinK::DivS | BinK::ModU | BinK::ModS => {
                let name = helper_name(op, ty);
                let c = Callee::Runtime(name);
                self.gen_call_ty(Some(dl), &c, &[a, b], Some(ty));
            }
        }
    }

    fn gen_addsub(&mut self, sub: bool, dl: &[Loc], a: Val, b: Val, ty: Ty) {
        let n = ty.bytes() as usize;
        // x += 1 / x -= 1 in place.
        if let Val::K(k) = b {
            let k = ty.norm(k);
            let sa0 = self.src(a, 0);
            let in_place = (0..n).all(|i| self.src(a, i as u32) == Src::L(dl[i]));
            let is_inc = (!sub && k == 1) || (sub && k == ty.mask() as i64);
            let is_dec = (!sub && k == ty.mask() as i64) || (sub && k == 1);
            if in_place && is_inc {
                let end = self.new_label();
                for i in 0..n {
                    self.e1(Mn::Inc, loc_op(dl[i]));
                    if i + 1 < n {
                        match dl[i] {
                            Loc::R(r) => {
                                self.e2(Mn::Cjne, Op::R(r), Op::imm(0));
                                if let Some(Item::Insn(ins)) = self.items.last_mut() {
                                    ins.ops.push(Op::label(&end));
                                }
                            }
                            l => {
                                self.e2(Mn::Mov, Op::A, loc_op(l));
                                self.e1(Mn::Jnz, Op::label(&end));
                            }
                        }
                    }
                }
                self.place_label(end, false);
                return;
            }
            if in_place && is_dec && n == 2 {
                let skip = self.new_label();
                self.load_a(&sa0);
                self.e1(Mn::Jnz, Op::label(&skip));
                self.e1(Mn::Dec, loc_op(dl[1]));
                self.place_label(skip, false);
                self.e1(Mn::Dec, loc_op(dl[0]));
                return;
            }
        }
        let mut carry = false;
        for i in 0..n {
            let sa = self.src(a, i as u32);
            let sb = self.src(b, i as u32);
            if !carry && !sub && sb == Src::K(0) {
                self.move_byte(dl[i], &sa);
                continue;
            }
            if !carry && sub && sb == Src::K(0) {
                self.move_byte(dl[i], &sa);
                continue;
            }
            self.load_a(&sa);
            let op = self.src_op(&sb).expect("complex operand in multi-byte add");
            if sub {
                if !carry {
                    if let Src::K(kk) = sb {
                        // a - k == a + (-k) for the first byte (C inverted) — keep subb for correctness of borrow chain.
                        let _ = kk;
                    }
                    self.e1(Mn::Clr, Op::C);
                }
                self.e2(Mn::Subb, Op::A, op);
            } else if !carry {
                self.e2(Mn::Add, Op::A, op);
            } else {
                self.e2(Mn::Addc, Op::A, op);
            }
            carry = true;
            self.store_a(dl[i]);
        }
    }

    fn gen_shift_const(&mut self, op: BinK, dl: &[Loc], a: Val, k: u32, ty: Ty) {
        let n = ty.bytes() as usize;
        if k == 0 {
            let moves = (0..n).map(|i| (dl[i], self.src(a, i as u32))).collect();
            self.par_move(moves);
            return;
        }
        if k as usize >= n * 8 {
            match op {
                BinK::ShrS => {
                    let top = self.src(a, n as u32 - 1);
                    self.load_a(&top);
                    self.e1(Mn::Rlc, Op::A);
                    self.e2(Mn::Subb, Op::A, Op::dir(A_DIR));
                    for i in 0..n {
                        self.store_a(dl[i]);
                    }
                }
                _ => {
                    for i in 0..n {
                        self.move_byte(dl[i], &Src::K(0));
                    }
                }
            }
            return;
        }
        let bytes = (k / 8) as usize;
        let bits = k % 8;
        // Byte shift into destination (careful with overlaps: go in the right direction).
        match op {
            BinK::Shl => {
                // d[i] = a[i - bytes]
                let mut moves = Vec::new();
                for i in 0..n {
                    let s = if i >= bytes { self.src(a, (i - bytes) as u32) } else { Src::K(0) };
                    moves.push((dl[i], s));
                }
                let (nz, z): (Vec<_>, Vec<_>) = moves.into_iter().partition(|m| m.1 != Src::K(0));
                self.par_move(nz);
                if bits > 0 {
                    // Shift d[bytes..n] left by `bits` using A and carry.
                    for _ in 0..bits {
                        for i in bytes..n {
                            self.load_a(&Src::L(dl[i]));
                            if i == bytes {
                                self.e2(Mn::Add, Op::A, Op::dir(A_DIR));
                            } else {
                                self.e1(Mn::Rlc, Op::A);
                            }
                            self.store_a(dl[i]);
                        }
                    }
                }
                for (d, s) in z {
                    self.move_byte(d, &s);
                }
            }
            BinK::ShrU | BinK::ShrS => {
                let signed = op == BinK::ShrS;
                let mut moves = Vec::new();
                for i in 0..n {
                    let s = if i + bytes < n { self.src(a, (i + bytes) as u32) } else { Src::K(0) };
                    moves.push((dl[i], s));
                }
                if signed && bytes > 0 {
                    // Fill with the sign byte.
                    let (nz, z): (Vec<_>, Vec<_>) = moves.into_iter().partition(|m| m.1 != Src::K(0));
                    self.par_move(nz);
                    let top = Src::L(dl[n - 1 - bytes]);
                    self.load_a(&top);
                    self.e1(Mn::Rlc, Op::A);
                    self.e2(Mn::Subb, Op::A, Op::dir(A_DIR));
                    for (d, _) in z {
                        self.store_a(d);
                    }
                } else {
                    let (nz, z): (Vec<_>, Vec<_>) = moves.into_iter().partition(|m| m.1 != Src::K(0));
                    self.par_move(nz);
                    for (d, s) in z {
                        self.move_byte(d, &s);
                    }
                }
                let top = n - bytes;
                if bits > 0 {
                    for _ in 0..bits {
                        for i in (0..top).rev() {
                            self.load_a(&Src::L(dl[i]));
                            if i == top - 1 {
                                if signed {
                                    self.e2(Mn::Mov, Op::C, Op::bit(0xE7));
                                } else {
                                    self.e1(Mn::Clr, Op::C);
                                }
                            }
                            self.e1(Mn::Rrc, Op::A);
                            self.store_a(dl[i]);
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    fn gen_shift_var(&mut self, op: BinK, dl: &[Loc], a: Val, b: Val, ty: Ty) {
        let n = ty.bytes() as usize;
        // d = a, then loop B times.
        let moves = (0..n).map(|i| (dl[i], self.src(a, i as u32))).collect();
        let sb = self.src(b, 0);
        // Load count into B first if it could be overwritten by the moves.
        self.load_b(&sb);
        self.par_move(moves);
        let top = self.new_label();
        let test = self.new_label();
        self.e1(Mn::Inc, Op::dir(B_DIR));
        self.jmp(&test);
        self.place_label(top.clone(), false);
        match op {
            BinK::Shl => {
                for i in 0..n {
                    self.e2(Mn::Mov, Op::A, loc_op(dl[i]));
                    if i == 0 {
                        self.e2(Mn::Add, Op::A, Op::dir(A_DIR));
                    } else {
                        self.e1(Mn::Rlc, Op::A);
                    }
                    self.e2(Mn::Mov, loc_op(dl[i]), Op::A);
                }
            }
            _ => {
                for i in (0..n).rev() {
                    self.e2(Mn::Mov, Op::A, loc_op(dl[i]));
                    if i == n - 1 {
                        if op == BinK::ShrS {
                            self.e2(Mn::Mov, Op::C, Op::bit(0xE7));
                        } else {
                            self.e1(Mn::Clr, Op::C);
                        }
                    }
                    self.e1(Mn::Rrc, Op::A);
                    self.e2(Mn::Mov, loc_op(dl[i]), Op::A);
                }
            }
        }
        self.place_label(test, false);
        self.e2(Mn::Djnz, Op::dir(B_DIR), Op::label(&top));
        self.st = State::default();
    }

    fn indirect_summary(&self, args: &[Val], ret: Option<Ty>) -> Summary {
        let order = [7u8, 6, 5, 4, 3, 2];
        let mut k = 0;
        let mut params = Vec::new();
        for a in args {
            let t = self.vty(*a, Ty::I16);
            let mut v = Vec::new();
            for _ in 0..t.bytes() {
                if k < order.len() {
                    v.push(Loc::R(order[k]));
                    k += 1;
                }
            }
            params.push(v);
        }
        let ret = match ret {
            None => vec![],
            Some(Ty::Bit) => vec![CARRY],
            Some(Ty::I8) => vec![ACC],
            Some(t) => [7u8, 6, 5, 4].iter().take(t.bytes() as usize).map(|r| Loc::R(*r)).collect(),
        };
        Summary { params, ret, clobbers: ALL_REGS, keeps_b: false, keeps_dptr: false }
    }

    fn gen_call(&mut self, dst: Option<Vec<Loc>>, c: &Callee, args: &[Val]) -> Summary {
        let ret_ty = match &dst {
            Some(d) if d.len() == 1 && d[0].is_bit() => Some(Ty::Bit),
            Some(d) if !d.is_empty() => Some(Ty::from_bytes(d.len() as u32)),
            _ => None,
        };
        self.gen_call_ty(dst, c, args, ret_ty)
    }

    fn gen_call_ty(&mut self, dst: Option<Vec<Loc>>, c: &Callee, args: &[Val], ret_ty: Option<Ty>) -> Summary {
        let s = match c {
            Callee::Indirect(_) => self.indirect_summary(args, ret_ty),
            _ => (self.cx.callee)(c),
        };
        let mut moves: Vec<(Loc, Src)> = Vec::new();
        // Indirect target into DPTR first.
        if let Callee::Indirect(t) = c {
            match t {
                Val::Addr(sym, o) => {
                    let e = self.addr_expr(sym, *o);
                    self.set_dptr_expr(e);
                }
                _ => {
                    let lo = self.src(*t, 0);
                    let hi = self.src(*t, 1);
                    self.move_to_sfr(DPL_DIR, &lo);
                    self.move_to_sfr(DPH_DIR, &hi);
                }
            }
        }
        for (i, a) in args.iter().enumerate() {
            let Some(pl) = s.params.get(i) else { continue };
            for (k, l) in pl.iter().enumerate() {
                moves.push((*l, self.src(*a, k as u32)));
            }
        }
        // Moves into DPL/DPH/B for an indirect call would clobber the target: not supported.
        self.par_move(moves);
        let sym = (self.cx.callee_sym)(c);
        self.e1(Mn::Call, Op::label(&sym));
        // State after call: keep constants in preserved registers.
        let mut keep = HashMap::new();
        for (k, v) in &self.st.mem {
            if let Op::R(r) = k {
                if s.clobbers & (1 << r) == 0 && matches!(v, Cont::K(_) | Cont::E(_)) {
                    keep.insert(k.clone(), v.clone());
                }
            }
        }
        let dp = if s.keeps_dptr { self.st.dptr.clone() } else { None };
        self.st = State::default();
        self.st.mem = keep;
        self.st.dptr = dp;
        if let Some(dl) = dst {
            if dl.is_empty() {
                return s;
            }
            let moves: Vec<(Loc, Src)> = dl
                .iter()
                .enumerate()
                .map(|(k, d)| {
                    let r = s.ret.get(k).copied().unwrap_or(ACC);
                    (*d, if r == ACC { Src::Acc } else if r == CARRY { Src::L(CARRY) } else { Src::L(r) })
                })
                .collect();
            // The byte in ACC must be stored first.
            let (acc, rest): (Vec<_>, Vec<_>) = moves.into_iter().partition(|m| m.1 == Src::Acc);
            for (d, _) in acc {
                self.store_a(d);
            }
            let (carry, rest): (Vec<_>, Vec<_>) = rest.into_iter().partition(|m| m.1 == Src::L(CARRY));
            for (d, _) in carry {
                self.store_c(d);
            }
            self.par_move(rest);
        }
        s
    }

    fn move_to_sfr(&mut self, addr: i64, s: &Src) {
        match s {
            Src::L(l) => {
                let op = loc_dir(*l, self.bank);
                self.e2(Mn::Mov, Op::dir(addr), op);
            }
            Src::Tree(_) | Src::Acc => {
                self.load_a(s);
                self.e2(Mn::Mov, Op::dir(addr), Op::A);
            }
            _ => {
                let op = self.src_op(s).unwrap();
                self.e2(Mn::Mov, Op::dir(addr), op);
            }
        }
    }

    fn gen_memset(&mut self, d: &Mem, v: Val, n: u32) {
        let s = self.src(v, 0);
        if self.is_direct(d) && n <= 6 {
            for k in 0..n as i32 {
                let l = self.direct_loc(d, k);
                self.move_byte(l, &s);
            }
            return;
        }
        match self.space_of(d) {
            PSpace::S(Space::Data | Space::Idata) => {
                let r = self.set_rptr(d, 0, &[v]);
                self.load_a(&s);
                if n <= 4 {
                    for k in 0..n {
                        if k > 0 {
                            self.e1(Mn::Inc, Op::R(r));
                        }
                        self.e2(Mn::Mov, Op::AtR(r), Op::A);
                    }
                } else {
                    let busy = self.busy_regs(&[v]) | (1 << r);
                    let cnt = self.counter_reg(busy);
                    self.set_counter(cnt, n);
                    let top = self.new_label();
                    self.place_label(top.clone(), false);
                    self.e2(Mn::Mov, Op::AtR(r), Op::A);
                    self.e1(Mn::Inc, Op::R(r));
                    self.djnz_counter(cnt, &top);
                }
                self.clobber_mem();
            }
            PSpace::S(Space::Xdata) => {
                self.set_dptr_mem(d, 0);
                let busy = self.busy_regs(&[v]);
                let cnt = self.counter_reg(busy);
                self.set_counter(cnt, n);
                let top = self.new_label();
                self.place_label(top.clone(), false);
                self.load_a(&s);
                self.e2(Mn::Movx, Op::AtDptr, Op::A);
                self.e1(Mn::Inc, Op::Dptr);
                self.djnz_counter(cnt, &top);
                self.st = State::default();
            }
            _ => panic!("memset in unsupported space"),
        }
    }

    fn counter_reg(&mut self, busy: RegSet) -> Option<u8> {
        let r = (2..8u8).rev().find(|r| busy & (1 << r) == 0);
        if let Some(r) = r {
            self.scratch |= 1 << r;
        }
        r
    }

    fn set_counter(&mut self, cnt: Option<u8>, n: u32) {
        match cnt {
            Some(r) => self.e2(Mn::Mov, Op::R(r), Op::imm((n & 0xff) as i64)),
            None => {
                self.e2(Mn::Mov, Op::dir(B_DIR), Op::imm((n & 0xff) as i64));
                self.uses_b = true;
            }
        }
    }

    fn djnz_counter(&mut self, cnt: Option<u8>, l: &Rc<str>) {
        match cnt {
            Some(r) => self.e2(Mn::Djnz, Op::R(r), Op::label(l)),
            None => self.e2(Mn::Djnz, Op::dir(B_DIR), Op::label(l)),
        }
    }

    fn gen_memcopy(&mut self, d: &Mem, s: &Mem, n: u32) {
        if self.is_direct(d) && self.is_direct(s) {
            for k in 0..n as i32 {
                let dl = self.direct_loc(d, k);
                let sl = self.direct_loc(s, k);
                self.move_byte(dl, &Src::L(sl));
            }
            return;
        }
        let dsp = if self.is_direct(d) { PSpace::S(Space::Data) } else { self.space_of(d) };
        let ssp = if self.is_direct(s) { PSpace::S(Space::Data) } else { self.space_of(s) };
        if n <= 2 {
            for k in 0..n as i32 {
                self.load_mem_a(s, k);
                self.store_mem_from_a(d, k, &[]);
            }
            return;
        }
        match (dsp, ssp) {
            (PSpace::S(Space::Data | Space::Idata), PSpace::S(Space::Data | Space::Idata)) => {
                let rs = self.set_rptr(s, 0, &[]);
                self.scratch |= 3;
                let rd = 1 - rs;
                self.e2(Mn::Mov, Op::A, Op::R(rs));
                let _ = rd;
                // Destination pointer into the other register.
                let busy = self.busy_regs(&[]);
                let _ = busy;
                self.set_rptr_forced(d, rd);
                let cnt = self.counter_reg(self.busy_regs(&[]) | 3);
                self.set_counter(cnt, n);
                let top = self.new_label();
                self.place_label(top.clone(), false);
                self.e2(Mn::Mov, Op::A, Op::AtR(rs));
                self.e2(Mn::Mov, Op::AtR(rd), Op::A);
                self.e1(Mn::Inc, Op::R(rs));
                self.e1(Mn::Inc, Op::R(rd));
                self.djnz_counter(cnt, &top);
                self.st = State::default();
            }
            (PSpace::S(Space::Data | Space::Idata), PSpace::S(Space::Code | Space::Xdata)) => {
                let rd = self.set_rptr(d, 0, &[]);
                self.set_dptr_mem(s, 0);
                let cnt = self.counter_reg(self.busy_regs(&[]) | (1 << rd));
                self.set_counter(cnt, n);
                let top = self.new_label();
                self.place_label(top.clone(), false);
                if ssp == PSpace::S(Space::Code) {
                    self.e1(Mn::Clr, Op::A);
                    self.e2(Mn::Movc, Op::A, Op::AtADptr);
                } else {
                    self.e2(Mn::Movx, Op::A, Op::AtDptr);
                }
                self.e2(Mn::Mov, Op::AtR(rd), Op::A);
                self.e1(Mn::Inc, Op::R(rd));
                self.e1(Mn::Inc, Op::Dptr);
                self.djnz_counter(cnt, &top);
                self.st = State::default();
            }
            _ => {
                // Generic per-byte copy.
                for k in 0..n as i32 {
                    self.load_mem_a(s, k);
                    self.store_mem_from_a(d, k, &[]);
                }
            }
        }
    }

    fn set_rptr_forced(&mut self, m: &Mem, r: u8) {
        let e = match m {
            Mem::Sym(s, o) => self.addr_expr(s, *o),
            Mem::Abs(_, a) => Expr::num(*a as i64),
            Mem::Ptr(Val::Addr(s, ao), o, _) => self.addr_expr(s, ao + o),
            Mem::Ptr(Val::K(k), o, _) => Expr::num((k + *o as i64) & 0xff),
            Mem::Ptr(p, o, _) => {
                let s = self.src(*p, 0);
                self.load_a(&s);
                if *o != 0 {
                    self.alu(BinK::Add, &Src::K((*o & 0xff) as u8));
                }
                self.e2(Mn::Mov, Op::R(r), Op::A);
                self.scratch |= 1 << r;
                return;
            }
        };
        self.e2(Mn::Mov, Op::R(r), Op::Imm(e.lo()));
        self.scratch |= 1 << r;
    }

    fn gen_asm(&mut self, text: &str) {
        let resolve = self.cx.asm_resolve;
        let mut p = AsmParser::new(resolve, self.bank);
        p.label_prefix = format!("L{}_asm{}", self.f.id, self.nlabel);
        self.nlabel += 1;
        match p.parse(text) {
            Ok(items) => {
                for it in items {
                    self.items.push(it);
                }
            }
            Err(e) => {
                eprintln!("error in inline assembly of '{}': {}", self.f.name, e);
                std::process::exit(1);
            }
        }
        self.st = State::default();
        self.scratch = ALL_REGS;
        self.uses_b = true;
        self.uses_dptr = true;
    }

    // ------------------------------------------------------------------
    // Terminators

    /// Emit a conditional branch on C (c_true: C=1 means condition true).
    fn branch_c(&mut self, c_true: bool, t: BlockId, f: BlockId, next: Option<BlockId>) {
        let (tl, fl) = (self.block_label(t), self.block_label(f));
        if next == Some(f) {
            self.e1(if c_true { Mn::Jc } else { Mn::Jnc }, Op::label(&tl));
        } else if next == Some(t) {
            self.e1(if c_true { Mn::Jnc } else { Mn::Jc }, Op::label(&fl));
        } else {
            self.e1(if c_true { Mn::Jc } else { Mn::Jnc }, Op::label(&tl));
            self.jmp(&fl);
        }
    }

    fn branch_a(&mut self, nz_true: bool, t: BlockId, f: BlockId, next: Option<BlockId>) {
        let (tl, fl) = (self.block_label(t), self.block_label(f));
        let (jt, jf) = if nz_true { (Mn::Jnz, Mn::Jz) } else { (Mn::Jz, Mn::Jnz) };
        if next == Some(f) {
            self.e1(jt, Op::label(&tl));
        } else if next == Some(t) {
            self.e1(jf, Op::label(&fl));
        } else {
            self.e1(jt, Op::label(&tl));
            self.jmp(&fl);
        }
    }

    fn branch_bit(&mut self, bit: Op, set_true: bool, t: BlockId, f: BlockId, next: Option<BlockId>) {
        let (tl, fl) = (self.block_label(t), self.block_label(f));
        let (jt, jf) = if set_true { (Mn::Jb, Mn::Jnb) } else { (Mn::Jnb, Mn::Jb) };
        if next == Some(f) {
            self.e2(jt, bit, Op::label(&tl));
        } else if next == Some(t) {
            self.e2(jf, bit, Op::label(&fl));
        } else {
            self.e2(jt, bit, Op::label(&tl));
            self.jmp(&fl);
        }
    }

    fn gen_br(&mut self, v: Val, t: BlockId, f: BlockId, next: Option<BlockId>) {
        let ty = self.vty(v, Ty::I16);
        if ty == Ty::Bit {
            match self.src(v, 0) {
                Src::L(l) if l.is_bit() => {
                    self.branch_bit(loc_op(l), true, t, f, next);
                }
                s => {
                    self.load_c_src(&s);
                    self.branch_c(true, t, f, next);
                }
            }
            return;
        }
        if ty == Ty::I8 {
            // Single-bit test?
            if let Val::R(r) = v {
                if self.fold.kind[r as usize] == FoldKind::Acc {
                    if let Inst::Bin(BinK::And, _, x, Val::K(m)) = self.fold.def(self.f, r).clone() {
                        if m != 0 && (m & (m - 1)) == 0 {
                            let bitn = m.trailing_zeros() as i64;
                            let sx = self.src(x, 0);
                            // Bit-addressable location?
                            if let Src::L(Loc::Dir(a)) = sx {
                                if a >= 0x80 && a % 8 == 0 {
                                    self.branch_bit(Op::bit(a as i64 + bitn), true, t, f, next);
                                    return;
                                }
                                if (0x20..0x30).contains(&a) {
                                    self.branch_bit(Op::bit((a as i64 - 0x20) * 8 + bitn), true, t, f, next);
                                    return;
                                }
                            }
                            self.load_a(&sx);
                            self.branch_bit(Op::bit(0xE0 + bitn), true, t, f, next);
                            return;
                        }
                    }
                }
            }
            let s = self.src(v, 0);
            self.load_a(&s);
            self.branch_a(true, t, f, next);
            return;
        }
        let n = ty.bytes();
        let s0 = self.src(v, 0);
        self.load_a(&s0);
        for k in 1..n {
            let s = self.src(v, k);
            let op = self.src_op(&s).unwrap();
            self.e2(Mn::Orl, Op::A, op);
        }
        self.branch_a(true, t, f, next);
    }

    fn gen_cmpbr(&mut self, c: Cond, a: Val, b: Val, ty: Ty, t: BlockId, f: BlockId, next: Option<BlockId>) {
        let n = ty.bytes();
        match c {
            Cond::Eq | Cond::Ne if ty != Ty::Bit => {
                let (eq_b, ne_b) = if c == Cond::Eq { (t, f) } else { (f, t) };
                if b == Val::K(0) {
                    self.gen_br(a, ne_b, eq_b, next);
                    return;
                }
                let ne_l = self.block_label(ne_b);
                let eq_l = self.block_label(eq_b);
                // If the fall-through is the ne block and it's a single byte, use xrl/jz.
                if n == 1 && next == Some(ne_b) {
                    let sa = self.src(a, 0);
                    let sb = self.src(b, 0);
                    if let (Src::L(Loc::R(r)), Src::K(k)) = (&sa, &sb) {
                        // cjne r,#k,skip ; jmp eq ; skip:
                        let _ = (r, k);
                    }
                    self.load_a(&sa);
                    match sb {
                        Src::K(1) => {
                            self.e1(Mn::Dec, Op::A);
                        }
                        Src::K(0xff) => {
                            self.e1(Mn::Inc, Op::A);
                        }
                        _ => self.alu(BinK::Xor, &sb),
                    }
                    self.e1(Mn::Jz, Op::label(&eq_l));
                    return;
                }
                for k in 0..n {
                    let sa = self.src(a, k);
                    let sb = self.src(b, k);
                    match (&sa, &sb) {
                        (Src::L(Loc::R(r)), Src::K(_) | Src::E(_)) if !self.a_holds(&sa) => {
                            let op = self.src_op(&sb).unwrap();
                            self.e2(Mn::Cjne, Op::R(*r), op);
                            if let Some(Item::Insn(i)) = self.items.last_mut() {
                                i.ops.push(Op::label(&ne_l));
                            }
                        }
                        _ => {
                            self.load_a(&sa);
                            let op = match &sb {
                                Src::L(l) => loc_dir(*l, self.bank),
                                s => self.src_op(s).expect("tree as compare operand"),
                            };
                            self.cjne_a(op, &ne_l);
                        }
                    }
                }
                if next != Some(eq_b) {
                    self.jmp(&eq_l);
                }
            }
            _ => {
                // Fast paths for 8-bit unsigned compare against a constant with a register.
                if n == 1 && !c.is_signed() {
                    if let (Src::L(Loc::R(r)), Val::K(k)) = (self.src(a, 0), b) {
                        if matches!(c, Cond::LtU | Cond::GeU) && !self.a_holds(&Src::L(Loc::R(r))) {
                            let l = self.new_label();
                            self.e2(Mn::Cjne, Op::R(r), Op::imm(k));
                            if let Some(Item::Insn(i)) = self.items.last_mut() {
                                i.ops.push(Op::label(&l));
                            }
                            self.place_label(l, true);
                            self.st.c = None;
                            // C = r < k
                            self.branch_c(c == Cond::LtU, t, f, next);
                            return;
                        }
                    }
                }
                if n <= 4 && c.is_signed() && b == Val::K(0) && matches!(c, Cond::LtS | Cond::GeS) {
                    let s = self.src(a, n - 1);
                    if let Src::L(Loc::Dir(addr)) = s {
                        if (0x20..0x30).contains(&addr) {
                            self.branch_bit(Op::bit((addr as i64 - 0x20) * 8 + 7), c == Cond::LtS, t, f, next);
                            return;
                        }
                    }
                    self.load_a(&s);
                    self.branch_bit(Op::bit(0xE7), c == Cond::LtS, t, f, next);
                    return;
                }
                let inv = self.cmp_to_c(c, a, b, ty);
                self.branch_c(!inv, t, f, next);
            }
        }
    }

    fn gen_switch(&mut self, v: Val, ty: Ty, cases: &[(i64, BlockId)], d: BlockId, next: Option<BlockId>) {
        let n = ty.bytes();
        let dl = self.block_label(d);
        if n == 1 {
            let s = self.src(v, 0);
            self.load_a(&s);
            let mut sorted: Vec<(i64, BlockId)> = cases.to_vec();
            sorted.sort();
            // xor/dec chain: A holds (v ^ prev) or (v - prev).
            let mut prev: i64 = 0;
            let mut first = true;
            for (k, b) in &sorted {
                let delta = (k - prev) & 0xff;
                let l = self.block_label(*b);
                if first && *k == 0 {
                    self.e1(Mn::Jz, Op::label(&l));
                } else if delta == 1 {
                    self.e1(Mn::Dec, Op::A);
                    self.e1(Mn::Jz, Op::label(&l));
                } else if delta == 0xff {
                    self.e1(Mn::Inc, Op::A);
                    self.e1(Mn::Jz, Op::label(&l));
                } else {
                    self.e2(Mn::Add, Op::A, Op::imm((-delta) & 0xff));
                    self.e1(Mn::Jz, Op::label(&l));
                }
                prev = *k;
                first = false;
            }
            self.st.a = None;
            if next != Some(d) {
                self.jmp(&dl);
            }
            return;
        }
        for (k, b) in cases {
            let skip = self.new_label();
            for i in 0..n {
                let s = self.src(v, i);
                self.load_a(&s);
                let kb = (k >> (8 * i)) & 0xff;
                self.cjne_a(Op::imm(kb), &skip);
            }
            let l = self.block_label(*b);
            self.jmp(&l);
            self.place_label(skip, false);
        }
        if next != Some(d) {
            self.jmp(&dl);
        }
    }

    fn gen_ret(&mut self, v: Option<Val>) {
        if let Some(v) = v {
            let ty = self.f.ret.unwrap_or(Ty::I8);
            if ty == Ty::Bit {
                let s = self.src(v, 0);
                self.load_c_src(&s);
            } else if self.ret_locs.len() == 1 && self.ret_locs[0] == ACC {
                let s = self.src(v, 0);
                self.load_a(&s);
            } else {
                let mut moves = Vec::new();
                for (k, l) in self.ret_locs.clone().iter().enumerate() {
                    moves.push((*l, self.src(v, k as u32)));
                }
                self.par_move(moves);
            }
        }
        self.epilogue();
    }

    fn epilogue(&mut self) {
        if self.f.attrs.naked {
            return;
        }
        if self.is_isr {
            self.items.push(Item::Comment("@isr_epilogue".into()));
            self.e0(Mn::Reti);
        } else {
            self.e0(Mn::Ret);
        }
    }

    // ------------------------------------------------------------------
    // Function

    pub fn run(mut self, entry_label: Rc<str>) -> FnCode {
        let f = self.f;
        self.items.push(Item::Label(entry_label));
        if self.is_isr {
            self.items.push(Item::Comment("@isr_prologue".into()));
        }
        let order = block_order(f);
        let preds = f.preds();
        for (pos, &b) in order.iter().enumerate() {
            let next = order.get(pos + 1).copied();
            let prev = if pos > 0 { Some(order[pos - 1]) } else { None };
            // Keep state if the only predecessor is the fall-through previous block.
            let keep = pos > 0 && preds[b as usize].len() == 1 && Some(preds[b as usize][0]) == prev && falls_through(&f.blocks[prev.unwrap() as usize].term, b);
            let lbl = self.block_label(b);
            if b == 0 && preds[0].is_empty() {
                self.items.push(Item::Label(lbl));
            } else {
                self.place_label(lbl, keep);
            }
            let blk = &f.blocks[b as usize];
            for (i, ins) in blk.insts.iter().enumerate() {
                self.cur = (b as usize, i);
                self.gen_inst(ins);
            }
            self.cur = (b as usize, blk.insts.len());
            match &blk.term {
                Term::Jmp(t) => {
                    if next != Some(*t) {
                        let l = self.block_label(*t);
                        self.jmp(&l);
                    }
                }
                Term::Br(v, t, e) => self.gen_br(*v, *t, *e, next),
                Term::CmpBr(c, a, bb, ty, t, e) => self.gen_cmpbr(*c, *a, *bb, *ty, *t, *e, next),
                Term::Switch(v, ty, cases, d) => self.gen_switch(*v, *ty, cases, *d, next),
                Term::Ret(v) => self.gen_ret(*v),
                Term::Unreachable => {}
            }
        }
        let clobbers = self.scratch | self.al.used_regs;
        FnCode { items: self.items, clobbers, uses_b: self.uses_b, uses_dptr: self.uses_dptr }
    }
}

fn falls_through(t: &Term, b: BlockId) -> bool {
    match t {
        Term::Jmp(x) => *x == b,
        Term::Br(_, x, y) | Term::CmpBr(_, _, _, _, x, y) => *x == b || *y == b,
        _ => false,
    }
}

/// Order blocks to maximize fall-through.
pub fn block_order(f: &Func) -> Vec<BlockId> {
    let n = f.blocks.len();
    let rpo = f.rpo();
    let mut placed = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let preds = f.preds();
    let mut queue: Vec<BlockId> = rpo.clone();
    queue.reverse();
    let mut cur = Some(0u32);
    while order.len() < rpo.len() {
        let b = match cur {
            Some(b) if !placed[b as usize] => b,
            _ => {
                // next unplaced in RPO
                let mut nb = None;
                for &x in &rpo {
                    if !placed[x as usize] {
                        nb = Some(x);
                        break;
                    }
                }
                match nb {
                    Some(x) => x,
                    None => break,
                }
            }
        };
        placed[b as usize] = true;
        order.push(b);
        // Prefer a successor that has this block as its only (or first) predecessor.
        let succs = match &f.blocks[b as usize].term {
            Term::Jmp(t) => vec![*t],
            Term::Br(_, t, e) | Term::CmpBr(_, _, _, _, t, e) => vec![*e, *t],
            Term::Switch(_, _, _, d) => vec![*d],
            _ => vec![],
        };
        cur = None;
        for s in succs {
            if !placed[s as usize] {
                let _ = &preds;
                cur = Some(s);
                break;
            }
        }
    }
    let _ = queue;
    order
}

pub fn helper_name(op: BinK, ty: Ty) -> &'static str {
    match (op, ty) {
        (BinK::Mul, Ty::I16) => "__mulint",
        (BinK::Mul, Ty::I32) => "__mullong",
        (BinK::DivU, Ty::I16) => "__divuint",
        (BinK::DivS, Ty::I16) => "__divsint",
        (BinK::ModU, Ty::I16) => "__moduint",
        (BinK::ModS, Ty::I16) => "__modsint",
        (BinK::DivU, Ty::I32) => "__divulong",
        (BinK::DivS, Ty::I32) => "__divslong",
        (BinK::ModU, Ty::I32) => "__modulong",
        (BinK::ModS, Ty::I32) => "__modslong",
        (BinK::Mul, Ty::I24) => "__mulint",
        (BinK::Mul, Ty::I64) => "__mullonglong",
        (BinK::DivU, Ty::I64) => "__divulonglong",
        (BinK::DivS, Ty::I64) => "__divslonglong",
        (BinK::ModU, Ty::I64) => "__modulonglong",
        (BinK::ModS, Ty::I64) => "__modslonglong",
        _ => panic!("no helper for {:?} {:?}", op, ty),
    }
}

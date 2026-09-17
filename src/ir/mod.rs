//! Intermediate representation.

pub mod build;
pub mod print;

use crate::ast::{FuncId, GlobalId};
use crate::types::{FuncAttrs, Space};
use std::rc::Rc;

pub type VReg = u32;
pub type BlockId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ty {
    Bit,
    I8,
    I16,
    I24,
    I32,
    I64,
}

impl Ty {
    pub fn bytes(self) -> u32 {
        match self {
            Ty::Bit | Ty::I8 => 1,
            Ty::I16 => 2,
            Ty::I24 => 3,
            Ty::I32 => 4,
            Ty::I64 => 8,
        }
    }
    pub fn bits(self) -> u32 {
        match self {
            Ty::Bit => 1,
            t => t.bytes() * 8,
        }
    }
    pub fn from_bytes(n: u32) -> Ty {
        match n {
            1 => Ty::I8,
            2 => Ty::I16,
            3 => Ty::I24,
            4 => Ty::I32,
            8 => Ty::I64,
            _ => panic!("bad width {}", n),
        }
    }
    pub fn mask(self) -> u64 {
        if self.bits() >= 64 { !0 } else { (1u64 << self.bits()) - 1 }
    }
    /// Normalize a constant to this width (zero-extended representation).
    pub fn norm(self, v: i64) -> i64 {
        ((v as u64) & self.mask()) as i64
    }
    pub fn sext(self, v: i64) -> i64 {
        let b = self.bits();
        if b >= 64 {
            return v;
        }
        let v = (v as u64) & self.mask();
        if (v >> (b - 1)) & 1 == 1 { (v | !self.mask()) as i64 } else { v as i64 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sym {
    Global(GlobalId),
    Func(FuncId),
    /// A memory object in the frame of a function.
    Frame(FuncId, u32),
    /// Runtime library symbol / named symbol.
    Named(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Val {
    R(VReg),
    /// Constant, normalized (zero-extended) to the width of its context.
    K(i64),
    /// Address of a symbol plus offset (16-bit).
    Addr(Sym, i32),
}

impl Val {
    pub fn reg(self) -> Option<VReg> {
        match self {
            Val::R(r) => Some(r),
            _ => None,
        }
    }
    pub fn konst(self) -> Option<i64> {
        match self {
            Val::K(k) => Some(k),
            _ => None,
        }
    }
    pub fn is_const(self) -> bool {
        !matches!(self, Val::R(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinK {
    Add,
    Sub,
    Mul,
    DivS,
    DivU,
    ModS,
    ModU,
    And,
    Or,
    Xor,
    Shl,
    ShrS,
    ShrU,
}

impl BinK {
    pub fn commutative(self) -> bool {
        matches!(self, BinK::Add | BinK::Mul | BinK::And | BinK::Or | BinK::Xor)
    }
    pub fn name(self) -> &'static str {
        match self {
            BinK::Add => "add",
            BinK::Sub => "sub",
            BinK::Mul => "mul",
            BinK::DivS => "divs",
            BinK::DivU => "divu",
            BinK::ModS => "mods",
            BinK::ModU => "modu",
            BinK::And => "and",
            BinK::Or => "or",
            BinK::Xor => "xor",
            BinK::Shl => "shl",
            BinK::ShrS => "shrs",
            BinK::ShrU => "shru",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnK {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cond {
    Eq,
    Ne,
    LtS,
    LeS,
    GtS,
    GeS,
    LtU,
    LeU,
    GtU,
    GeU,
}

impl Cond {
    pub fn negate(self) -> Cond {
        match self {
            Cond::Eq => Cond::Ne,
            Cond::Ne => Cond::Eq,
            Cond::LtS => Cond::GeS,
            Cond::GeS => Cond::LtS,
            Cond::LeS => Cond::GtS,
            Cond::GtS => Cond::LeS,
            Cond::LtU => Cond::GeU,
            Cond::GeU => Cond::LtU,
            Cond::LeU => Cond::GtU,
            Cond::GtU => Cond::LeU,
        }
    }
    /// Condition with swapped operands.
    pub fn swap(self) -> Cond {
        match self {
            Cond::Eq => Cond::Eq,
            Cond::Ne => Cond::Ne,
            Cond::LtS => Cond::GtS,
            Cond::GtS => Cond::LtS,
            Cond::LeS => Cond::GeS,
            Cond::GeS => Cond::LeS,
            Cond::LtU => Cond::GtU,
            Cond::GtU => Cond::LtU,
            Cond::LeU => Cond::GeU,
            Cond::GeU => Cond::LeU,
        }
    }
    pub fn is_signed(self) -> bool {
        matches!(self, Cond::LtS | Cond::LeS | Cond::GtS | Cond::GeS)
    }
    pub fn eval(self, a: i64, b: i64, ty: Ty) -> bool {
        let (ua, ub) = (ty.norm(a) as u64, ty.norm(b) as u64);
        let (sa, sb) = (ty.sext(a), ty.sext(b));
        match self {
            Cond::Eq => ua == ub,
            Cond::Ne => ua != ub,
            Cond::LtS => sa < sb,
            Cond::LeS => sa <= sb,
            Cond::GtS => sa > sb,
            Cond::GeS => sa >= sb,
            Cond::LtU => ua < ub,
            Cond::LeU => ua <= ub,
            Cond::GtU => ua > ub,
            Cond::GeU => ua >= ub,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Cond::Eq => "eq",
            Cond::Ne => "ne",
            Cond::LtS => "lts",
            Cond::LeS => "les",
            Cond::GtS => "gts",
            Cond::GeS => "ges",
            Cond::LtU => "ltu",
            Cond::LeU => "leu",
            Cond::GtU => "gtu",
            Cond::GeU => "geu",
        }
    }
}

/// Target address space of a memory access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PSpace {
    S(Space),
    /// Generic 3-byte pointer (base value is I24 with the tag in the top byte).
    Generic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mem {
    /// Direct access to a symbol (its space is known from the symbol).
    Sym(Sym, i32),
    /// Absolute address in a space.
    Abs(Space, u32),
    /// Indirect access through a pointer value.
    Ptr(Val, i32, PSpace),
}

impl Mem {
    pub fn base_reg(&self) -> Option<VReg> {
        match self {
            Mem::Ptr(Val::R(r), _, _) => Some(*r),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Callee {
    Direct(FuncId),
    Indirect(Val),
    /// Runtime helper.
    Runtime(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inst {
    Copy(VReg, Val),
    Bin(BinK, VReg, Val, Val),
    Un(UnK, VReg, Val),
    /// dst = (a cond b) ? 1 : 0; operands have type `ty`.
    Cmp(Cond, VReg, Val, Val, Ty),
    /// Zero/sign extend to the width of dst.
    Ext(VReg, Val, bool),
    /// Truncate to the width of dst.
    Trunc(VReg, Val),
    Load(VReg, Mem),
    Store(Mem, Val, Ty),
    Call(Option<VReg>, Callee, Vec<Val>),
    Asm(Rc<str>),
    /// Save the interrupt enable flag into a bit vreg and disable interrupts.
    CritEnter(VReg),
    CritExit(VReg),
    /// Copy `n` bytes between memory locations.
    MemCopy(Mem, Mem, u32),
    /// Fill `n` bytes with a value.
    MemSet(Mem, Val, u32),
    Nop,
}

impl Inst {
    pub fn def(&self) -> Option<VReg> {
        match self {
            Inst::Copy(d, _)
            | Inst::Bin(_, d, _, _)
            | Inst::Un(_, d, _)
            | Inst::Cmp(_, d, _, _, _)
            | Inst::Ext(d, _, _)
            | Inst::Trunc(d, _)
            | Inst::Load(d, _)
            | Inst::CritEnter(d) => Some(*d),
            Inst::Call(d, _, _) => *d,
            _ => None,
        }
    }
    pub fn set_def(&mut self, v: VReg) {
        match self {
            Inst::Copy(d, _)
            | Inst::Bin(_, d, _, _)
            | Inst::Un(_, d, _)
            | Inst::Cmp(_, d, _, _, _)
            | Inst::Ext(d, _, _)
            | Inst::Trunc(d, _)
            | Inst::Load(d, _)
            | Inst::CritEnter(d) => *d = v,
            Inst::Call(d, _, _) => *d = Some(v),
            _ => {}
        }
    }
    /// Visit all value operands.
    pub fn for_each_val(&self, mut f: impl FnMut(&Val)) {
        let mem = |m: &Mem, f: &mut dyn FnMut(&Val)| {
            if let Mem::Ptr(b, _, _) = m {
                f(b)
            }
        };
        match self {
            Inst::Copy(_, a) | Inst::Un(_, _, a) | Inst::Ext(_, a, _) | Inst::Trunc(_, a) => f(a),
            Inst::Bin(_, _, a, b) | Inst::Cmp(_, _, a, b, _) => {
                f(a);
                f(b)
            }
            Inst::Load(_, m) => mem(m, &mut f),
            Inst::Store(m, v, _) => {
                mem(m, &mut f);
                f(v)
            }
            Inst::Call(_, c, args) => {
                if let Callee::Indirect(v) = c {
                    f(v)
                }
                for a in args {
                    f(a)
                }
            }
            Inst::CritExit(v) => f(&Val::R(*v)),
            Inst::MemCopy(d, s, _) => {
                mem(d, &mut f);
                mem(s, &mut f)
            }
            Inst::MemSet(d, v, _) => {
                mem(d, &mut f);
                f(v)
            }
            Inst::Asm(_) | Inst::CritEnter(_) | Inst::Nop => {}
        }
    }
    pub fn for_each_val_mut(&mut self, mut f: impl FnMut(&mut Val)) {
        fn mem(m: &mut Mem, f: &mut dyn FnMut(&mut Val)) {
            if let Mem::Ptr(b, _, _) = m {
                f(b)
            }
        }
        match self {
            Inst::Copy(_, a) | Inst::Un(_, _, a) | Inst::Ext(_, a, _) | Inst::Trunc(_, a) => f(a),
            Inst::Bin(_, _, a, b) | Inst::Cmp(_, _, a, b, _) => {
                f(a);
                f(b)
            }
            Inst::Load(_, m) => mem(m, &mut f),
            Inst::Store(m, v, _) => {
                mem(m, &mut f);
                f(v)
            }
            Inst::Call(_, c, args) => {
                if let Callee::Indirect(v) = c {
                    f(v)
                }
                for a in args {
                    f(a)
                }
            }
            Inst::CritExit(_) => {}
            Inst::MemCopy(d, s, _) => {
                mem(d, &mut f);
                mem(s, &mut f)
            }
            Inst::MemSet(d, v, _) => {
                mem(d, &mut f);
                f(v)
            }
            Inst::Asm(_) | Inst::CritEnter(_) | Inst::Nop => {}
        }
    }
    pub fn uses(&self) -> Vec<VReg> {
        let mut v = Vec::new();
        self.for_each_val(|x| {
            if let Val::R(r) = x {
                v.push(*r)
            }
        });
        if let Inst::CritExit(r) = self {
            if !v.contains(r) {
                v.push(*r);
            }
        }
        v
    }
    /// True if the instruction has effects beyond defining its result.
    pub fn has_side_effects(&self) -> bool {
        match self {
            Inst::Store(..) | Inst::Call(..) | Inst::Asm(..) | Inst::CritEnter(..) | Inst::CritExit(..) | Inst::MemCopy(..) | Inst::MemSet(..) => true,
            // Division by zero is UB; treat as pure. Loads from volatile memory are side-effecting.
            Inst::Load(_, m) => mem_is_volatile(m),
            _ => false,
        }
    }
}

thread_local! {
    static VOLATILE_SYMS: std::cell::RefCell<std::collections::HashSet<Sym>> = Default::default();
}

pub fn set_volatile_syms(s: std::collections::HashSet<Sym>) {
    VOLATILE_SYMS.with(|v| *v.borrow_mut() = s);
}

/// Loads with possible side effects: SFRs, xdata through pointers/absolute (memory-mapped I/O), volatile globals.
pub fn mem_is_volatile(m: &Mem) -> bool {
    match m {
        Mem::Sym(s, _) => VOLATILE_SYMS.with(|v| v.borrow().contains(s)),
        Mem::Abs(sp, _) => matches!(sp, Space::Sfr | Space::Sbit | Space::Xdata | Space::Pdata),
        Mem::Ptr(_, _, sp) => matches!(sp, PSpace::S(Space::Xdata) | PSpace::S(Space::Pdata) | PSpace::Generic),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Jmp(BlockId),
    /// Branch if the value is nonzero.
    Br(Val, BlockId, BlockId),
    CmpBr(Cond, Val, Val, Ty, BlockId, BlockId),
    Switch(Val, Ty, Vec<(i64, BlockId)>, BlockId),
    Ret(Option<Val>),
    Unreachable,
}

impl Term {
    pub fn succs(&self) -> Vec<BlockId> {
        match self {
            Term::Jmp(b) => vec![*b],
            Term::Br(_, t, f) | Term::CmpBr(_, _, _, _, t, f) => vec![*t, *f],
            Term::Switch(_, _, cases, d) => {
                let mut v: Vec<BlockId> = cases.iter().map(|c| c.1).collect();
                v.push(*d);
                v
            }
            Term::Ret(_) | Term::Unreachable => vec![],
        }
    }
    pub fn succs_mut(&mut self) -> Vec<&mut BlockId> {
        match self {
            Term::Jmp(b) => vec![b],
            Term::Br(_, t, f) | Term::CmpBr(_, _, _, _, t, f) => vec![t, f],
            Term::Switch(_, _, cases, d) => {
                let mut v: Vec<&mut BlockId> = cases.iter_mut().map(|c| &mut c.1).collect();
                v.push(d);
                v
            }
            Term::Ret(_) | Term::Unreachable => vec![],
        }
    }
    pub fn for_each_val(&self, mut f: impl FnMut(&Val)) {
        match self {
            Term::Br(v, _, _) | Term::Switch(v, _, _, _) => f(v),
            Term::CmpBr(_, a, b, _, _, _) => {
                f(a);
                f(b)
            }
            Term::Ret(Some(v)) => f(v),
            _ => {}
        }
    }
    pub fn for_each_val_mut(&mut self, mut f: impl FnMut(&mut Val)) {
        match self {
            Term::Br(v, _, _) | Term::Switch(v, _, _, _) => f(v),
            Term::CmpBr(_, a, b, _, _, _) => {
                f(a);
                f(b)
            }
            Term::Ret(Some(v)) => f(v),
            _ => {}
        }
    }
    pub fn uses(&self) -> Vec<VReg> {
        let mut v = Vec::new();
        self.for_each_val(|x| {
            if let Val::R(r) = x {
                v.push(*r)
            }
        });
        v
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub insts: Vec<Inst>,
    pub term: Term,
}

#[derive(Clone, Debug)]
pub struct VRegInfo {
    pub ty: Ty,
    pub name: Option<Rc<str>>,
}

#[derive(Clone, Debug)]
pub struct FrameObj {
    pub size: u32,
    pub space: Space,
    pub name: Rc<str>,
    /// Holds the incoming value of this parameter index (aggregate or address-taken parameter).
    pub param: Option<usize>,
}

/// How a parameter is received.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamLoc {
    /// In a vreg.
    Reg(VReg),
    /// Written directly into a frame object by the caller.
    Frame(u32),
}

#[derive(Clone, Debug)]
pub struct Func {
    pub id: FuncId,
    pub name: Rc<str>,
    pub vregs: Vec<VRegInfo>,
    pub blocks: Vec<Block>,
    pub params: Vec<ParamLoc>,
    pub param_tys: Vec<Ty>,
    pub ret: Option<Ty>,
    /// Aggregate return: frame object holding the result.
    pub ret_obj: Option<u32>,
    pub frame: Vec<FrameObj>,
    pub attrs: FuncAttrs,
    pub is_static: bool,
    pub addr_taken: bool,
    pub nooverlay: bool,
    pub variadic: bool,
}

impl Func {
    pub fn new_vreg(&mut self, ty: Ty) -> VReg {
        self.vregs.push(VRegInfo { ty, name: None });
        (self.vregs.len() - 1) as VReg
    }
    pub fn ty(&self, v: VReg) -> Ty {
        self.vregs[v as usize].ty
    }
    pub fn new_block(&mut self) -> BlockId {
        self.blocks.push(Block { insts: Vec::new(), term: Term::Unreachable });
        (self.blocks.len() - 1) as BlockId
    }
    /// Check IR invariants (debugging aid).
    pub fn verify(&self) -> Result<(), String> {
        let n = self.vregs.len() as u32;
        let nb = self.blocks.len() as u32;
        for (bi, b) in self.blocks.iter().enumerate() {
            for (ii, i) in b.insts.iter().enumerate() {
                for u in i.uses() {
                    if u >= n {
                        return Err(format!("{}: b{} i{}: use of %{} out of range ({:?})", self.name, bi, ii, u, i));
                    }
                }
                if let Some(d) = i.def() {
                    if d >= n {
                        return Err(format!("{}: b{} i{}: def of %{} out of range", self.name, bi, ii, d));
                    }
                }
            }
            for u in b.term.uses() {
                if u >= n {
                    return Err(format!("{}: b{} term: use of %{} out of range", self.name, bi, u));
                }
            }
            for s in b.term.succs() {
                if s >= nb {
                    return Err(format!("{}: b{}: successor b{} out of range", self.name, bi, s));
                }
            }
        }
        Ok(())
    }

    pub fn preds(&self) -> Vec<Vec<BlockId>> {
        let mut p = vec![Vec::new(); self.blocks.len()];
        for (i, b) in self.blocks.iter().enumerate() {
            for s in b.term.succs() {
                if !p[s as usize].contains(&(i as BlockId)) {
                    p[s as usize].push(i as BlockId);
                }
            }
        }
        p
    }
    /// Reverse post-order of reachable blocks.
    pub fn rpo(&self) -> Vec<BlockId> {
        let n = self.blocks.len();
        let mut visited = vec![false; n];
        let mut order = Vec::new();
        let mut stack: Vec<(BlockId, usize)> = vec![(0, 0)];
        visited[0] = true;
        while let Some((b, i)) = stack.pop() {
            let succs = self.blocks[b as usize].term.succs();
            if i < succs.len() {
                stack.push((b, i + 1));
                let s = succs[i];
                if !visited[s as usize] {
                    visited[s as usize] = true;
                    stack.push((s, 0));
                }
            } else {
                order.push(b);
            }
        }
        order.reverse();
        order
    }
}

/// Initialized or reserved data object in the final program.
#[derive(Clone, Debug)]
pub struct DataObj {
    pub sym: Sym,
    pub name: Rc<str>,
    pub space: Space,
    pub size: u32,
    pub init: Option<crate::ast::InitData>,
    pub at: Option<u32>,
    pub is_bit: bool,
}

pub struct Module {
    pub funcs: Vec<Option<Func>>,
    pub data: Vec<DataObj>,
}

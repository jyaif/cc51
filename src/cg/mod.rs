//! Code generation for the MCS-51.

pub mod alloc;
pub mod fold;
pub mod select;
pub mod layout;
pub mod outline;
pub mod peep;
pub mod program;

use crate::asm::{Expr, Op};
use crate::ast::FuncId;
use std::rc::Rc;

/// A byte (or bit) storage location for a virtual register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Loc {
    /// Register Rn of the function's register bank.
    R(u8),
    /// Byte slot `n` in the frame of a function.
    Slot(FuncId, u16),
    /// Bit slot `n` in the bit frame of a function.
    BitSlot(FuncId, u16),
    /// Absolute direct address (SFR or fixed RAM).
    Dir(u8),
    /// Absolute bit address.
    BitAbs(u8),
    /// Byte `off` of a global object (direct addressable).
    Glob(usize, u16),
    /// Byte `off` of frame object `obj` of a function.
    Obj(FuncId, u32, u16),
    /// A global bit variable.
    GBit(usize),
}

impl Loc {
    pub fn is_reg(self) -> bool {
        matches!(self, Loc::R(_))
    }
    pub fn is_bit(self) -> bool {
        matches!(self, Loc::BitSlot(..) | Loc::BitAbs(_) | Loc::GBit(_))
    }
    /// A RAM location whose contents can be tracked (not an SFR).
    pub fn trackable(self) -> bool {
        match self {
            Loc::Dir(a) => a < 0x80,
            Loc::BitAbs(a) => a < 0x80,
            _ => true,
        }
    }
}

pub fn slot_sym(f: FuncId, n: u16) -> Rc<str> {
    format!("__s{}_{}", f, n).into()
}
pub fn bit_slot_sym(f: FuncId, n: u16) -> Rc<str> {
    if f == IARGS {
        return format!("__iargb{}", n).into();
    }
    format!("__b{}_{}", f, n).into()
}
pub fn frame_obj_sym(f: FuncId, n: u32) -> Rc<str> {
    if f == IARGS {
        return "__iargs".into();
    }
    format!("__o{}_{}", f, n).into()
}

/// Pseudo function owning the shared argument area of fixed-convention calls
/// (arguments beyond the registers, and bit arguments).
pub const IARGS: FuncId = usize::MAX;

/// Parameter locations of the fixed calling convention (address-taken and recursive functions,
/// and calls through pointers). `None` marks a parameter passed in the callee's frame.
pub fn fixed_param_locs(tys: &[Option<crate::ir::Ty>]) -> Vec<Vec<Loc>> {
    use crate::ir::Ty;
    let order = [7u8, 6, 5, 4, 3, 2];
    let (mut k, mut off, mut bit) = (0usize, 0u16, 0u16);
    let mut params = Vec::new();
    for t in tys {
        let mut v = Vec::new();
        match t {
            None => {}
            Some(Ty::Bit) => {
                v.push(Loc::BitSlot(IARGS, bit));
                bit += 1;
            }
            Some(t) => {
                for _ in 0..t.bytes() {
                    if k < order.len() {
                        v.push(Loc::R(order[k]));
                        k += 1;
                    } else {
                        v.push(Loc::Obj(IARGS, 0, off));
                        off += 1;
                    }
                }
            }
        }
        params.push(v);
    }
    params
}

thread_local! {
    static GLOBAL_NAMES: std::cell::RefCell<Vec<Rc<str>>> = Default::default();
}

pub fn set_global_names(v: Vec<Rc<str>>) {
    GLOBAL_NAMES.with(|g| *g.borrow_mut() = v);
}

pub fn global_sym(g: usize) -> Rc<str> {
    GLOBAL_NAMES.with(|n| n.borrow()[g].clone())
}

/// Operand for a location.
pub fn loc_op(l: Loc) -> Op {
    match l {
        Loc::R(n) => Op::R(n),
        Loc::Slot(f, n) => Op::Dir(Expr::sym(&slot_sym(f, n))),
        Loc::BitSlot(f, n) => Op::Bit(Expr::sym(&bit_slot_sym(f, n))),
        Loc::Dir(a) => Op::dir(a as i64),
        Loc::BitAbs(a) => Op::bit(a as i64),
        Loc::Glob(g, o) => Op::Dir(Expr::sym_off(&global_sym(g), o as i64)),
        Loc::Obj(f, n, o) => Op::Dir(Expr::sym_off(&frame_obj_sym(f, n), o as i64)),
        Loc::GBit(g) => Op::Bit(Expr::sym(&global_sym(g))),
    }
}

/// Direct-address operand for a location (registers via their bank address).
pub fn loc_dir(l: Loc, bank: u8) -> Op {
    match l {
        Loc::R(n) => Op::dir((bank * 8 + n) as i64),
        other => loc_op(other),
    }
}

/// Registers R0..R7 as a bitmask.
pub type RegSet = u8;

pub const ALL_REGS: RegSet = 0xff;

/// Calling-convention and effect summary of a function (or runtime helper).
#[derive(Clone, Debug)]
pub struct Summary {
    /// Byte locations of each parameter (empty for aggregate/memory parameters).
    pub params: Vec<Vec<Loc>>,
    /// Return value byte locations. `Dir(0xE0)` is A, `BitAbs(0xD7)` is the carry flag.
    pub ret: Vec<Loc>,
    /// Registers possibly modified.
    pub clobbers: RegSet,
    /// Preserves B.
    pub keeps_b: bool,
    /// Preserves DPTR.
    pub keeps_dptr: bool,
}

pub const ACC: Loc = Loc::Dir(0xE0);
pub const CARRY: Loc = Loc::BitAbs(0xD7);

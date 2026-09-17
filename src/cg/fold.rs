//! Decides which single-use temporaries are evaluated at their use site (expression trees).

use crate::ir::*;
use crate::types::Space;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldKind {
    /// Not folded: the vreg has a storage location.
    None,
    /// Computed into A (8-bit) or C (bit) right before use.
    Acc,
    /// Load from directly-addressable memory, read at the use site.
    Leaf,
    /// Pointer = base + zero-extended 8-bit index, used as a load/store address.
    PtrIdx,
}

pub struct FoldInfo {
    pub kind: Vec<FoldKind>,
    pub def_at: Vec<(u32, u32)>,
}

impl FoldInfo {
    pub fn is_folded(&self, v: VReg) -> bool {
        self.kind[v as usize] != FoldKind::None
    }
    pub fn def<'a>(&self, f: &'a Func, v: VReg) -> &'a Inst {
        let (b, i) = self.def_at[v as usize];
        &f.blocks[b as usize].insts[i as usize]
    }
}

pub fn direct_mem(m: &Mem) -> bool {
    match m {
        Mem::Sym(Sym::Global(_), _) | Mem::Sym(Sym::Frame(..), _) => true,
        Mem::Abs(Space::Data | Space::Sfr, a) => *a < 0x100,
        _ => false,
    }
}

struct Ctx<'a> {
    f: &'a Func,
    nuses: Vec<u32>,
    ndefs: Vec<u32>,
    def_at: Vec<(u32, u32)>,
    kind: Vec<FoldKind>,
    /// Data-space symbols (to check that leaf loads are non-volatile direct memory).
    mem_space: &'a dyn Fn(&Mem) -> Option<Space>,
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    /// Value computed into A/C.
    Acc,
    /// Direct memory leaf.
    Leaf,
    /// Pointer base for an indexed load/store.
    Ptr,
}

impl<'a> Ctx<'a> {
    fn candidate(&self, v: VReg, b: u32, cursor: usize) -> bool {
        let v = v as usize;
        self.nuses[v] == 1 && self.ndefs[v] == 1 && self.def_at[v].0 == b && cursor > 0 && self.def_at[v].1 as usize == cursor - 1
    }

    /// Try to fold operand `v` (whose def must be at cursor-1) in `role`. Returns true if folded.
    fn try_fold(&mut self, v: Val, role: Role, b: u32, cursor: &mut usize) -> bool {
        let Val::R(r) = v else { return false };
        if !self.candidate(r, b, *cursor) {
            return false;
        }
        let ty = self.f.ty(r);
        let def = self.f.blocks[b as usize].insts[*cursor - 1].clone();
        let ok = match role {
            Role::Leaf => match &def {
                Inst::Load(_, m) => direct_mem(m) && !mem_is_volatile(m) && ty != Ty::Bit,
                _ => false,
            },
            Role::Acc => {
                if ty != Ty::I8 && ty != Ty::Bit {
                    false
                } else {
                    match &def {
                        Inst::Bin(op, _, _, bb) => {
                            if ty == Ty::Bit {
                                matches!(op, BinK::And | BinK::Or | BinK::Xor)
                            } else {
                                // Variable shifts use B as a counter; fine inside a tree only if b is simple.
                                !matches!(op, BinK::DivS | BinK::ModS) || bb.is_const()
                            }
                        }
                        Inst::Un(..) => true,
                        Inst::Load(_, m) => {
                            // Generic pointer loads use a helper; keep them as roots.
                            !matches!(m, Mem::Ptr(_, _, PSpace::Generic))
                        }
                        Inst::Trunc(_, x) => {
                            let xt = match x {
                                Val::R(xr) => self.f.ty(*xr),
                                _ => Ty::I16,
                            };
                            ty == Ty::I8 || xt == Ty::I8 || xt == Ty::Bit || ty == Ty::Bit
                        }
                        Inst::Cmp(..) => true,
                        Inst::Call(_, c, _) => ty == Ty::I8 && !matches!(c, Callee::Runtime(_)) || ty == Ty::Bit,
                        Inst::Copy(_, Val::R(_)) => false,
                        Inst::Ext(..) => ty == Ty::I8 || ty == Ty::Bit,
                        _ => false,
                    }
                }
            }
            Role::Ptr => match &def {
                Inst::Bin(BinK::Add, _, base, Val::R(idx)) if ty == Ty::I16 => {
                    // base must be a constant/address or a simple location; idx must be a zext of an 8-bit value.
                    let base_ok = !matches!(base, Val::R(br) if self.nuses[*br as usize] == 1 && self.def_at[*br as usize].0 == b && self.def_at[*br as usize].1 as usize == *cursor - 2);
                    let idx_def = self.def_at[*idx as usize];
                    base_ok
                        && self.nuses[*idx as usize] == 1
                        && self.ndefs[*idx as usize] == 1
                        && idx_def.0 == b
                        && idx_def.1 as usize + 2 == *cursor
                        && matches!(&self.f.blocks[b as usize].insts[idx_def.1 as usize], Inst::Ext(_, Val::R(x), false) if self.f.ty(*x) == Ty::I8)
                }
                _ => false,
            },
        };
        let _ = &self.mem_space;
        if !ok {
            return false;
        }
        *cursor -= 1;
        self.kind[r as usize] = match role {
            Role::Acc => FoldKind::Acc,
            Role::Leaf => FoldKind::Leaf,
            Role::Ptr => FoldKind::PtrIdx,
        };
        // Recurse into the def's operands.
        match &def {
            Inst::Bin(op, _, a, bb) => {
                if role == Role::Ptr {
                    // idx = zext(x): fold the zext as Acc-like marker, then x may be folded into A.
                    let Val::R(idx) = bb else { unreachable!() };
                    *cursor -= 1;
                    self.kind[*idx as usize] = FoldKind::Acc;
                    if let Inst::Ext(_, x, _) = self.f.blocks[b as usize].insts[*cursor].clone() {
                        self.try_fold(x, Role::Acc, b, cursor);
                    }
                    let _ = a;
                } else {
                    // Secondary operand first (it was evaluated last).
                    let simple_b = !matches!(op, BinK::Shl | BinK::ShrU | BinK::ShrS) || bb.is_const();
                    if simple_b {
                        self.try_fold(*bb, Role::Leaf, b, cursor);
                    }
                    self.try_fold(*a, Role::Acc, b, cursor);
                }
            }
            Inst::Un(_, _, a) | Inst::Ext(_, a, _) | Inst::Trunc(_, a) => {
                let at = match a {
                    Val::R(x) => self.f.ty(*x),
                    _ => Ty::I8,
                };
                if at == Ty::I8 || at == Ty::Bit {
                    self.try_fold(*a, Role::Acc, b, cursor);
                } else {
                    self.try_fold(*a, Role::Leaf, b, cursor);
                }
            }
            Inst::Cmp(_, _, a, bb, cty) => {
                self.try_fold(*bb, Role::Leaf, b, cursor);
                if *cty == Ty::I8 || *cty == Ty::Bit {
                    self.try_fold(*a, Role::Acc, b, cursor);
                } else {
                    self.try_fold(*a, Role::Leaf, b, cursor);
                }
            }
            Inst::Load(_, m) => self.fold_mem(m, b, cursor),
            _ => {}
        }
        true
    }

    fn fold_mem(&mut self, m: &Mem, b: u32, cursor: &mut usize) {
        if let Mem::Ptr(p, off, sp) = m {
            if matches!(sp, PSpace::S(Space::Code) | PSpace::S(Space::Xdata) | PSpace::S(Space::Data) | PSpace::S(Space::Idata)) {
                let off_ok = match sp {
                    PSpace::S(Space::Code) => *off == 0,
                    _ => true,
                };
                if off_ok {
                    self.try_fold(*p, Role::Ptr, b, cursor);
                }
            }
        }
    }

    fn block(&mut self, bi: u32) {
        let blk = &self.f.blocks[bi as usize];
        let n = blk.insts.len();
        let mut cursor = n;
        // Terminator first.
        match blk.term.clone() {
            Term::Br(v, _, _) => {
                self.try_fold(v, Role::Acc, bi, &mut cursor);
            }
            Term::CmpBr(_, a, b, ty, _, _) => {
                self.try_fold(b, Role::Leaf, bi, &mut cursor);
                if ty == Ty::I8 {
                    self.try_fold(a, Role::Acc, bi, &mut cursor);
                } else {
                    self.try_fold(a, Role::Leaf, bi, &mut cursor);
                }
            }
            Term::Switch(v, ty, _, _) if ty == Ty::I8 => {
                self.try_fold(v, Role::Acc, bi, &mut cursor);
            }
            Term::Ret(Some(v)) => {
                let t = match v {
                    Val::R(r) => self.f.ty(r),
                    _ => Ty::I16,
                };
                if t == Ty::I8 || t == Ty::Bit {
                    self.try_fold(v, Role::Acc, bi, &mut cursor);
                }
            }
            _ => {}
        }
        while cursor > 0 {
            let idx = cursor - 1;
            let ins = self.f.blocks[bi as usize].insts[idx].clone();
            if let Some(d) = ins.def() {
                if self.kind[d as usize] != FoldKind::None {
                    // Already folded into a later user; this should not happen since we move the cursor.
                    cursor -= 1;
                    continue;
                }
            }
            cursor = idx;
            match &ins {
                Inst::Bin(op, d, a, b) => {
                    let dt = self.f.ty(*d);
                    if dt == Ty::I8 || dt == Ty::Bit {
                        let simple_b = !matches!(op, BinK::Shl | BinK::ShrU | BinK::ShrS) || b.is_const();
                        if simple_b {
                            self.try_fold(*b, Role::Leaf, bi, &mut cursor);
                        }
                        self.try_fold(*a, Role::Acc, bi, &mut cursor);
                    } else {
                        self.try_fold(*b, Role::Leaf, bi, &mut cursor);
                        self.try_fold(*a, Role::Leaf, bi, &mut cursor);
                    }
                }
                Inst::Un(_, d, a) | Inst::Copy(d, a) | Inst::Ext(d, a, _) | Inst::Trunc(d, a) => {
                    let at = match a {
                        Val::R(x) => self.f.ty(*x),
                        _ => Ty::I16,
                    };
                    let _ = d;
                    if at == Ty::I8 || at == Ty::Bit {
                        self.try_fold(*a, Role::Acc, bi, &mut cursor);
                    } else {
                        self.try_fold(*a, Role::Leaf, bi, &mut cursor);
                    }
                }
                Inst::Cmp(_, _, a, b, ty) => {
                    self.try_fold(*b, Role::Leaf, bi, &mut cursor);
                    if *ty == Ty::I8 || *ty == Ty::Bit {
                        self.try_fold(*a, Role::Acc, bi, &mut cursor);
                    } else {
                        self.try_fold(*a, Role::Leaf, bi, &mut cursor);
                    }
                }
                Inst::Store(m, v, ty) => {
                    // The address is evaluated after the value in the IR.
                    let before = cursor;
                    self.fold_mem(m, bi, &mut cursor);
                    let ptr_folded = cursor != before;
                    if (*ty == Ty::I8 || *ty == Ty::Bit) && !ptr_folded {
                        self.try_fold(*v, Role::Acc, bi, &mut cursor);
                    } else {
                        self.try_fold(*v, Role::Leaf, bi, &mut cursor);
                    }
                }
                Inst::Load(_, m) => {
                    self.fold_mem(m, bi, &mut cursor);
                }
                _ => {}
            }
        }
    }
}

pub fn compute(f: &Func, mem_space: &dyn Fn(&Mem) -> Option<Space>) -> FoldInfo {
    let n = f.vregs.len();
    let mut nuses = vec![0u32; n];
    let mut ndefs = vec![0u32; n];
    let mut def_at = vec![(u32::MAX, 0u32); n];
    for p in &f.params {
        if let ParamLoc::Reg(r) = p {
            ndefs[*r as usize] += 1;
        }
    }
    for (bi, b) in f.blocks.iter().enumerate() {
        for (ii, ins) in b.insts.iter().enumerate() {
            for u in ins.uses() {
                nuses[u as usize] += 1;
            }
            if let Some(d) = ins.def() {
                ndefs[d as usize] += 1;
                def_at[d as usize] = (bi as u32, ii as u32);
            }
        }
        for u in b.term.uses() {
            nuses[u as usize] += 1;
        }
    }
    let mut cx = Ctx { f, nuses, ndefs, def_at, kind: vec![FoldKind::None; n], mem_space };
    for bi in 0..f.blocks.len() {
        cx.block(bi as u32);
    }
    FoldInfo { kind: cx.kind, def_at: cx.def_at }
}

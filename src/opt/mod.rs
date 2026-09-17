//! IR optimization passes.

pub mod bitset;
pub mod cfg;
pub mod combine;
pub mod dataflow;
pub mod dce;
pub mod inline;
pub mod narrow;

use crate::ir::{Callee, Func, Ty};

pub struct OptCtx<'a> {
    pub level: u32,
    /// IR parameter types of a callee (None if unknown).
    pub param_tys: &'a dyn Fn(&Callee) -> Option<Vec<Ty>>,
}

pub fn optimize_func(f: &mut Func, cx: &OptCtx) {
    cfg::simplify(f);
    if cx.level == 0 {
        return;
    }
    for _ in 0..16 {
        let mut changed = false;
        changed |= combine::run(f);
        changed |= dataflow::propagate(f);
        changed |= dce::run(f);
        changed |= cfg::simplify(f);
        if !changed {
            changed |= narrow::run(f, cx.param_tys);
        }
        if !changed {
            break;
        }
    }
}

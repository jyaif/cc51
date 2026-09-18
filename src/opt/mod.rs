//! IR optimization passes.

pub mod bitset;
pub mod cfg;
pub mod combine;
pub mod dataflow;
pub mod dce;
pub mod inline;
pub mod loops;
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
    let check = std::env::var_os("CC51_VERIFY").is_some();
    let verify = |f: &Func, pass: &str| {
        if check {
            if let Err(e) = f.verify() {
                panic!("IR invalid after {}: {}\n{}", pass, e, crate::ir::print::func(f));
            }
        }
    };
    verify(f, "build");
    for _ in 0..16 {
        let mut changed = false;
        changed |= combine::run(f);
        verify(f, "combine");
        changed |= dataflow::propagate(f);
        verify(f, "propagate");
        changed |= dce::run(f);
        changed |= cfg::simplify(f);
        verify(f, "simplify");
        if !changed {
            changed |= narrow::run(f, cx.param_tys);
            verify(f, "narrow");
        }
        if !changed {
            changed |= loops::run(f);
            verify(f, "loops");
        }
        if !changed {
            break;
        }
    }
}

//! IR optimization passes.

pub mod bitset;
pub mod cfg;
pub mod combine;
pub mod dataflow;
pub mod dce;

use crate::ir::Func;

pub fn optimize_func(f: &mut Func, level: u32) {
    cfg::simplify(f);
    if level == 0 {
        return;
    }
    for _ in 0..12 {
        let mut changed = false;
        changed |= combine::run(f);
        changed |= dataflow::propagate(f);
        changed |= dce::run(f);
        changed |= cfg::simplify(f);
        if !changed {
            break;
        }
    }
}

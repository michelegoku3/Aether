//! Startup orchestration: the deferred pass is best-effort by design.
//!
//! The deferred steps touch Steam, the shell and legacy data, so these tests
//! exercise the isolation contract with injected tasks instead of the real
//! migrations. A failing step must never skip the steps after it.

use std::cell::RefCell;

use crate::core::migration::run_isolated;

#[test]
fn a_failing_deferred_step_does_not_stop_later_steps() {
    let order = RefCell::new(Vec::new());

    run_isolated("first", || {
        order.borrow_mut().push("first");
        Ok(())
    });
    run_isolated("second", || {
        order.borrow_mut().push("second");
        Err("simulated failure".to_string())
    });
    run_isolated("third", || {
        order.borrow_mut().push("third");
        Ok(())
    });

    assert_eq!(*order.borrow(), vec!["first", "second", "third"]);
}

#[test]
fn every_deferred_step_runs_exactly_once() {
    let calls = RefCell::new(0);

    run_isolated("only", || {
        *calls.borrow_mut() += 1;
        Ok(())
    });

    assert_eq!(*calls.borrow(), 1);
}

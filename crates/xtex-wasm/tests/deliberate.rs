//! Deliberate runtime failure: proves the TEST step can turn CI red.
//! Reverted before merge.
#[test]
fn deliberate_failure_to_prove_the_gate_can_go_red() {
    let args = std::env::args().count();
    assert_eq!(
        args,
        usize::MAX,
        "this failure exists to prove CI can see one"
    );
}

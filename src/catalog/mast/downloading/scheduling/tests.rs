use super::*;

#[test]
fn whole_block_reservations_wait_without_partially_admitting_a_group() {
    assert!(fits_budget(0, 256, 512));
    assert!(fits_budget(256, 256, 512));
    assert!(!fits_budget(512, 1, 512));
    assert!(!fits_budget(400, 113, 512));
    assert!(fits_budget(0, 113, 512));                                                           // releasing a whole group permits the waiting group
    assert!(!fits_budget(usize::MAX, 1, 512));
}

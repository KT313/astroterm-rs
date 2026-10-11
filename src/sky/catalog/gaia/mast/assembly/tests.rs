use super::*;
use crate::catalog::MastBlock;

pub(super) fn allocations(stars: &StarStorage, bounds: &Vec<f32>, ids: &Vec<u64>) -> [(usize, usize); 10] {
    fn vector<T>(values: &Vec<T>) -> (usize, usize) { (values.as_ptr() as usize, values.capacity()) }
    let c = stars.owned_columns();
    [vector(&c.u0), vector(&c.w), vector(&c.magnitude), vector(&c.brightness_key), vector(&c.distance),
        vector(&c.id), vector(&c.display_color), vector(stars.owned_source_ids().unwrap()), vector(bounds), vector(ids)]
}

#[test]
fn selected_capacity_checks_counts_ordinals_and_empty_inputs_before_allocation() {
    let root = tempfile::tempdir().unwrap();
    let store = BlobStore::new(root.path().into());
    let blob = store.publish("fixture", b"fixture", false).unwrap();
    let mut selection = MastSelection { store, blocks: Vec::new(), discovery: "test".into(), requested: 0, available: 0 };
    assert_eq!(count_selected_rows(&selection).unwrap(), 0);
    let block = MastBlock { key: "fixture".into(), hash: blob.hash, rows: 5, selected: 3, base: 0, stamp: blob.stamp, source: None };
    selection.blocks.push(block.clone());
    selection.requested = 3;
    assert_eq!(count_selected_rows(&selection).unwrap(), 3);                                     // do not reserve the two overfetched suffix rows
    selection.blocks[0].base = u64::from(u32::MAX) - 2;
    assert_eq!(count_selected_rows(&selection).unwrap(), 3);                                     // u32::MAX itself remains a supported ID
    selection.blocks[0].base += 1;
    assert!(count_selected_rows(&selection).is_err());
    selection.blocks[0].base = u64::MAX;
    assert!(count_selected_rows(&selection).is_err());
    selection.blocks[0] = block.clone();
    selection.blocks[0].selected = 6;
    assert!(count_selected_rows(&selection).is_err());
    selection.blocks[0] = block.clone();
    selection.requested = 2;
    assert!(count_selected_rows(&selection).is_err());

    selection.requested = u32::MAX;
    selection.blocks[0].rows = u32::MAX as usize;
    selection.blocks[0].selected = u32::MAX as usize;
    selection.blocks.push(block);                                                              // total exceeds u32 count even though each block's ordinals fit
    assert!(count_selected_rows(&selection).is_err());
}

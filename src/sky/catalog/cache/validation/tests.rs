use super::*;
use crate::progress::NoProgress;
use std::collections::HashSet;

fn check(ids: &[u32]) -> io::Result<IdScratch> {
    let mut scratch = IdScratch::new(ids)?;
    for &id in ids {
        if !scratch.insert(id) { return Err(invalid("duplicate stable star ID")); }
    }
    scratch.finish(&mut NoProgress(io::sink()))?;
    Ok(scratch)
}

#[test]
fn rounded_storage_sizes_choose_the_smaller_exact_representation() {
    for (ids, kind, bytes) in [
        (vec![], "bitmap", 0),
        (vec![0], "sorted u32 copy", 4),
        (vec![0, 63], "bitmap", 8),                                                     // a full word ties the two-ID copy
        (vec![0, 64], "sorted u32 copy", 8),                                            // one more bit would need a second word
        (vec![0, 63, 64, 127], "bitmap", 16),
        (vec![0, 63, 64, 128], "sorted u32 copy", 16),
        (vec![u32::MAX], "sorted u32 copy", 4),
        (vec![0, u32::MAX], "sorted u32 copy", 8),
    ] {
        let scratch = check(&ids).unwrap();
        assert_eq!((scratch.kind(), scratch.bytes()), (kind, bytes), "{ids:?}");
    }
    assert_eq!(storage_sizes(100_000_000, 99_999_999).unwrap(), (12_500_000, 400_000_000)); // calculate large-run requirements without allocating them
    assert_eq!(storage_sizes(1, u32::MAX).unwrap(), (536_870_912, 4));
    assert!(storage_sizes(usize::MAX, 0).is_err());
}

#[test]
fn dense_sparse_and_word_boundary_duplicates_match_the_hash_set_oracle() {
    for count in [0, 1, 2, 63, 64, 65, 128, 513] {
        for sparse in [false, true] {
            let mut ids: Vec<_> = (0..count).map(|id| if sparse { u32::MAX - id * 1_000_003 } else { id }).collect();
            ids.reverse();
            for duplicate in [false, true] {
                let mut input = ids.clone();
                if duplicate && !input.is_empty() { input.push(input[input.len() / 2]); }
                let expected = input.iter().collect::<HashSet<_>>().len() == input.len();
                assert_eq!(check(&input).is_ok(), expected, "count={count}, sparse={sparse}, duplicate={duplicate}");
            }
        }
    }
    assert!(check(&[0, 63, 64, 127, 128, 63]).is_err());
    assert!(check(&[u32::MAX, 0, u32::MAX]).is_err());
}

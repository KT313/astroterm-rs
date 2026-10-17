//! Four texels per SSE2 opacity test: merging skips groups of four transparent inputs, and the lit-row scan
//! sets four column bits at once. Texels that are merged go through the scalar `merge_group_texel` in their
//! original order, so the result is bit-identical. Texel fields are read explicitly; no layout cast is used.
use std::arch::x86_64::*;
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::Texel;
use super::images::merge_group_texel;

/// Merge one group image onto the merged texels, skipping every group of four whose inputs are all transparent.
pub(super) fn merge_group_texels(merged: &mut [Texel], next: &[Texel]) {
    assert_eq!(merged.len(), next.len(), "a group image must match the merged image");
    let batched = next.len() / 4 * 4;
    for (base, four) in merged[..batched].chunks_exact_mut(4).zip(next[..batched].chunks_exact(4)) {
        let mut lit = find_lit_texels(four);
        while lit != 0 {                                                                    // lowest lane first: texel order kept
            let lane = lit.trailing_zeros() as usize;
            lit &= lit - 1;
            merge_group_texel(&mut base[lane], four[lane]);
        }
    }

    // A slice that is not a whole number of groups keeps its last one to three texels on the scalar path.
    for (base, texel) in merged[batched..].iter_mut().zip(&next[batched..]) { merge_group_texel(base, *texel); }
}

/// One bit per lit texel, one word per tile row, as `images::scan_lit_rows_scalar` records them.
pub(super) fn scan_lit_rows(texels: &[Texel], lit_rows: &mut Vec<u64>) {
    const { assert!(TILE_EDGE_TEXELS.is_multiple_of(4) && TILE_EDGE_TEXELS <= 64) };
    lit_rows.clear();
    for row in texels.chunks_exact(TILE_EDGE_TEXELS) {
        let mut lit_columns = 0_u64;
        for (group, four) in row.chunks_exact(4).enumerate() {
            lit_columns |= u64::from(find_lit_texels(four)) << (group * 4);                // bit = column, as in the scalar scan
        }
        lit_rows.push(lit_columns);
    }
}

/// Bit `i` is set when texel `i` of the four holds light: its opacity is not equal to zero, so NaN counts as lit
/// and -0.0 does not, exactly like `!Texel::is_empty`.
#[inline(always)]
fn find_lit_texels(four: &[Texel]) -> u32 {
    // SAFETY: SSE2 is the x86-64 baseline. The intrinsics work on registers only; the four opacities are read as
    // fields (indexing panics on a short slice), never through a pointer into Texel's layout.
    unsafe {
        let opacities = _mm_setr_ps(four[0].opacity, four[1].opacity, four[2].opacity, four[3].opacity);
        _mm_movemask_ps(_mm_cmpneq_ps(opacities, _mm_setzero_ps())) as u32
    }
}

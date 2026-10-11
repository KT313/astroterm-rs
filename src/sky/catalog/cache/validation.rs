//! Exact internal-ID uniqueness checks with scratch bounded by the smaller of a bitmap and a sorted copy.
use crate::{catalog::cache::invalid, progress::DatasetProgress};
use std::{io, mem::size_of};

pub(super) enum IdScratch { Bitmap(Vec<u64>), Sorted(Vec<u32>) }

impl IdScratch {
    pub(super) fn new(ids: &[u32]) -> io::Result<Self> {
        let Some(&maximum) = ids.iter().max() else { return Ok(Self::Bitmap(Vec::new())); };
        let (bitmap_bytes, sorted_bytes) = storage_sizes(ids.len(), maximum)?;
        if bitmap_bytes <= sorted_bytes {
            let words = bitmap_bytes / size_of::<u64>();
            let mut bits = Vec::new();
            bits.try_reserve_exact(words).map_err(io::Error::other)?;
            bits.resize(words, 0);                                                             // one bit records whether this internal ID has already appeared
            Ok(Self::Bitmap(bits))
        } else {
            let mut sorted = Vec::new();
            sorted.try_reserve_exact(ids.len()).map_err(io::Error::other)?;                      // sparse IDs should not allocate a bitmap up to the largest value
            Ok(Self::Sorted(sorted))
        }
    }

    pub(super) fn insert(&mut self, id: u32) -> bool {
        match self {
            Self::Bitmap(bits) => {
                let word = &mut bits[(id / 64) as usize];
                let mask = 1u64 << (id % 64);
                let unseen = *word & mask == 0;
                *word |= mask;
                unseen
            },
            Self::Sorted(ids) => { ids.push(id); true },                                         // duplicates in a sparse catalog are checked together after sorting
        }
    }

    pub(super) fn finish(&mut self, progress: &mut dyn DatasetProgress) -> io::Result<()> {
        if let Self::Sorted(ids) = self {
            progress.set_detail("Sorting and checking sparse internal IDs");
            ids.sort_unstable();
            if ids.windows(2).any(|pair| pair[0] == pair[1]) { return Err(invalid("duplicate stable star ID")); }
        }
        Ok(())
    }

    pub(super) fn kind(&self) -> &'static str { match self { Self::Bitmap(_) => "bitmap", Self::Sorted(_) => "sorted u32 copy" } }
    pub(super) fn bytes(&self) -> usize { match self { Self::Bitmap(bits) => bits.len() * size_of::<u64>(), Self::Sorted(ids) => ids.len() * size_of::<u32>() } }
}

fn storage_sizes(count: usize, maximum: u32) -> io::Result<(usize, usize)> {
    let words = (u64::from(maximum) + 1).div_ceil(64);                                            // widen before adding so u32::MAX remains valid
    let bitmap_bytes = usize::try_from(words).ok().and_then(|words| words.checked_mul(size_of::<u64>()))
        .ok_or_else(|| invalid("internal-ID bitmap size overflow"))?;
    let sorted_bytes = count.checked_mul(size_of::<u32>()).ok_or_else(|| invalid("internal-ID copy size overflow"))?;
    Ok((bitmap_bytes, sorted_bytes))
}

#[cfg(test)]
mod tests;

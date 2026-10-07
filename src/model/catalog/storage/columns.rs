//! The per-star columns, declared once. `StarRowVec` owns one vector per column; `StarRowSlice` borrows every
//! column at once. Prepared disk caches are decoded into the same owned vectors.
use soa_derive::StructOfArray;

/// One prepared star. Every field is plain old data, so each column is a castable cache section.
#[derive(Clone, Copy, Debug, PartialEq, StructOfArray)]
#[soa_derive(Clone, Debug, PartialEq)]
pub struct StarRow {
    pub u0: [f32; 3],          // stored J2000 unit direction
    pub w: [f32; 3],           // normalized motion per Julian year; zero when a precise entry applies
    pub magnitude: f32,
    pub brightness_key: f32,   // conservative brightest magnitude; the sort key within a grid cell
    pub distance: f32,         // parsecs; zero means no usable distance
    pub id: u64,
    pub name: u32,             // zero means absent, otherwise name_table index + 1
    pub designation: [u8; 16],
    pub spectral_type: [u8; 2],
    pub color: f32,
    pub flags: u8,             // bit 0: singular fallback, bit 1: known color
    pub precise_index: u32,    // zero means compact, otherwise precise_motions index + 1
}

/// Cache section numbers of the columns, in declaration order; the side tables follow at 12 and 13.
pub(super) mod section {
    pub const U0: usize = 0;
    pub const W: usize = 1;
    pub const MAGNITUDE: usize = 2;
    pub const BRIGHTNESS_KEY: usize = 3;
    pub const DISTANCE: usize = 4;
    pub const ID: usize = 5;
    pub const NAME: usize = 6;
    pub const DESIGNATION: usize = 7;
    pub const SPECTRAL_TYPE: usize = 8;
    pub const COLOR: usize = 9;
    pub const FLAGS: usize = 10;
    pub const PRECISE_INDEX: usize = 11;
    pub const NAME_TABLE: usize = 12;
    pub const PRECISE_MOTIONS: usize = 13;
}
/// Number of star-storage sections in the cache file; catalog-level sections start here.
pub const STAR_SECTIONS: usize = 14;

crate::rows::row_columns!(StarRow { u0, w, magnitude, brightness_key, distance, id, name, designation, spectral_type, color, flags, precise_index });

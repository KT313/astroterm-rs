//! The per-star columns, declared once. `StarRowVec` owns one vector per column; `StarRowSlice` borrows every
//! column at once. Prepared disk caches are decoded into the same owned vectors.
use soa_derive::StructOfArray;

/// One prepared star. Every field is plain old data, so each column is a castable cache section.
#[derive(Clone, Copy, Debug, PartialEq, StructOfArray)]
#[soa_derive(Clone, Debug, PartialEq)]
pub struct StarRow {
    pub u0: [f32; 3],          // stored J2000 unit direction
    pub w: [f32; 3],           // normalized motion per Julian year
    pub magnitude: u16,       // decode as code / 1000 - 10
    pub brightness_key: u16,   // conservative brightest magnitude; the sort key within a grid cell
    pub distance: f32,         // parsecs; zero means no usable distance
    pub id: u32,
    pub name: u32,             // zero means absent, otherwise label entry (one-based)
    pub display_color: u8,     // index in the shared pixel/terminal palette
}

/// Cache section numbers of the columns, in declaration order; the precise-motion side table follows at 8.
pub(super) mod section {
    pub const U0: usize = 0;
    pub const W: usize = 1;
    pub const MAGNITUDE: usize = 2;
    pub const BRIGHTNESS_KEY: usize = 3;
    pub const DISTANCE: usize = 4;
    pub const ID: usize = 5;
    pub const NAME: usize = 6;
    pub const DISPLAY_COLOR: usize = 7;
    pub const PRECISE_MOTIONS: usize = 8;
}
/// Number of star-storage sections in the cache file; catalog-level sections start here.
pub const STAR_SECTIONS: usize = 9;

crate::rows::row_columns!(StarRow { u0, w, magnitude, brightness_key, distance, id, name, display_color });

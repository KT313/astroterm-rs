//! The per-star columns, declared once. `StarRowVec` owns one vector per column; `StarRowSlice` borrows every
//! column at once. Prepared disk caches are decoded into the same owned vectors.
use soa_derive::StructOfArray;

/// One prepared star. Every field is plain old data, so each column is a castable cache section.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// Numerical/display columns shared by both label representations. No per-star name column lives here.
#[derive(Clone, Copy, Debug, PartialEq, StructOfArray)]
#[soa_derive(Clone, Debug, PartialEq)]
pub struct StarColumns {
    pub u0: [f32; 3],
    pub w: [f32; 3],
    pub magnitude: u16,
    pub brightness_key: u16,
    pub distance: f32,
    pub id: u32,
    pub display_color: u8,
}
pub type StarRowVec = StarColumnsVec;
pub type StarRowSlice<'a> = StarColumnsSlice<'a>;
impl From<StarRow> for StarColumns {
    fn from(row: StarRow) -> Self {
        Self { u0: row.u0, w: row.w, magnitude: row.magnitude, brightness_key: row.brightness_key,
            distance: row.distance, id: row.id, display_color: row.display_color }
    }
}

/// Cache section numbers of the columns, in declaration order; the precise-motion side table follows at 8.
#[cfg(test)]
pub(super) mod section {
    pub const U0: usize = 0;
    pub const ID: usize = 5;
    pub const DISPLAY_COLOR: usize = 7;
}
/// Number of star-storage sections in the cache file; catalog-level sections start here.
pub const STAR_SECTIONS: usize = 9;

crate::rows::row_columns!(StarRow { u0, w, magnitude, brightness_key, distance, id, name, display_color });

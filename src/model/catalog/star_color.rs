//! Shared display palette. Stored indices are validated before a prepared catalog is published.

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StarColor {
    #[default]
    Default = 0,
    HotBlue = 1,
    BlueWhite = 2,
    White = 3,
    YellowWhite = 4,
    Yellow = 5,
    Orange = 6,
    RedOrange = 7,
}

// Index -> pixel RGB. This single palette replaces per-star copies of the colour; the character renderer's
// terminal colours are mapped from the variant in `model::presentation`.
const PALETTE: [[u8; 3]; 8] = [
    [230, 236, 255],
    [155, 185, 255],
    [180, 205, 255],
    [220, 231, 255],
    [248, 245, 235],
    [255, 234, 192],
    [255, 192, 125],
    [255, 142, 91],
];
impl StarColor {
    pub fn from_index(index: u8) -> Option<Self> {
        Some(match index {
            0 => Self::Default, 1 => Self::HotBlue, 2 => Self::BlueWhite, 3 => Self::White,
            4 => Self::YellowWhite, 5 => Self::Yellow, 6 => Self::Orange, 7 => Self::RedOrange,
            _ => return None,
        })
    }
    pub fn index(self) -> u8 { self as u8 }
    pub fn rgb(self) -> [u8; 3] { PALETTE[self as usize] }
    /// RGB of a stored palette index without building the enum; panics on an index the catalog would have rejected.
    pub fn rgb_of(index: u8) -> [u8; 3] { PALETTE[usize::from(index)] }
}

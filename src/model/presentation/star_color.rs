//! Shared display palette. Stored indices are validated before a prepared catalog is published.
use crate::canvas::Color;

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

// Index -> (pixel RGB, character-terminal color). This single palette replaces per-star copies of both colors.
const PALETTE: [([u8; 3], Option<Color>); 8] = [
    ([230, 236, 255], None),
    ([155, 185, 255], Some(Color::Cyan)),
    ([180, 205, 255], Some(Color::Cyan)),
    ([220, 231, 255], None),
    ([248, 245, 235], None),
    ([255, 234, 192], None),
    ([255, 192, 125], Some(Color::Yellow)),
    ([255, 142, 91], Some(Color::Red)),
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
    pub fn rgb(self) -> [u8; 3] { PALETTE[self as usize].0 }
    /// RGB of a stored palette index without building the enum; panics on an index the catalog would have rejected.
    pub fn rgb_of(index: u8) -> [u8; 3] { PALETTE[usize::from(index)].0 }
    pub fn terminal_color(self) -> Option<Color> { PALETTE[self as usize].1 }
}

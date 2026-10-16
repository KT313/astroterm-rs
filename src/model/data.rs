//! Runtime table and cache definitions. Implementations live in model/catalog and the frame-step modules.
//!
//! Every ordinary star belongs to exactly one region and speed group. A region's array index identifies its
//! fixed patch of sky. Group indices run from slowest to fastest; shared thresholds and hold-time settings stay
//! in `constants.rs`. Catalog columns remain immutable after loading; calculated samples and tiles are mutable.

use crate::formulas::Vector3;
use crate::catalog::{MagnitudeClipping, StarNames};
use crate::constants::SPEED_GROUP_COUNT;
use crate::formulas::BodyId;
use crate::model::Constellation;
use soa_derive::StructOfArray;

/// Catalog tables and their associated stellar caches, owned together.
#[derive(Clone, Debug, PartialEq)]
pub struct Data {
    pub solar_system_sun: SunTable,
    pub solar_system_planets: PlanetsTable,
    pub solar_system_moons: MoonsTable,

    pub constellation_stars: StarTable,
    pub constellation_last_update_ms: Option<i64>, // simulation timestamp in milliseconds of the constellation samples; None until calculated
    pub constellation_simulation: SimulationCache, // always considered; uses the fast group's hold time
    pub constellation_figures: Vec<Constellation>, // line endpoints index rows of constellation_stars

    pub star_regions: Box<[Region]>,              // exactly CELL_COUNT = 6 * 4^GRID_DEPTH entries, allocated on the heap
    pub names: StarNames,                        // shared text for the name references in star tables
    pub star_opacities: StarOpacities,            // shared drawing curve for the current zoom
    pub tile_bytes: usize,                       // maintained when group or region images change
    pub clipping: MagnitudeClipping,             // counts used by the catalog brightness-clipping notice
}

/// The solar-system tables identify the bodies evaluated by the astronomy models. They introduce no new cache.
#[derive(Clone, Debug, PartialEq)]
pub struct SunTable {
    pub sun: Body,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanetsTable {
    pub planets: Vec<Body>,                      // includes Earth, whose state is needed by the observer
}

#[derive(Clone, Debug, PartialEq)]
pub struct MoonsTable {
    pub moons: Vec<Satellite>,                   // each entry also identifies the body it orbits
}

/// The Sun or a planet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Body {
    pub id: BodyId,
    pub name: &'static str,
}

/// A moon and the body it orbits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Satellite {
    pub id: BodyId,
    pub primary: BodyId,
    pub name: &'static str,
}

/// One fixed sky patch. All regions use the same SPEED_GROUP_BOUNDS; thresholds are not copied per region.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub speed_groups: [StarGroup; SPEED_GROUP_COUNT], // SPEED_GROUP_COUNT = SPEED_GROUP_BOUNDS.len() + 1
    pub merged_tile: RegionTileCache,
}

/// One group's original stars, calculated samples and rendered image, with no separate flat cache arrays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StarGroup {
    pub ttl_ms: i64,                              // simulated milliseconds; shorter for faster groups, assigned at startup
    pub last_update_ms: Option<i64>,              // simulation timestamp in milliseconds of this group's samples; None until calculated
    pub stars: StarTable,
    pub simulation: SimulationCache,
    pub tile: GroupTileCache,
}

/// One table's original star data, ordered by conservative brightness key, brightest first.
/// Every column and the chosen label vector have the same length; a row index identifies one star across them.
#[derive(Clone, Debug, PartialEq)]
pub struct StarTable {
    pub columns: StarColumns,
    pub labels: StarLabels,
}

/// One star's numerical fields. StructOfArray generates one Vec per field, preserving column-based storage.
/// StarColumns below is that generated owning table; StarColumnSlice is its borrowed view.
#[derive(Clone, Copy, Debug, PartialEq, StructOfArray)]
#[soa_derive(Clone, Debug, PartialEq)]
pub struct StarColumn {
    pub u0: [f32; 3],                        // original J2000 unit direction
    pub w: [f32; 3],                        // normalized motion per Julian year
    pub magnitude: u16,                     // original magnitude; decode as code / 1000.0 - 10.0
    pub brightness_key: u16,                // conservative brightest magnitude, encoded the same way
    pub distance: f32,                      // parsecs; zero means no usable distance
    pub id: u32,                            // internal star ID, distinct from Gaia's source ID
    pub display_color: u8,                  // index into the shared color palette
}
pub type StarColumns = StarColumnVec;
pub type StarRowVec = StarColumns;
pub type StarRowSlice<'a> = StarColumnSlice<'a>;

/// Stored catalogs reference shared names; Gaia retains source IDs for labels generated only when displayed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StarLabels {
    Names(Vec<u32>),                            // one-based entries into Data.names; zero means no name
    SourceIds(Vec<u64>),                        // original Gaia IDs, rendered as "Gaia DR3 <id>"
}

/// Calculated samples for the first N rows of one star table; both vectors have length N.
///
/// Key: the owning StarGroup.last_update_ms, or Data.constellation_last_update_ms for constellation stars.
/// This is the simulation timestamp in milliseconds for every sample, not wall-clock time. Refresh when absent or when
/// the absolute timestamp difference exceeds the TTL in milliseconds. Ordinary groups use StarGroup.ttl_ms; constellation
/// stars use the fastest configured TTL. The owning timestamp and these samples must be updated together.
/// If more rows are needed within that hold, append them at the stored date without changing last_update_ms.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimulationCache {
    pub directions: Vec<Vector3>,
    pub magnitudes: Vec<u16>,                   // current magnitude codes; u16::MAX means unplaceable
}

/// Drawing inputs that determine a group's image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupTileKey {
    pub simulated_at_ms: i64,                      // StarGroup.last_update_ms after samples have been calculated
    pub star_count: usize,                      // number of leading table rows considered for drawing
    pub magnitude_threshold: u16,
    pub zoom_boost: f64,
}

/// Cached group image. Key: its simulation date, required row count, threshold and zoom boost.
/// Redraw when any key field changes; None means never rendered. No independent image TTL.
/// An empty group uses a canonical empty key, defined by GroupTileKey::EMPTY.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupTileCache {
    pub key: Option<GroupTileKey>,
    pub tile: Tile,
}

/// Cached region image, merged slowest group first. Key: the group keys used by the last merge.
/// Rebuild if absent or any group key differs. Keep the image while all keys match, even outside the view.
/// Lit rows and content radius are rebuilt with the image; neither has an independent time or view key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegionTileCache {
    pub group_keys: Option<[Option<GroupTileKey>; SPEED_GROUP_COUNT]>,
    pub tile: Tile,
    pub lit_rows: Vec<u64>,                     // one bit per texel in each row, allowing projection to skip empty areas
    pub content_radius: Option<f64>,            // texels from the tile centre, including filter support; None until merged, zero for no light
}

/// A sky-fixed image: TILE_EDGE_TEXELS squared entries in row order, or no entries for an empty group/region.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tile {
    pub texels: Vec<Texel>,
}

/// Stored floating-point RGB contributions and opacity, matching the existing tile representation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Texel {
    pub rgb: [f32; 3],
    pub opacity: f32,
}

/// Cached per-magnitude opacity curve. Key: zoom_boost; rebuild when it changes or the table is uninitialized.
/// Shared by tile stars and constellation stars so the same magnitude receives the same opacity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StarOpacities {
    pub(crate) zoom_boost: f64,
    pub(crate) opacities: Vec<f32>,
}

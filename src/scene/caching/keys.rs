//! Exact raster inputs, rather than copies of observed records. Structural comparison also handles public
//! reference skies whose metadata or geometry changes without a production cache generation changing.
use crate::model::{ProjectedSky, PreparedScene, RenderOptions};
use crate::scene::{format_star_label, select_dynamically_named_stars};
use crate::scene::raster::appearance::select_star_appearance_prepared;
use crate::scene::resolve_star_rgb;

use crate::model::{SceneKey, StarKeys, PixelStarKey, CharacterStarKey};
pub(crate) fn capture_star_keys(
    keys: &mut StarKeys,
    sky: &ProjectedSky<'_>,
    options: &RenderOptions,
    characters: bool,
    prepared: Option<&PreparedScene>,
    named_candidates: &mut Vec<usize>,
) {
    if characters {
        if !matches!(keys, StarKeys::Characters { .. }) {
            *keys = StarKeys::Characters { glyphs: Vec::new(), labels: Vec::new() };
        }
        let StarKeys::Characters { glyphs, labels } = keys else { unreachable!() };
        capture_character_keys(glyphs, labels, sky, options, prepared)
    } else {
        // Keep exact magnitude and base RGB: deriving final radius/strength here would repeat per-star
        // floating-point rounding on cache misses. The existing rasterizer remains the only owner of that work.
        if !matches!(keys, StarKeys::Pixels(_)) { *keys = StarKeys::Pixels(Vec::new()); }
        let StarKeys::Pixels(stars) = keys else { unreachable!() };
        stars.clear();
        stars.reserve(sky.stars.len());
        named_candidates.clear();
        for (index, entry) in sky.stars.iter().enumerate() {
            if prepared.is_some_and(|p| crate::scene::raster::prepared::is_prepared_star_named(p, &entry.star)) {
                named_candidates.push(index);
            }
            if entry.star.magnitude > options.magnitude_threshold {
                continue;
            }
            if let Some(cell) = entry.cell {
                stars.push(PixelStarKey {
                    cell,
                    magnitude: entry.star.magnitude,
                    color: resolve_star_rgb(&entry.star, prepared),
                });
            }
        }
    }
}

fn capture_character_keys(
    glyphs: &mut Vec<CharacterStarKey>, labels: &mut Vec<(usize, String)>,
    sky: &ProjectedSky<'_>, options: &RenderOptions, prepared: Option<&PreparedScene>,
) {
    let dynamically_named = if options.dynamic_names {
        select_dynamically_named_stars(options, sky)
    } else {
        Vec::new()
    };
    glyphs.clear();
    glyphs.reserve(sky.stars.len());
    labels.clear();
    for (index, entry) in sky.stars.iter().enumerate() {
        if entry.star.magnitude > options.magnitude_threshold {
            continue;
        }
        let Some(cell) = entry.cell else { continue };
        let appearance = select_star_appearance_prepared(&entry.star, sky.names, prepared);
        let label = if dynamically_named.contains(&index) {
            Some(format_star_label(&entry.star, sky.names, options.unicode))
        } else if entry.star.magnitude <= options.label_threshold {
            appearance.label.map(std::borrow::Cow::Borrowed)
        } else {
            None
        };
        if let Some(label) = label {
            // The glyph index preserves glyph/label interleaving and collisions with later stars.
            labels.push((glyphs.len(), label.into_owned()));
        }
        glyphs.push(CharacterStarKey {
            cell,
            glyph: if options.unicode {
                appearance.unicode
            } else {
                appearance.ascii
            },
            color: options.select_color(appearance.color),
        });
    }
}

pub(crate) fn describe_star_keys(keys: &StarKeys, input: usize) -> String {
    let (retained, elements, capacity, labels, text_bytes) = match keys {
        StarKeys::Pixels(stars) => (
            stars.len(),
            stars.len() * std::mem::size_of::<PixelStarKey>(),
            stars.capacity() * std::mem::size_of::<PixelStarKey>(),
            0,
            0,
        ),
        StarKeys::Characters { glyphs, labels } => (
            glyphs.len(),
            glyphs.len() * std::mem::size_of::<CharacterStarKey>()
                + labels.len() * std::mem::size_of::<(usize, String)>(),
            glyphs.capacity() * std::mem::size_of::<CharacterStarKey>()
                + labels.capacity() * std::mem::size_of::<(usize, String)>(),
            labels.len(),
            labels.iter().map(|(_, text)| text.len()).sum(),
        ),
    };
    format!(
        "input stars={input}; retained ordered raster inputs={retained}; copied full star records=0; \
         star-key element bytes={elements}; allocated element capacity bytes={capacity}; \
         resolved labels={labels}; label UTF-8 bytes={text_bytes}; exact structural comparison (no hashes)"
    )
}

/// Release live candidate entries after a hit while keeping only straightforward flat-vector capacity.
/// Clearing figures and labels drops their nested allocations; no committed key is modified here.
pub(crate) fn clear_scene_candidate(key: &mut SceneKey) {
    match &mut key.stars {
        StarKeys::Pixels(stars) => stars.clear(),
        StarKeys::Characters { glyphs, labels } => { glyphs.clear(); labels.clear(); }
    }
    key.planets.clear();
    key.moon = None;
    key.constellations.clear();
    key.horizon.clear();
    key.labels.clear();
}

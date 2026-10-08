//! Exact raster inputs, rather than copies of observed records. Structural comparison also handles public
//! reference skies whose metadata or geometry changes without a production cache generation changing.
use crate::model::{ProjectedSky, RenderOptions};
use crate::scene::{format_star_label, select_dynamically_named_stars};
use crate::scene::raster::appearance::select_star_appearance;
use crate::scene::star_rgb;

use crate::model::{SceneKey, StarKeys, PixelStarKey, CharacterStarKey};
pub(crate) fn capture_star_keys(
    keys: &mut StarKeys,
    sky: &ProjectedSky<'_>,
    options: &RenderOptions,
    characters: bool,
) {
    if characters {
        if !matches!(keys, StarKeys::Characters { .. }) {
            *keys = StarKeys::Characters { glyphs: Vec::new(), labels: Vec::new() };
        }
        let StarKeys::Characters { glyphs, labels } = keys else { unreachable!() };
        capture_character_keys(glyphs, labels, sky, options)
    } else {
        // Keep exact magnitude and base RGB: deriving final opacity here would repeat per-star
        // floating-point rounding on cache misses. The existing rasterizer remains the only owner of that work.
        if !matches!(keys, StarKeys::Pixels(_)) { *keys = StarKeys::Pixels(Vec::new()); }
        let StarKeys::Pixels(stars) = keys else { unreachable!() };
        stars.clear();
        stars.reserve(sky.stars.len());
        stars.extend(prepare_pixel_star_inputs(sky, options));
    }
}

/// Resolve only the star fields used by pixel drawing; callers may collect them or draw headlessly.
pub(in crate::scene) fn prepare_pixel_star_inputs<'a>(sky: &'a ProjectedSky<'_>, options: &'a RenderOptions) -> impl Iterator<Item = PixelStarKey> + 'a {
    sky.stars.iter().filter_map(move |entry| {
        if entry.star.magnitude > options.magnitude_threshold { return None; }
        entry.cell.filter(|&cell| crate::scene::pixel_star_fits(cell, sky.viewport))
            .map(|cell| PixelStarKey { cell, magnitude: entry.star.magnitude, color: star_rgb(&entry.star) })
    })
}

fn capture_character_keys(
    glyphs: &mut Vec<CharacterStarKey>, labels: &mut Vec<(usize, String)>,
    sky: &ProjectedSky<'_>, options: &RenderOptions,
) {
    let dynamically_named = select_dynamically_named_stars(options, sky);
    glyphs.clear();
    glyphs.reserve(sky.stars.len());
    labels.clear();
    for (index, entry) in sky.stars.iter().enumerate() {
        if entry.star.magnitude > options.magnitude_threshold {
            continue;
        }
        let Some(cell) = entry.cell else { continue };
        let appearance = select_star_appearance(&entry.star, sky.names);
        let label = if dynamically_named.contains(&index) {
            Some(format_star_label(&entry.star, sky.names, options.unicode))
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

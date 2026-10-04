//! Exact raster inputs, rather than copies of observed records. Structural comparison also handles public
//! reference skies whose metadata or geometry changes without a production cache generation changing.
use crate::{
    canvas::Color,
    projection::ProjectedSky,
    scene::{
        RenderOptions,
        appearance::select_star_appearance_prepared,
        format_star_label,
        prepared::{PreparedScene, resolve_star_rgb},
        select_dynamically_named_stars,
    },
};

#[derive(Clone, PartialEq)]
pub(super) struct PixelStarKey {
    cell: (i32, i32),
    magnitude: f64,
    color: [u8; 3],
}

#[derive(Clone, PartialEq)]
pub(super) struct CharacterStarKey {
    cell: (i32, i32),
    glyph: char,
    color: Option<Color>,
}

#[derive(Clone, PartialEq)]
pub(super) enum StarKeys {
    Pixels(Vec<PixelStarKey>),
    Characters {
        glyphs: Vec<CharacterStarKey>,
        labels: Vec<(usize, String)>,
    },
}

impl StarKeys {
    pub(super) fn capture(
        sky: &ProjectedSky<'_>,
        options: &RenderOptions,
        characters: bool,
        prepared: Option<&PreparedScene>,
        named_candidates: &mut Vec<usize>,
    ) -> Self {
        if characters {
            Self::capture_characters(sky, options, prepared)
        } else {
            // Keep exact magnitude and base RGB: deriving final radius/strength here would repeat per-star
            // floating-point rounding on cache misses. The existing rasterizer remains the only owner of that work.
            let mut stars = Vec::with_capacity(sky.stars.len());
            named_candidates.clear();
            for (index, entry) in sky.stars.iter().enumerate() {
                if prepared.is_some_and(|p| p.is_named(&entry.star)) {
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
            Self::Pixels(stars)
        }
    }

    fn capture_characters(sky: &ProjectedSky<'_>, options: &RenderOptions, prepared: Option<&PreparedScene>) -> Self {
        let dynamically_named = if options.dynamic_names {
            select_dynamically_named_stars(options, sky)
        } else {
            Vec::new()
        };
        let mut glyphs = Vec::with_capacity(sky.stars.len());
        let mut labels = Vec::new();
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
        Self::Characters { glyphs, labels }
    }

    pub(super) fn describe(&self, input: usize) -> String {
        let (retained, elements, capacity, labels, text_bytes) = match self {
            Self::Pixels(stars) => (
                stars.len(),
                stars.len() * std::mem::size_of::<PixelStarKey>(),
                stars.capacity() * std::mem::size_of::<PixelStarKey>(),
                0,
                0,
            ),
            Self::Characters { glyphs, labels } => (
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
}

//! Human-readable labels for Markdown dumps only. Rust fields, stored rows and cache formats stay unchanged.
//! Context matters: identical tuple types can index different tables or use different coordinate systems.
use crate::rows::Column;

pub(super) fn label_columns(path: &str, columns: &mut [Column]) {
    let names = table_labels(path);
    if !names.is_empty() {
        assert_eq!(names.len(), columns.len(), "debug column schema changed for {path}");
        for (column, &name) in columns.iter_mut().zip(names) { column.name = name; }
    } else {
        for column in columns { column.name = field_label(column.name); }
    }
}

// The named-field crosswalk is below row_columns! in rows/mod.rs. Anonymous-row mappings (label -> storage):
// name_byte_range -> name_table[row][0..2], start inclusive / end exclusive.
// precise-motion direction x/y/z -> [0]/[1]/[2]; scaled velocity x/y/z -> [3]/[4]/[5]; distance -> [6].
// catalog_row_boundary -> grid.offsets[row]; sky_region_index -> region.cells[row].
// catalog_row_index -> endpoint_indices/candidate_indices/candidates/selected[row], or the stellar HashMap key.
// motion (direction_j2000, current_magnitude) -> tuple .0/.1; passes_brightness_filter -> eligible[row].
// working_row_index -> corrections.indices[row]; moon_illumination/moon_phase -> illumination tuple .0/.1.
// projection star key (direction, passes_brightness_filter) -> tuple .0/.1.
// projection order key (observed_star_index, current_magnitude, star_id) -> tuple .0/.1/.2.
// projection stars (observed_star_index, screen_coordinates) -> tuple .0/.1; projected_star_index -> order[row].
// body key (body_kind, direction) -> tuple .0/.1; constellation key (catalog_row_index, direction, current_magnitude) -> .0/.1/.2.
// screen_endpoint_pair -> horizon[row]; screen_coordinates/label_text -> labels[row].0/.1.
// projected_star_index -> named_candidates[row]; has_been_invalidated -> stellar entry's Cache.has_been_invalidated.
// character/glyph_dimensions_and_spacing/coverage_mask_bytes -> glyph HashMap key / Glyph.metrics / Glyph.coverage.len().
// *_pixel_row/text_row/*_chunk label the existing bounded preview; they are not additional stored fields.
fn table_labels(path: &str) -> &'static [&'static str] {
    match path {
        "preparation" => &["max_direction_change_radians"],
        "persistent.catalog.stars" => &["initial_direction", "scaled_velocity_per_year", "initial_magnitude", "brightest_possible_magnitude", "initial_distance_parsecs", "star_id", "name_entry", "encoded_catalog_designation", "spectral_type_code", "color_index_bv", "data_flags", "precise_motion_entry"],
        "persistent.catalog.stars.name_table" => &["name_byte_range"],
        "persistent.catalog.stars.precise_motions" => &["initial_direction_x", "initial_direction_y", "initial_direction_z", "scaled_velocity_x_per_year", "scaled_velocity_y_per_year", "scaled_velocity_z_per_year", "initial_distance_parsecs"],
        "persistent.catalog.grid.offsets" => &["catalog_row_boundary"],
        "persistent.catalog.grid.coarse_caps" | "persistent.catalog.grid.fine_caps" => &["center_direction", "angular_radius_radians"],
        "persistent.catalog.names" => &["name_text_chunk"],
        "persistent.catalog.endpoint_indices" | "cache.sky.figure_override.endpoints" | "cache.sky.candidate_indices"
            | "cache.observation.candidates" | "cache.observation.selected" => &["catalog_row_index"],
        "cache.observation.region" => &["sky_region_index"],
        "cache.observation.working" => &["catalog_row_index", "is_draw_candidate"],
        "cache.observation.prepared_classes" => &["motion_properties"],
        "cache.observation.stellar_scratch" => &["catalog_row_index", "initial_magnitude", "needs_recalculation", "trajectory_parameters", "motion_properties", "calculated_sample", "reuse_window_simulation_seconds", "calculated_at_tt_jd"],
        "cache.observation.stellar" => &["catalog_row_index", "direction_j2000", "current_magnitude", "used_tangential_motion_fallback", "has_been_invalidated"],
        "cache.observation.motion" => &["direction_j2000", "current_magnitude"],
        "cache.observation.eligible" => &["passes_brightness_filter"],
        "cache.observation.corrections" => &["working_row_index"],
        "cache.observation.bodies" => &["position_au", "velocity_au_per_day"],
        "cache.observation.illumination" => &["moon_illumination", "moon_phase"],
        "cache.observation.observer" | "cache.observation.light_time" => &["observer_body", "surface_coordinates", "height_above_surface_m", "frame_time", "observer_position_and_velocity", "reference_to_body_rotation", "reference_to_horizon_rotation", "has_atmosphere", "body_emission_times_tt_jd"],
        "cache.simulation.planets" | "cache.simulation.moon" => &["sample_time_tt_jd", "reuse_half_window_days", "sampled_state"],
        "cache.simulation.orientation" => &["sample_time_tt_jd", "reuse_half_window_days", "orientation_matrix"],
        "cache.projection.star_candidate" | "cache.projection.stars.key" => &["direction", "passes_brightness_filter"],
        "cache.projection.order_candidate" | "cache.projection.order.key" => &["observed_star_index", "current_magnitude", "star_id"],
        "cache.projection.stars" => &["observed_star_index", "screen_coordinates"],
        "cache.projection.order" => &["projected_star_index"],
        "cache.projection.bodies.key" => &["body_kind", "direction"],
        "cache.projection.constellations.key" => &["catalog_row_index", "direction", "current_magnitude"],
        "timings.steps" => &["step_name", "nesting_depth", "average_seconds"],
        "timings.trace.steps" => &["step_name", "nesting_depth", "elapsed_seconds", "details", "own_diagnostic_seconds", "parent_step_index"],
        _ if path.ends_with(".horizon") => &["screen_endpoint_pair"],
        _ if path.ends_with(".labels") => &["screen_coordinates", "label_text"],
        _ if path.ends_with(".named_candidates") => &["projected_star_index"],
        _ if path.ends_with(".prepared.stars") => &["base_rgb_color", "terminal_color", "has_proper_name"],
        _ if path.ends_with(".raster_text.glyphs") => &["character", "glyph_dimensions_and_spacing", "coverage_mask_bytes"],
        _ if path.ends_with(".frame_image") || path.ends_with(".scene_cache.pixels") => &["rgba_pixel_row"],
        _ if path.ends_with(".rgb") => &["rgb_pixel_row"],
        _ if path.ends_with(".upload") => &["text_chunk"],
        _ if path.ends_with(".compressed") || path.ends_with(".serialized") => &["byte_chunk"],
        _ if path.ends_with(".text") || path.ends_with(".composed") || path.ends_with(".serialization_blank")
            || path.ends_with(".frame.sky") || path.ends_with(".frame.panel") || path.ends_with(".presenter.screen")
            || path.ends_with(".presenter.previous") || path.ends_with(".scene_cache.characters") => &["text_row"],
        _ => &[],
    }
}

fn field_label(name: &'static str) -> &'static str {
    match name {
        "source_index" => "catalog_row_index",
        "drawable" => "passes_brightness_filter",
        "magnitude" => "current_magnitude",
        "position" => "direction",
        "kind" => "body_kind",
        "cell" => "screen_coordinates",
        "id" => "star_id",
        "projected_index" => "projected_star_index",
        "maximum_magnitude" => "faintest_endpoint_magnitude",
        "arcs" => "projected_line_sections",
        "abbreviation" => "constellation_abbreviation",
        "segments" => "star_index_pairs",
        "glyph" => "symbol",
        "start" => "start_coordinates",
        "end" => "end_coordinates",
        "points" => "path_coordinates",
        "includes_start" => "includes_original_start",
        "includes_end" => "includes_original_end",
        _ => name,
    }
}

/// Short, untruncated definitions accompany the original owner's existing size/cache notes.
pub(super) fn column_notes(path: &str) -> &'static [&'static str] {
    match path {
        "persistent.catalog.stars" => &[
            "Initial means the J2000 reference date. Directions use fixed J2000 equatorial axes. Scaled velocity is per Julian year (365.25 days), not radians per year; it includes distance change when known.",
            "Magnitude measures apparent brightness: smaller numbers mean brighter. Brightest possible magnitude is a conservative bound within the supported interval, not an all-time physical maximum.",
            "name_entry and precise_motion_entry are one-based references to the corresponding side tables; zero means absent. star_id is stable identity, not a row index.",
            "A zero compact distance means no usable distance here; a precise-motion entry may provide it instead. A precise-motion entry overrides the compact trajectory.",
            "spectral_type_code holds two character bytes (for example [75, 49] means K1). encoded_catalog_designation contains packed label identifiers, not display text.",
            "color_index_bv is the astronomical blue-minus-visual color measurement, not RGB. data_flags: bit 0 (1) = tangential-motion fallback; bit 1 (2) = color index available. Without bit 1, color_index_bv is a placeholder.",
        ],
        "persistent.catalog.stars.name_table" => &["Each pair is [start byte, exclusive end byte] in persistent.catalog.names; the name_entry column refers to this table."],
        "persistent.catalog.stars.precise_motions" => &["Seven components of each original packed row are shown separately for readability; storage is unchanged. Direction uses J2000 axes, velocity is scaled per Julian year, distance is in parsecs; zero distance means unavailable."],
        "persistent.catalog.grid.offsets" => &["Adjacent boundaries delimit a sky region's catalog rows. The last boundary is the start of the always-checked tail, not the end of the full catalog."],
        "persistent.catalog.grid.coarse_caps" | "persistent.catalog.grid.fine_caps" => &["A cap covers a circular patch of sky: its center is a unit direction in J2000 axes and its radius is an angle in radians."],
        "persistent.catalog.names" => &["Rows preview chunks of the shared UTF-8 text buffer, not individual names; name_byte_range locates each name."],
        "preparation" => &["Conservative maximum angular drift from the initial direction over the supported interval, in radians. Freed after preparation."],
        "cache.observation.working" => &["is_draw_candidate records early selection membership, before the current-time brightness check. Other rows are retained for constellation lines."],
        "cache.sky.stars" | "cache.projection.star_candidate" | "cache.projection.stars.key" => &["passes_brightness_filter does not guarantee visibility on screen. Completed observation directions use East/North/Up (x/y/z); during observation the same mutable record passes through earlier coordinate systems."],
        "cache.sky.planets" | "cache.sky.moon" | "cache.projection.bodies.key" | "cache.projection.constellations.key" => &["After completed observation, stars, planets and the Moon use unit East/North/Up directions (x/y/z). Earlier body-subtraction stages hold physical displacements. body_kind includes the Sun."],
        "cache.simulation.planets" => &["TT Julian dates count days on the simulation's terrestrial time scale. Reuse covers sample_time ± reuse_half_window_days. States are in J2000 axes, AU and AU/day, relative to the solar-system center of mass; order: Sun, Mercury, Venus, Earth, Mars, Jupiter, Saturn, Uranus, Neptune."],
        "cache.simulation.moon" => &["Sample time is a TT Julian date; reuse extends the stated number of days in both directions. Moon state is Earth-relative in J2000 axes, with position in AU and velocity in AU/day."],
        "cache.simulation.orientation" => &["Sample time is a TT Julian date; reuse extends the stated number of days in both directions. The matrix stores slow orientation changes; it does not include daily rotation."],
        "cache.observation.observer" | "cache.observation.light_time" => &[
            "Surface latitude/longitude are radians, longitude positive east; height is meters. Observer state uses J2000 axes, AU and AU/day relative to the solar-system center of mass. One AU is approximately the Earth–Sun distance.",
            "Rotation matrices convert reference-frame vectors into body-fixed or local East/North/Up axes. frame_time contains UTC, UT1 and TT Julian dates. has_atmosphere indicates availability, not whether refraction was requested.",
            "Emission times are TT Julian dates ordered: Sun, Mercury, Venus, Earth, Mars, Jupiter, Saturn, Uranus, Neptune, Moon. Before light-time sampling they are initialized to reception time.",
        ],
        "cache.observation.prepared_classes" => &["Motion property bits: bit 0 (1) = stationary; bit 1 (2) = has usable distance. These determine which propagation and brightness calculations apply."],
        "cache.observation.stellar_scratch" => &["catalog_row_index addresses the persistent star table. Times are TT Julian dates; reuse duration is simulated seconds. motion_properties uses bit 0 = stationary, bit 1 = usable distance; empty scratch retains capacity only."],
        "cache.observation.stellar" | "cache.observation.motion" => &["Directions are evaluated at the sample time but expressed in fixed J2000 axes. Magnitudes are current apparent brightness, with smaller numbers brighter. The fallback ignores radial distance change and keeps brightness constant."],
        "cache.observation.eligible" => &["Rows follow the working table; true means the star passed candidate membership and current brightness checks, before screen projection."],
        "cache.observation.corrections" => &["Each index addresses cache.observation.working, not the catalog or the final observed-star table."],
        "cache.observation.bodies" => &["Emission-time body states use J2000 axes, AU and AU/day relative to the solar-system center of mass. The separate Moon value is described in the table's existing notes."],
        "cache.observation.relative" => &["x/y/z are physical observer-relative displacement in AU along J2000 axes, not normalized directions."],
        "cache.observation.apparent" => &["x/y/z are unit star directions in J2000 axes after apparent-direction corrections. Body and Moon vectors are included in the auxiliary note."],
        "cache.observation.horizontal" | "cache.observation.refracted" => &["x/y/z mean East/North/Up. Stars, planets and the Moon have unit directions after aberration. The refracted table additionally includes atmospheric bending."],
        "cache.observation.illumination" => &["Illuminated fraction ranges from 0 to 1; phase_angle is radians; waxing means the illuminated fraction is increasing. moon_phase is the named phase."],
        "timings.steps" => &["average_seconds is a smoothed wall-time average, not a sum. nesting_depth counts parent steps."],
        "timings.trace.steps" => &["Elapsed wall times include children; do not sum parents and children. own_diagnostic_seconds excludes child diagnostics. parent_step_index addresses this trace table; repeated batch passes may be aggregated."],
        _ if path.ends_with(".constellations") || path.ends_with(".figure_override.figures") => &[
            "Source star_index_pairs refer to catalog rows. For projected figures, faintest_endpoint_magnitude is the largest magnitude among the defining stars; larger means fainter.",
            "Nested ProjectedArc fields: start/end = screen coordinates; points = sampled path coordinates; includes_start/includes_end mean the section reaches the original star endpoints rather than a clipping boundary.",
        ],
        _ if path.ends_with(".named_candidates") => &["Indices refer to the projected stars in drawing order, not catalog rows; candidates have proper names but still undergo label rules."],
        _ if path.ends_with(".raster_text.glyphs") => &["Coverage mask bytes describe how much each glyph pixel is filled. Metrics contain glyph size, placement offsets and advance spacing."],
        _ if path.ends_with(".prepared.stars") => &["Rows follow the catalog. base_rgb_color is red/green/blue in 0..255; terminal_color is a character-renderer color; has_proper_name excludes designation-only labels."],
        _ if path.starts_with("cache.projection.") || path.contains("scene_cache") => &["Screen coordinates are (row, column), local to the sky viewport; units are terminal cells or pixels according to the renderer. RGB channels range from 0 to 255; terminal colors use named palette entries. Magnitudes use smaller numbers for brighter stars."],
        _ => &[],
    }
}

pub(super) fn label_color_columns(columns: &mut [Column]) {
    for column in columns {
        if column.name == "color" {
            column.name = if column.dtype == "[u8; 3]" { "base_rgb_color" } else { "terminal_color" };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::Row;
    use crate::model::{StarRow, ObservedStar, SelectedStar, ObserverState, StellarWork, DrawRecord,
        PixelStarKey, CharacterStarKey, StarDisplay, ProjectedArc};

    fn names<T: Row>(path: &str) -> Vec<&'static str> {
        let mut columns = T::columns();
        let original_types: Vec<_> = columns.iter().map(|c| c.dtype).collect();
        label_columns(path, &mut columns);
        label_color_columns(&mut columns);
        assert_eq!(columns.iter().map(|c| c.dtype).collect::<Vec<_>>(), original_types);
        assert!(columns.iter().all(|c| !c.name.is_empty()));
        columns.into_iter().map(|c| c.name).collect()
    }

    #[test]
    fn contextual_labels_keep_types_and_distinguish_index_domains_and_units() {
        assert_eq!(names::<StarRow>("persistent.catalog.stars"), ["initial_direction", "scaled_velocity_per_year", "initial_magnitude", "brightest_possible_magnitude", "initial_distance_parsecs", "star_id", "name_entry", "encoded_catalog_designation", "spectral_type_code", "color_index_bv", "data_flags", "precise_motion_entry"]);
        assert_eq!(names::<SelectedStar>("cache.observation.working"), ["catalog_row_index", "is_draw_candidate"]);
        assert_eq!(names::<ObservedStar>("cache.sky.stars"), ["catalog_row_index", "passes_brightness_filter", "current_magnitude", "direction"]);
        assert_eq!(names::<(usize, (i32, i32))>("cache.projection.stars"), ["observed_star_index", "screen_coordinates"]);
        assert_eq!(names::<usize>("cache.observation.corrections"), ["working_row_index"]);
        assert_eq!(names::<usize>("cache.projection.order"), ["projected_star_index"]);
        assert_eq!(names::<DrawRecord>("cache.projection.draw_order_scratch"), ["current_magnitude", "star_id", "projected_star_index"]);
        assert_eq!(names::<PixelStarKey>("cache.rendering.pixels.scene_cache.pixels.key.stars"), ["screen_coordinates", "current_magnitude", "base_rgb_color"]);
        assert_eq!(names::<CharacterStarKey>("cache.rendering.characters.scene_cache.characters.key.stars"), ["screen_coordinates", "symbol", "terminal_color"]);
        assert_eq!(names::<StarDisplay>("cache.rendering.pixels.scene_cache.prepared.stars"), ["base_rgb_color", "terminal_color", "has_proper_name"]);
        assert_eq!(names::<ProjectedArc>("arc"), ["start_coordinates", "end_coordinates", "path_coordinates", "includes_original_start", "includes_original_end"]);
        assert_eq!(names::<ObserverState>("cache.observation.observer")[8], "body_emission_times_tt_jd");
        assert_eq!(names::<StellarWork>("cache.observation.stellar_scratch")[7], "calculated_at_tt_jd");
    }
}

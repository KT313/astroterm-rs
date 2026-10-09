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
// label_byte_boundary -> names.boundaries[row]; unicode_and_ascii_entries -> names.ascii_alternatives[row].
// precise-motion direction x/y/z -> [0]/[1]/[2]; scaled velocity x/y/z -> [3]/[4]/[5]; distance -> [6].
// catalog_row_boundary -> grid.offsets[row]; sky_region_index -> region.cells[row] (including the final constellation region).
// stellar_scratch (initial_magnitude, trajectory_parameters, motion_properties) -> magnitude/motion/class.
// regional table (simulation_region_id, calculated_at_tt_jd, reuse_window_simulation_seconds) -> entry index/calculated_at/valid_seconds.
// regional has_been_invalidated/generation are Cache fields; sample_count/samples inspect its original stored Vec.
// catalog_row_index -> endpoint_indices/candidate_indices/candidates/selected[row], or an endpoint row.
// region_output_work: direction_j2000/current_magnitude/uses_motion_fallback -> direction/magnitude/used_singular_fallback.
// completed_request -> last_request; selected_stars_using_motion_fallback -> selected_fallback_count.
// regional passes_brightness_filter -> region.eligible[row].
// horizontal_sources.regions -> region ID / output start / output end / membership version / apparent version.
// horizontal_sources.body_generation/revision -> body_direction_version/horizontal_request_version.
// moon_illumination/moon_phase -> illumination tuple .0/.1.
// projection star key (direction, passes_brightness_filter) -> tuple .0/.1.
// projection order key (observed_star_index, current_magnitude, star_id) -> tuple .0/.1/.2.
// projection stars (observed_star_index, screen_coordinates) -> tuple .0/.1; regional_cells uses (region_slot + observed_index, cell).
// region_slot selects regional_output, observed_index selects final cached directions; projected_star_index -> legacy order[row].
// requested_sources: sky_region_index -> tuple.0; validated_range_version -> tuple.1.
// layout_sources: region ID / correction-result version / stellar-sample version; directions remain in their original caches.
// regional_output: sky_region_index -> region; observed row start/end -> start/end; correction/stellar/aberration versions -> selection_generation/motion_generation/apparent_generation.
// assembled_for -> region/start/end/cell-generation/order-generation tuple; regional_ranges marks each region's drawable start/end.
// regional_cell_work (catalog_star_index, screen_coordinates) -> tuple .0/.1; regional_order_work (region_star_index, current_magnitude, star_id) -> tuple .0/.1/.2.
// region_cell_scratch[row] is the optional cell for one row of the current region; cached regional_orders[row].0 is membership-versioned region-local row offset.
// body key (body_kind, direction) -> tuple .0/.1; constellation key (catalog_row_index, direction, current_magnitude) -> .0/.1/.2.
// screen_endpoint_pair -> horizon[row]; screen_coordinates/label_text -> labels[row].0/.1.
// has_been_invalidated -> region entry's Cache.has_been_invalidated.
// character/glyph_dimensions_and_spacing/coverage_mask_bytes -> glyph HashMap key / Glyph.metrics / Glyph.coverage.len().
// *_pixel_row/text_row/*_chunk label the existing bounded preview; they are not additional stored fields.
fn table_labels(path: &str) -> &'static [&'static str] {
    match path {
        "preparation" => &["max_direction_change_radians"],
        "persistent.catalog.stars" => &["initial_direction", "scaled_velocity_per_year", "initial_magnitude", "brightest_possible_magnitude", "initial_distance_parsecs", "star_id", "name_entry", "display_color_index"],
        "persistent.catalog.names.boundaries" => &["label_byte_boundary"],
        "persistent.catalog.names.ascii_alternatives" => &["unicode_and_ascii_entries"],
        "persistent.catalog.stars.precise_motions" => &["initial_direction_x", "initial_direction_y", "initial_direction_z", "scaled_velocity_x_per_year", "scaled_velocity_y_per_year", "scaled_velocity_z_per_year", "initial_distance_parsecs"],
        "persistent.catalog.grid.offsets" => &["catalog_row_boundary"],
        "persistent.catalog.grid.coarse_caps" | "persistent.catalog.grid.fine_caps" => &["center_direction", "angular_radius_radians"],
        "persistent.catalog.names" => &["name_text_chunk"],
        "persistent.catalog.endpoint_indices" | "cache.sky.figure_override.endpoints" | "cache.sky.candidate_indices" => &["catalog_row_index"],
        "cache.selection.requested_sources" => &["sky_region_index", "validated_range_version"],
        "cache.selection.region" => &["sky_region_index"],
        "cache.selection.working" => &["catalog_row_index", "is_draw_candidate"],
        "cache.simulation.stars.prepared_classes" => &["motion_properties"],
        "cache.simulation.stars.region_output_work" => &["direction_j2000", "current_magnitude", "uses_motion_fallback"],
        "cache.simulation.stars.last_request" => &["completed_request"],
        "cache.simulation.stars.selected_fallback_count" => &["selected_stars_using_motion_fallback"],
        "cache.simulation.stars.stellar_scratch" => &["initial_magnitude", "trajectory_parameters", "motion_properties"],
        "cache.simulation.stars.refresh_regions" => &["simulation_region_id"],
        "cache.observation.horizontal_sources.regions" => &["sky_region_index", "observed_row_start", "observed_row_end_exclusive", "correction_membership_version", "aberration_version"],
        "cache.observation.horizontal_sources.body_generation" => &["body_direction_version"],
        "cache.observation.horizontal_sources.revision" => &["horizontal_request_version"],
        "cache.observation.layout_sources" => &["sky_region_index", "correction_membership_version", "stellar_sample_version"],
        "cache.observation.regional_output" => &["sky_region_index", "observed_row_start", "observed_row_end_exclusive", "correction_membership_version", "stellar_sample_version", "aberration_version"],
        "cache.projection.regional_cell_work" => &["catalog_star_index", "screen_coordinates"],
        "cache.projection.regional_order_work" => &["region_star_index", "current_magnitude", "star_id"],
        "cache.projection.regional_cells" => &["region_and_observed_row", "screen_coordinates"],
        "cache.projection.regional_ranges" => &["draw_row_start", "draw_row_end_exclusive"],
        "cache.projection.region_cell_scratch" => &["optional_screen_coordinates"],
        "cache.projection.assembled_for" => &["sky_region_index", "observed_row_start", "observed_row_end_exclusive", "projected_cell_version", "regional_order_version"],
        "cache.observer.bodies" => &["position_au", "velocity_au_per_day"],
        "cache.observation.illumination" => &["moon_illumination", "moon_phase"],
        "cache.observer.observer" | "cache.observer.light_time" => &["observer_body", "surface_coordinates", "height_above_surface_m", "frame_time", "observer_position_and_velocity", "reference_to_body_rotation", "reference_to_horizon_rotation", "has_atmosphere", "body_emission_times_tt_jd"],
        "cache.simulation.solar_system.planets" | "cache.simulation.solar_system.moon" => &["sample_time_tt_jd", "reuse_half_window_days", "sampled_state"],
        "cache.simulation.solar_system.orientation" => &["sample_time_tt_jd", "reuse_half_window_days", "orientation_matrix"],
        "cache.projection.star_candidate" | "cache.projection.stars.key" => &["direction", "passes_brightness_filter"],
        "cache.projection.order_candidate" | "cache.projection.order.key" => &["observed_star_index", "current_magnitude", "star_id"],
        "cache.projection.stars" => &["observed_star_index", "screen_coordinates"],
        "cache.projection.order" => &["projected_star_index"],
        "cache.projection.bodies.key" => &["body_kind", "direction"],
        "cache.projection.constellations.key" => &["catalog_row_index", "direction", "current_magnitude"],
        "timings.steps" => &["step_name", "nesting_depth", "average_seconds"],
        "timings.trace.steps" => &["step_name", "nesting_depth", "elapsed_seconds", "details", "own_diagnostic_seconds", "parent_step_index"],
        _ if path.ends_with(".production.regions") => &["Trusted raster dependencies, one record per requested region; not a copied star table. observed includes row spans plus selection, motion/magnitude and apparent generations. cells/order are completed projection generations. Empty candidates retain capacity after a hit."],
        _ if path.ends_with(".pixel_inputs") => &["Accepted star drawing inputs prepared only on a production raster miss; empty after success with retained capacity. Failed draws can retain unpublished inputs until the next refresh. Editable/headless callers keep their exact key instead."],
        _ if path.ends_with(".star_layer") => &["premultiplied_rgb", "opacity"],
        _ if path.ends_with(".star_opacities") => &["opacity"],
        _ if path.ends_with(".star_opacities.zoom_boost") => &["star_opacity_multiplier"],
        _ if path.ends_with(".horizon") => &["screen_endpoint_pair"],
        _ if path.ends_with(".labels") => &["screen_coordinates", "label_text"],
        _ if path.ends_with(".raster_text.glyphs") => &["character", "glyph_dimensions_and_spacing", "coverage_mask_bytes"],
        _ if path.ends_with(".frame_image") || path.ends_with(".scene_cache.pixels") => &["rgba_pixel_row"],
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
            "Magnitude measures apparent brightness: smaller numbers mean brighter. Both prepared magnitude columns store u16 codes (code / 1000 - 10); previews show the code and decoded value. Brightest possible magnitude bounds the supported interval; code 0 bypasses early pruning because a lower bound may have been clipped.",
            "name_entry is one-based and addresses the shared label boundaries; zero means absent. star_id is a stable u32 identity, not the sorted row index.",
            "Zero distance means unavailable. Stars requiring precision exceptions or load-time motion fallback are currently rejected with an unsupported-feature error.",
            "display_color_index is a prepared palette category, not a measured B-V value: 0 default, 1 hot blue, 2 blue-white, 3 white, 4 yellow-white, 5 yellow, 6 orange, 7 red-orange. The palette supplies both pixel and terminal colors.",
        ],
        "persistent.catalog.names.boundaries" => &["For nonzero name_entry e, label bytes are boundaries[e-1]..boundaries[e]. The last boundary ends the buffer; offsets count UTF-8 bytes."],
        "persistent.catalog.names.ascii_alternatives" => &["Sparse [Unicode entry, ASCII entry] pairs, sorted by Unicode entry. Both reference the same shared label buffer through its boundaries. Labels without a pair use the same text in either mode."],
        "persistent.catalog.star_exceptions" => &["Reserved sparse table: catalog_row_index refers to the final sorted star table; precise_motion_entry is one-based, zero means none. All nonempty exception tables are rejected until sparse support is implemented."],
        "persistent.catalog.stars.precise_motions" => &["Reserved payload; accepted catalogs keep this empty. Seven components of each original packed row are shown separately for readability; storage is unchanged. Direction uses J2000 axes, velocity is scaled per Julian year, distance is in parsecs; zero distance means unavailable."],
        "persistent.catalog.grid.offsets" => &["Adjacent boundaries delimit a sky region's catalog rows. The last boundary ends the catalog. The final range contains constellation endpoints exclusively; preceding ranges are spatial regions."],
        "persistent.catalog.grid.coarse_caps" | "persistent.catalog.grid.fine_caps" => &["A cap covers a circular patch of sky: its center is a unit direction in J2000 axes and its radius is an angle in radians."],
        "persistent.catalog.names" => &["Rows preview chunks of the shared UTF-8 text buffer, not individual names; the boundary table locates each label."],
        "preparation" => &["Conservative maximum angular drift from the initial direction over the supported interval, in radians. Freed after preparation."],
        "cache.selection.requested_sources" => &["Ordered metadata for the current requested regions. Each row identifies a region and its validated-range generation, not an individual star. selection_revision changes when this sequence changes, even if the resulting working star rows stay equal."],
        "cache.selection.working" => &["is_draw_candidate records early selection membership, before the current-time brightness check. Other rows are retained for constellation lines."],
        "cache.sky.stars" | "cache.projection.star_candidate" | "cache.projection.stars.key" => &["passes_brightness_filter does not guarantee visibility on screen. Completed observation directions use East/North/Up (x/y/z); during observation the same mutable record passes through earlier coordinate systems."],
        "cache.sky.planets" | "cache.sky.moon" | "cache.projection.bodies.key" | "cache.projection.constellations.key" => &["After completed observation, stars, planets and the Moon use unit East/North/Up directions (x/y/z). Earlier body-subtraction stages hold physical displacements. body_kind includes the Sun."],
        "cache.simulation.solar_system.group" => &["One always-requested solar-system group. TT calculation time belongs to a complete reception/emission preparation. Request generation tracks lifecycle, not numerical changes. Camera movement is not an input; observer location is."],
        "cache.simulation.solar_system.planet_work" | "cache.simulation.solar_system.moon_work" | "cache.simulation.solar_system.orientation_work" => &["Reusable sample preparation buffer. Successful preparation swaps its allocation with the corresponding saved samples and clears its rows while retaining capacity. A failed preparation may leave unpublished partial rows here."],
        "cache.simulation.solar_system.planets" => &["TT Julian dates count days on the simulation's terrestrial time scale. Reuse covers sample_time ± reuse_half_window_days. States are in J2000 axes, AU and AU/day, relative to the solar-system center of mass; order: Sun, Mercury, Venus, Earth, Mars, Jupiter, Saturn, Uranus, Neptune."],
        "cache.simulation.solar_system.moon" => &["Sample time is a TT Julian date; reuse extends the stated number of days in both directions. Moon state is Earth-relative in J2000 axes, with position in AU and velocity in AU/day."],
        "cache.simulation.solar_system.orientation" => &["Sample time is a TT Julian date; reuse extends the stated number of days in both directions. The matrix stores slow orientation changes; it does not include daily rotation."],
        "cache.observer.observer" | "cache.observer.light_time" => &[
            "Surface latitude/longitude are radians, longitude positive east; height is meters. Observer state uses J2000 axes, AU and AU/day relative to the solar-system center of mass. One AU is approximately the Earth–Sun distance.",
            "Rotation matrices convert reference-frame vectors into body-fixed or local East/North/Up axes. frame_time contains UTC, UT1 and TT Julian dates. has_atmosphere indicates availability, not whether refraction was requested.",
            "Emission times are TT Julian dates ordered: Sun, Mercury, Venus, Earth, Mars, Jupiter, Saturn, Uranus, Neptune, Moon. Before light-time sampling they are initialized to reception time.",
        ],
        "cache.simulation.stars.prepared_classes" => &["Motion property bits: bit 0 (1) = stationary; bit 1 (2) = has usable distance. These determine which propagation and brightness calculations apply."],
        "cache.simulation.stars.stellar_scratch" => &["Numeric inputs for one bounded batch, in catalog order; calculated samples enter region_output_work before it is swapped into the completed regional cache. motion_properties uses bit 0 = stationary, bit 1 = usable distance; empty scratch retains capacity only. No per-star cache metadata."],
        "cache.simulation.stars.region_output_work" => &["Reusable replacement samples for one region. Empty after commit, with the displaced allocation retained for a later refresh; no combined selected-star copy is stored."],
        "cache.simulation.stars.last_request" => &["Provenance of the completed request: selection owner/working generation, requested-region revision, requested TT and regional-results revision. This request time does not replace individual regions' calculation times."],
        "cache.observer.bodies" => &["Emission-time body states use J2000 axes, AU and AU/day relative to the solar-system center of mass. The separate Moon value is described in the table's existing notes."],
        "cache.observation.relative" => &["x/y/z are physical observer-relative displacement in AU along J2000 axes, not normalized directions."],
        "cache.observation.horizontal" | "cache.observation.refracted" => &["x/y/z mean East/North/Up. Stars, planets and the Moon have unit directions after aberration. The refracted table additionally includes atmospheric bending."],
        "cache.observation.illumination" => &["Illuminated fraction ranges from 0 to 1; phase_angle is radians; waxing means the illuminated fraction is increasing. moon_phase is the named phase."],
        "timings.steps" => &["average_seconds is a smoothed wall-time average, not a sum. nesting_depth counts parent steps."],
        "timings.trace.steps" => &["Elapsed wall times include children; do not sum parents and children. own_diagnostic_seconds excludes child diagnostics. parent_step_index addresses this trace table; repeated batch passes may be aggregated."],
        _ if path.ends_with(".constellations") || path.ends_with(".figure_override.figures") => &[
            "Source star_index_pairs refer to catalog rows. For projected figures, faintest_endpoint_magnitude is the largest magnitude among the defining stars; larger means fainter.",
            "Nested ProjectedArc fields: start/end = screen coordinates; points = sampled path coordinates; includes_start/includes_end mean the section reaches the original star endpoints rather than a clipping boundary.",
        ],
        _ if path.ends_with(".production.regions") => &["Trusted raster dependencies, one record per requested region; not a copied star table. observed includes row spans plus selection, motion/magnitude and apparent generations. cells/order are completed projection generations. Empty candidates retain capacity after a hit."],
        _ if path.ends_with(".pixel_inputs") => &["Accepted star drawing inputs prepared only on a production raster miss; empty after success with retained capacity. Failed draws can retain unpublished inputs until the next refresh. Editable/headless callers keep their exact key instead."],
        _ if path.ends_with(".star_layer") => &["Row-major star pixels; RGB is premultiplied by the opacity, both f32 in 0..1. The opacity floor is applied during composition, not stored here. Capacity is reused; on a cache hit the last drawn layer remains. premultiplied_rgb -> StarPixel.rgb; opacity -> StarPixel.opacity."],
        _ if path.ends_with(".star_opacities") => &["One opacity per catalog magnitude code (row index = thousandths of a magnitude above -10.000) for the current star opacity multiplier. Rebuilt only when a field-of-view change alters the multiplier; empty until the first pixel redraw."],
        _ if path.ends_with(".raster_text.glyphs") => &["Coverage mask bytes describe how much each glyph pixel is filled. Metrics contain glyph size, placement offsets and advance spacing."],
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
        PixelStarKey, CharacterStarKey, ProjectedArc};

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
        assert_eq!(names::<StarRow>("persistent.catalog.stars"), ["initial_direction", "scaled_velocity_per_year", "initial_magnitude", "brightest_possible_magnitude", "initial_distance_parsecs", "star_id", "name_entry", "display_color_index"]);
        assert_eq!(names::<SelectedStar>("cache.selection.working"), ["catalog_row_index", "is_draw_candidate"]);
        assert_eq!(names::<ObservedStar>("cache.sky.stars"), ["catalog_row_index", "passes_brightness_filter", "current_magnitude", "direction"]);
        assert_eq!(names::<(usize, (i32, i32))>("cache.projection.stars"), ["observed_star_index", "screen_coordinates"]);
        assert_eq!(names::<usize>("cache.projection.order"), ["projected_star_index"]);
        assert_eq!(names::<DrawRecord>("cache.projection.draw_order_scratch"), ["current_magnitude", "star_id", "projected_star_index"]);
        assert_eq!(names::<PixelStarKey>("cache.rendering.pixels.scene_cache.pixels.key.stars"), ["screen_coordinates", "current_magnitude", "base_rgb_color"]);
        assert_eq!(names::<CharacterStarKey>("cache.rendering.characters.scene_cache.characters.key.stars"), ["screen_coordinates", "symbol", "terminal_color"]);
        assert_eq!(names::<ProjectedArc>("arc"), ["start_coordinates", "end_coordinates", "path_coordinates", "includes_original_start", "includes_original_end"]);
        assert_eq!(names::<ObserverState>("cache.observer.observer")[8], "body_emission_times_tt_jd");
        assert_eq!(names::<StellarWork>("cache.simulation.stars.stellar_scratch"), ["initial_magnitude", "trajectory_parameters", "motion_properties"]);
    }
}

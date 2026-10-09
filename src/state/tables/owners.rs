//! One listing per owner struct: which fields are tables and which cache group governs them.
//! Read top-down: root → persistent / cache / timings → each stage → its fields.
use super::{Bytes, Opaque, Single, TimingSteps, TableVisitor, Tables, join};
use crate::cache::Group;
use crate::model::{ObservedSky, SceneKey, SkyCatalog};
use crate::state::{
    ApplicationState, Caches, CharacterState, ObservationCache, Persistent, PixelState, ProjectionCache,
    RenderingState, SceneCache, SimulationState,
};
use crate::timing::StepTimes;

// --- root --------------------------------------------------------------------------------------------------------

list_tables!(ApplicationState { leaves: [current_view, preparation], scalars: [], groups: [persistent, cache, timings] });
list_tables!(Persistent { leaves: [], scalars: [], groups: [catalog] });
list_tables!(Caches { leaves: [], scalars: [], groups: [sky, simulation, observer, selection, observation, projection, rendering] });

// --- persistent ------------------------------------------------------------------------------------------------

/// The original star table retains its complete schema and allocation capacities.
impl Tables for SkyCatalog {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        let path = |name: &str| join(prefix, name);
        visit(&path("stars"), &self.stars, None);
        visit(&path("magnitude_clipping"), &Single(&self.stars.magnitude_clipping()), None);
        visit(&path("star_exceptions"), &self.star_exceptions, None);
        visit(&path("names.boundaries"), self.names.boundaries(), None);
        visit(&path("names.ascii_alternatives"), self.names.ascii_alternatives(), None);
        visit(&path("stars.precise_motions"), &super::PreciseMotions(self.stars.precise_motions()), None);
        visit(&path("grid.offsets"), &self.grid.offsets, None);
        visit(&path("grid.coarse_caps"), &self.grid.coarse_caps, None);
        visit(&path("grid.fine_caps"), &self.grid.fine_caps, None);
        visit(&path("endpoint_indices"), &self.figures.endpoints, None);
        visit(&path("names"), &self.names, None);
        visit(&path("constellations"), &self.figures.figures, None);
    }
}

// --- cache: observed sky, simulation, observation ---------------------------------------------------------------

impl Tables for ObservedSky {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        visit(&join(prefix, "stars"), &self.stars, None);
        visit(&join(prefix, "planets"), &self.planets, None);
        visit(&join(prefix, "moon"), &Single(&self.moon), None);
        visit(&join(prefix, "candidate_indices"), &self.candidate_indices, None);
        if let Some(figures) = self.figure_override() {
            visit(&join(prefix, "figure_override.figures"), &figures.figures, None);
            visit(&join(prefix, "figure_override.endpoints"), &figures.endpoints, None);
        }
    }
}

impl Tables for SimulationState {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        visit(&join(prefix, "planets"), &self.planets, None);
        visit(&join(prefix, "moon"), &self.moon, None);
        visit(&join(prefix, "orientation"), &self.orientation, None);
        visit(&join(prefix, "planet_work"), &self.planet_work, None);
        visit(&join(prefix, "moon_work"), &self.moon_work, None);
        visit(&join(prefix, "orientation_work"), &self.orientation_work, None);
        visit(&join(prefix, "group"), &Single(&self.group), None);
    }
}

impl Tables for ObservationCache {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        let path = |name: &str| join(prefix, name);
        visit(&path("regions"), &super::regions::ObservationRegionsTable(&self.regions), None);
        visit(&path("regional_output"), &self.regional_output, Some(Group::Projection));
        self.horizontal_sources.visit_tables(&path("horizontal_sources"), visit);
        visit(&path("layout_sources"), &self.layout_sources, None);
        visit(&path("published"), &Single(&self.published), None);
        visit(&path("use_refraction"), &Single(&self.use_refraction), None);
        visit(&path("horizontal_work"), &self.horizontal_work, None);
        visit(&path("refraction_work"), &self.refraction_work, None);
        visit(&path("relative"), &self.relative, Some(Group::SolarSystemGeometry));
        visit(&path("body_apparent"), &self.body_apparent, Some(Group::ApparentDirections));
        visit(&path("horizontal"), &self.horizontal, Some(Group::HorizontalSky));
        visit(&path("refracted"), &self.refracted, Some(Group::Refraction));
        visit(&path("illumination"), &super::ScalarCache(&self.illumination), Some(Group::SolarSystemGeometry));
    }
}

// --- cache: projection ---------------------------------------------------------------------------------------------

/// The cached values plus the keys that are large enough to matter (candidate lists and figure copies).
impl Tables for ProjectionCache {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        let path = |name: &str| join(prefix, name);
        visit(&path("regional_stars"), &super::regions::RegionalTable { entries: &self.regional_stars, nested_bytes: super::TableBytes::vector }, Some(Group::Projection));
        visit(&path("regional_orders"), &super::regions::RegionalTable { entries: &self.regional_orders, nested_bytes: super::TableBytes::vector }, Some(Group::DrawOrder));
        visit(&path("regional_cell_work"), &self.regional_cell_work, None);
        visit(&path("regional_order_work"), &self.regional_order_work, None);
        visit(&path("regional_cells"), &self.regional_cells, None);
        visit(&path("regional_ranges"), &self.regional_ranges, None);
        visit(&path("region_cell_scratch"), &self.region_cell_scratch, None);
        visit(&path("assembled_for"), &self.assembled_for, None);
        visit(&path("source_revision"), &Single(&self.source_revision), None);
        visit(&path("render_context"), &Single(&self.render_context), None);
        visit(&path("star_candidate"), &self.star_candidate, None);
        visit(&path("order_candidate"), &self.order_candidate, None);
        visit(&path("draw_order_scratch"), &self.draw_order_scratch, None);
        visit(&path("stars"), &self.stars, Some(Group::Projection));
        visit(&path("stars.key"), &self.stars.key(), None);
        visit(&path("order"), &self.order, Some(Group::DrawOrder));
        visit(&path("order.key"), &self.order.key(), None);
        visit(&path("bodies"), &self.bodies, Some(Group::Projection));
        visit(&path("bodies.key"), &self.bodies.key(), None);
        visit(&path("constellations"), &self.constellations, Some(Group::Projection));
        visit(&path("constellations.key"), &self.constellations.key(), None);
        visit(&path("horizon"), &self.horizon, Some(Group::ViewGeometry));
    }
}

// --- cache: rendering ------------------------------------------------------------------------------------------

/// Nothing is listed before the terminal opens; afterwards the character or pixel backend lists its buffers.
impl Tables for RenderingState {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        match self {
            Self::Pending => {}
            Self::Chars(state) => state.visit_tables(&join(prefix, "characters"), visit),
            Self::Pixels(state) => state.visit_tables(&join(prefix, "pixels"), visit),
        }
    }
}
impl Tables for CharacterState {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        let path = |name: &str| join(prefix, name);
        visit(&path("frame.sky"), &self.frame.sky, None);
        visit(&path("frame.panel"), &self.frame.panel, None);
        visit(&path("presenter.screen"), &self.presenter.screen, None);
        visit(&path("presenter.previous"), &self.presenter.previous, None);
        visit(&path("fields"), &self.fields, None);
        visit(&path("step_fields"), &self.step_fields, None);
        self.scene_cache.visit_tables(&path("scene_cache"), visit);
    }
}
impl Tables for PixelState {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        let path = |name: &str| join(prefix, name);
        visit(&path("frame_image"), &self.frame_image, None);
        visit(&path("rgb"), &self.rgb, None);
        visit(&path("rgb_version"), &Single(&self.rgb_version), None);
        visit(&path("frame_key"), &Single(&self.frame_key), None);
        visit(&path("encoding_key"), &Single(&self.encoding_key), None);
        visit(&path("displayed_key"), &Single(&self.displayed_key), None);
        visit(&path("display_valid"), &Single(&self.display_valid), None);
        visit(&path("text"), &self.text, None);
        visit(&path("text_version"), &Single(&self.text_version), Some(Group::RasterAssets));
        visit(&path("text_cache.labels"), &self.text_cache.labels, Some(Group::RasterAssets));
        if let Some(key) = &self.text_cache.labels_key { visit(&path("text_cache.label_regions"), &key.projection.regions, Some(Group::RasterAssets)); }
        if let Some(key) = &self.text_cache.text_key {
            visit(&path("text_cache.fields"), &key.fields, Some(Group::RasterAssets));
            visit(&path("text_cache.planets"), &key.planets, Some(Group::RasterAssets));
            visit(&path("text_cache.horizon"), &key.horizon, Some(Group::RasterAssets));
        }
        visit(&path("composed"), &self.composed, None);
        visit(&path("serialization_blank"), &self.serialization_blank, None);
        visit(&path("serialized"), &Bytes::binary(&self.serialized), None);
        visit(&path("upload"), &Bytes::string(&self.upload), None);
        visit(&path("compressed"), &Bytes::binary(&self.compressed), None);
        visit(&path("fields"), &self.fields, None);
        visit(&path("raster_text.glyphs"), &self.raster_text.as_ref().map(|r| &r.glyphs), Some(Group::RasterAssets));
        visit(&path("encoded"), &Opaque { present: self.encoded.is_some(), what: "encoded terminal-protocol payload" }, None);
        self.scene_cache.visit_tables(&path("scene_cache"), visit);
    }
}
impl Tables for SceneCache {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        let path = |name: &str| join(prefix, name);
        visit(&path("star_layer"), &self.star_layer, None);
        visit(&path("pixel_inputs"), &self.pixel_inputs, None);
        if let Some(key) = &self.pixel_candidate { key.visit_tables(&path("pixel_candidate"), visit); }
        if let Some(key) = &self.character_candidate { key.visit_tables(&path("character_candidate"), visit); }
        visit(&path("pixels"), &self.pixels, Some(Group::Raster));
        if let Some(key) = self.pixels.key() { key.visit_tables(&path("pixels.key"), visit); }
        visit(&path("characters"), &self.characters, Some(Group::Raster));
        if let Some(key) = self.characters.key() { key.visit_tables(&path("characters.key"), visit); }
    }
}
/// The vectors inside a raster key; its scalar members (viewport, options, flags) are not tables.
impl Tables for SceneKey {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        if let Some(key) = &self.production { visit(&join(prefix, "production.regions"), &key.regions, None); }
        visit(&join(prefix, "stars"), &self.stars, None);
        visit(&join(prefix, "planets"), &self.planets, None);
        visit(&join(prefix, "constellations"), &self.constellations, None);
        visit(&join(prefix, "horizon"), &self.horizon, None);
        visit(&join(prefix, "labels"), &self.labels, None);
    }
}

// --- timings -----------------------------------------------------------------------------------------------------

/// Smoothed step averages and, when tracing is on, the recorded trace steps.
impl Tables for StepTimes {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        visit(&join(prefix, "steps"), &TimingSteps(self), None);
        if let Some(trace) = self.trace() { visit(&join(prefix, "trace.steps"), &trace.steps, None); }
    }
}

impl super::Table for crate::model::CatalogPreparation {
    fn shape(&self) -> Vec<usize> { vec![self.motion_bounds.len()] }
    fn rows(&self) -> usize { self.motion_bounds.len() }
    fn bytes(&self) -> super::TableBytes { super::TableBytes::vector(&self.motion_bounds) }
    fn columns(&self) -> Vec<crate::rows::Column> { vec![crate::rows::Column { name: "motion_bound", dtype: "f32" }] }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { super::preview_indices(self.motion_bounds.len()).map(|i| (i, vec![self.motion_bounds[i].to_string()])).collect() }
}

list_tables!(crate::state::ObserverPreparationCache { leaves: [bodies @ SolarSystemObservation], scalars: [observer @ ObserverState, light_time @ SolarSystemObservation], groups: [] });

impl Tables for crate::state::StarSelectionCache {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        let path = |name: &str| join(prefix, name);
        visit(&path("region_candidates"), &super::regions::RegionalTable { entries: &self.region_candidates, nested_bytes: super::regions::no_nested_bytes }, Some(Group::CandidateSelection));
        visit(&path("region_selected"), &super::regions::RegionalTable { entries: &self.region_selected, nested_bytes: super::regions::no_nested_bytes }, Some(Group::WorkingSet));
        visit(&path("region"), &self.region, Some(Group::CandidateSelection));
        visit(&path("requested_sources"), &self.requested_sources, None);
        visit(&path("statistics"), &Single(&self.statistics), None);
        visit(&path("selection_revision"), &Single(&self.selection_revision), None);
        visit(&path("working"), &self.working, Some(Group::WorkingSet));
    }
}

list_tables!(crate::state::SimulationCaches { leaves: [], scalars: [], groups: [solar_system, stars] });
impl Tables for crate::state::StellarSimulationState {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        let path = |name: &str| join(prefix, name);
        visit(&path("prepared_classes"), &self.prepared_classes, None);
        visit(&path("stellar_scratch"), &self.stellar_scratch, None);
        visit(&path("refresh_regions"), &self.refresh_regions, None);
        visit(&path("regions"), &self.regions, Some(Group::StellarState));
        visit(&path("region_output_work"), &self.region_output_work, None);
        visit(&path("last_request"), &Single(&self.last_request), None);
        visit(&path("selected_fallback_count"), &Single(&self.selected_fallback_count), None);
    }
}

impl Tables for crate::state::HorizontalSources {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        visit(&join(prefix, "regions"), &self.regions, None);
        visit(&join(prefix, "body_generation"), &Single(&self.body_generation), None);
        visit(&join(prefix, "revision"), &Single(&self.revision), None);
    }
}

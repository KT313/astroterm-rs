//! Prepared-catalog cache schema, fingerprints and semantic checks. I/O lives here above the pure models;
//! prepared arrays are decoded into owned memory before publication.
use crate::model::{SkyCatalog, CELL_COUNT, SkyGrid, Constellation, StarStorage, STAR_SECTIONS};
use super::super::grid::stored_cell;
use crate::catalog::{
    StarNames,
    cache::{PreparedCatalogBytes, invalid, write_sections},
    load_constellation_figures,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

// star-storage sections first, then grid offsets, always-checked indices, endpoints, names and figures
const GRID_OFFSETS: usize = STAR_SECTIONS;
const ALWAYS_CHECKED: usize = STAR_SECTIONS + 1;
const ENDPOINTS: usize = STAR_SECTIONS + 2;
const NAMES: usize = STAR_SECTIONS + 3;
const FIGURES: usize = STAR_SECTIONS + 4;
const SECTION_COUNT: usize = STAR_SECTIONS + 5;

/// Covers the layout, all numerical contracts, parsing/selection policies and embedded supplemental data.
/// Source hashes deliberately invalidate caches even for conservative implementation-only changes.
pub fn catalog_fingerprint() -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"astroterm catalog v2; vector columns; little-endian u64 indices; validation=1; HR=1; override=1; motion=1; quantization=1; cube-Morton=1");
    for value in [
        crate::astro::COMPUTATIONAL_INTERVAL.start_tt,
        crate::astro::COMPUTATIONAL_INTERVAL.end_tt,
        crate::astro::JULIAN_YEAR_DAYS,
        crate::astro::models::stars::SINGULAR_RATIO,
        crate::astro::models::stars::ALWAYS_CHECKED_ANGLE,
        crate::model::QUANTIZATION_MARGIN,
    ] {
        hash.update(value.to_le_bytes());
    }
    hash.update([crate::model::GRID_DEPTH]);
    for source in [
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/catalog/athyg.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/catalog/space_motion.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/catalog/bsc5.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/catalog/tables.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/catalog/cache/array.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/astro/mod.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/astro/coords.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/catalog/mod.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/catalog/names.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/catalog/designation.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/catalog/cache/encoding.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/catalog/cache/mod.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/model/catalog/storage/mod.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/model/catalog/storage/columns.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/model/catalog/storage/views.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/model/catalog/grid.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/model/catalog/records.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/model/celestial/objects.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sky/catalog/grid/mod.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sky/mod.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sky/catalog/pipeline.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sky/catalog/preparation/mod.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sky/catalog/cache/mod.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sky/catalog/cache/pipeline.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sky/catalog/cache/loading.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sky/catalog/cache/format.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sky/catalog/stars/mod.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/astro/models/stars.rs")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/bsc5")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/bsc5_names.txt")).as_slice(),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/bsc5_constellations.txt")).as_slice(),
    ] {
        hash.update(source);
    }
    hash.finalize().into()
}

pub fn cache_path(source: &Path, directory: &Path) -> io::Result<PathBuf> {
    let canonical = source.canonicalize()?;
    let metadata = fs::metadata(&canonical)?;
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    let mut hash = Sha256::new();
    hash.update(canonical.as_os_str().as_encoded_bytes());
    hash.update(metadata.len().to_le_bytes());
    hash.update(modified.as_secs().to_le_bytes());
    hash.update(modified.subsec_nanos().to_le_bytes());
    Ok(directory.join(format!(
        "{}.catalog",
        hash.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
    )))
}

pub fn write_cached_catalog(path: &Path, catalog: &SkyCatalog, fingerprint: &[u8; 32]) -> io::Result<()> {
    validate_catalog(catalog, true)?;
    let figures = encode_figures(&catalog.constellations)?;
    let mut sections = catalog.stars.cache_sections();
    sections.extend([
        catalog.grid.offsets.bytes(),
        catalog.always_checked.bytes(),
        catalog.endpoint_indices.bytes(),
        catalog.names.bytes(),
        &figures,
    ]);
    write_sections(path, fingerprint, &sections)
}

pub fn load_cached_catalog(path: &Path, fingerprint: &[u8; 32]) -> io::Result<SkyCatalog> {
    let data = PreparedCatalogBytes::open(path, fingerprint)?;
    if data.section_count() != SECTION_COUNT {
        return Err(invalid("catalog section count mismatch"));
    }
    let stars = StarStorage::from_prepared(&data)?;
    let names = StarNames::from_array(data.decode(NAMES)?.into())?;
    let constellations = decode_figures(data.section(FIGURES)?)?;
    let mut catalog = SkyCatalog {
        stars,
        names,
        constellations,
        grid: SkyGrid::from_offsets(data.decode(GRID_OFFSETS)?.into()),
        always_checked: data.decode(ALWAYS_CHECKED)?.into(),
        endpoint_indices: data.decode(ENDPOINTS)?.into(),
        singular_count: 0,
    };
    drop(data); // release the file snapshot; the catalog owns every decoded column
    validate_catalog(&catalog, false)?;
    catalog.singular_count = catalog.stars.iter().filter(|s| s.singular_fallback).count();
    Ok(catalog)
}

fn validate_catalog(catalog: &SkyCatalog, full: bool) -> io::Result<()> {
    catalog.stars.validate(&catalog.names, full)?;
    let n = catalog.stars.len();
    let offsets = &catalog.grid.offsets;
    if offsets.len() != CELL_COUNT + 1
        || offsets[0] != 0
        || offsets.windows(2).any(|p| p[0] > p[1])
        || offsets[CELL_COUNT] > n
    {
        return Err(invalid("invalid grid offsets"));
    }
    let tail = offsets[CELL_COUNT];
    if catalog.always_checked.len() != n - tail || catalog.always_checked.iter().copied().ne(tail..n) {
        return Err(invalid("grid/fast-mover partition mismatch"));
    }
    let mut ids = HashSet::with_capacity(n);
    for cell in 0..=CELL_COUNT {
        let (start, end) = if cell == CELL_COUNT {
            (tail, n)
        } else {
            (offsets[cell], offsets[cell + 1])
        };
        for i in start..end {
            if !ids.insert(catalog.stars.id(i)) {
                return Err(invalid("duplicate stable star ID"));
            }
            if i > start
                && (catalog.stars.brightness_key(i - 1) > catalog.stars.brightness_key(i)
                    || (catalog.stars.brightness_key(i - 1) == catalog.stars.brightness_key(i)
                        && catalog.stars.id(i - 1) < catalog.stars.id(i)))
            {
                return Err(invalid("unsorted brightness keys/IDs"));
            }
            if full && stored_cell(&catalog.stars, i) != cell {
                return Err(invalid("star assigned to wrong cell"));
            }
        }
    }
    let mut endpoints = Vec::new();
    for figure in &catalog.constellations {
        for &index in figure.segments.iter().flatten() {
            if index >= n {
                return Err(invalid("constellation index out of range"));
            }
            endpoints.push(index);
        }
    }
    endpoints.sort_unstable();
    endpoints.dedup();
    if endpoints.as_slice() != &*catalog.endpoint_indices {
        return Err(invalid("endpoint union mismatch"));
    }
    Ok(())
}

fn encode_figures(figures: &[Constellation]) -> io::Result<Vec<u8>> {
    let known = load_constellation_figures().map_err(io::Error::other)?;
    let mut bytes = Vec::new();
    let mut seen = HashSet::new();
    bytes.extend_from_slice(&(figures.len() as u64).to_le_bytes());
    for figure in figures {
        let id = known
            .iter()
            .position(|f| f.abbreviation == figure.abbreviation)
            .ok_or_else(|| invalid("unknown constellation abbreviation"))?;
        if !seen.insert(id) {
            return Err(invalid("duplicate constellation"));
        }
        bytes.extend_from_slice(&(id as u64).to_le_bytes());
        bytes.extend_from_slice(&(figure.segments.len() as u64).to_le_bytes());
        for &index in figure.segments.iter().flatten() {
            bytes.extend_from_slice(&(index as u64).to_le_bytes());
        }
    }
    Ok(bytes)
}
fn decode_figures(bytes: &[u8]) -> io::Result<Vec<Constellation>> {
    let known = load_constellation_figures().map_err(io::Error::other)?;
    let values: &[u64] = bytemuck::try_cast_slice(bytes).map_err(|_| invalid("unaligned constellation section"))?;
    let (&count, mut rest) = values.split_first().ok_or_else(|| invalid("missing figures"))?;
    if count > known.len() as u64 {
        return Err(invalid("too many figures"));
    }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for _ in 0..count {
        if rest.len() < 2 {
            return Err(invalid("truncated figure"));
        }
        let (id, count) = (rest[0] as usize, rest[1] as usize);
        rest = &rest[2..];
        let figure = known
            .get(id)
            .filter(|_| seen.insert(id))
            .ok_or_else(|| invalid("invalid figure ID"))?;
        if count == 0 || count > rest.len() / 2 {
            return Err(invalid("invalid segment count"));
        }
        let segments = rest[..count * 2]
            .chunks_exact(2)
            .map(|pair| [pair[0] as usize, pair[1] as usize])
            .collect();
        result.push(Constellation {
            abbreviation: figure.abbreviation,
            segments,
        });
        rest = &rest[count * 2..];
    }
    if !rest.is_empty() {
        return Err(invalid("trailing figure data"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::load_sky_catalog;
    use crate::catalog::{load_embedded_catalog, datasets::{Dataset, DatasetDirectories}};
    use crate::astro::{J2000, Observer};
    use crate::canvas::Canvas;
    use crate::model::{Sky, ProjectionViewport as Viewport, View, RenderOptions};
    use crate::projection::project_sky;
    use crate::scene::draw_sky_scene;
    use crate::sky::update_sky_positions;
    use crate::timing::StepTimes;
    use std::sync::Arc;

    fn prepared() -> SkyCatalog {
        crate::sky::prepare_owned_catalog(load_embedded_catalog().unwrap())
    }
    fn render(catalog: SkyCatalog, threshold: f64, date: f64) -> Canvas {
        let mut sky = Sky::new(Arc::new(catalog));
        update_sky_positions(
            &mut sky,
            date,
            &Observer::default(),
            threshold,
            &mut StepTimes::default(),
        );
        crate::sky::refract_sky_positions(&mut sky);
        let mut canvas = Canvas::new(41, 81);
        let options = RenderOptions {
            unicode: true,
            braille: true,
            color: true,
            constellations: true,
            grid: false,
            magnitude_threshold: threshold,
            label_threshold: 0.25,
            dynamic_names: true,
        };
        draw_sky_scene(
            &mut canvas,
            &options,
            &project_sky(&sky, &View::default(), Viewport { height: 41, width: 81 }).view(&sky),
        );
        canvas
    }
    #[test]
    fn owned_roundtrip_preserves_data_threshold_edges_and_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog");
        let source = prepared();
        let fingerprint = catalog_fingerprint();
        write_cached_catalog(&path, &source, &fingerprint).unwrap();
        let cached = load_cached_catalog(&path, &fingerprint).unwrap();
        assert_eq!(source, cached);
        for threshold in [5_f32.next_down() as f64, 5.0, 5_f32.next_up() as f64] {
            for date in [J2000, J2000 + 1e6, crate::astro::COMPUTATIONAL_INTERVAL.end_tt] {
                assert_eq!(
                    render(source.clone(), threshold, date),
                    render(cached.clone(), threshold, date)
                );
            }
        }
        drop(cached);
        let cached = load_cached_catalog(&path, &fingerprint).unwrap();
        write_cached_catalog(&path, &source, &fingerprint).unwrap(); // atomic replacement cannot change already loaded data
        fs::remove_file(&path).unwrap();
        assert_eq!(cached, source); // no file or reader is needed after loading
    }
    fn section(bytes: &[u8], index: usize) -> usize {
        u64::from_le_bytes(bytes[64 + index * 16..72 + index * 16].try_into().unwrap()) as usize
    }
    fn fix_checksum(bytes: &mut [u8]) {
        let mut crc = crc32fast::Hasher::new();
        crc.update(&bytes[..56]);
        crc.update(&bytes[60..]);
        bytes[56..60].copy_from_slice(&crc.finalize().to_le_bytes());
    }
    #[test]
    fn corrupt_containers_and_semantic_data_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog");
        let source = prepared();
        let fingerprint = catalog_fingerprint();
        write_cached_catalog(&path, &source, &fingerprint).unwrap();
        let original = fs::read(&path).unwrap();
        for case in 0..12 {
            let mut bytes = original.clone();
            match case {
                0 => bytes.truncate(32),
                1 => bytes[64..72].copy_from_slice(&u64::MAX.to_le_bytes()),
                2 => bytes[16] ^= 1,
                3 => {
                    let start = section(&bytes, 0);
                    bytes[start] ^= 1;
                }
                4 => {
                    let start = section(&bytes, NAMES);
                    bytes[start] = 255;
                }
                5 => {
                    let pair = source.grid.offsets.windows(2).find(|p| p[1] - p[0] >= 2).unwrap();
                    let start = section(&bytes, 3) + pair[0] * 4; // brightness keys
                    bytes[start..start + 4].copy_from_slice(&f32::MAX.to_le_bytes());
                }
                6 => {
                    let start = section(&bytes, 0);
                    bytes[start..start + 4].copy_from_slice(&f32::NAN.to_le_bytes());
                }
                7 => {
                    let start = section(&bytes, 7); // name indices
                    bytes[start..start + 4].copy_from_slice(&u32::MAX.to_le_bytes());
                }
                8 => {
                    let start = section(&bytes, 6); // stable IDs
                    let id = bytes[start..start + 8].to_vec();
                    bytes[start + 8..start + 16].copy_from_slice(&id);
                }
                9 => {
                    let start = section(&bytes, 8); // designations
                    bytes[start] = 255;
                }
                10 => {
                    let start = section(&bytes, GRID_OFFSETS);
                    bytes[start..start + 8].copy_from_slice(&1_u64.to_le_bytes());
                }
                _ => {
                    let start = section(&bytes, ENDPOINTS);
                    bytes[start..start + 8].copy_from_slice(&u64::MAX.to_le_bytes());
                }
            }
            if case != 0 && case != 2 && case != 3 {
                fix_checksum(&mut bytes);
            } // test semantic checks independently of CRC
            fs::write(&path, &bytes).unwrap();
            assert!(load_cached_catalog(&path, &fingerprint).is_err(), "case {case}");
        }
    }
    #[test]
    fn corrupt_cache_rebuilds_and_unwritable_cache_locations_do_not_block_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.csv");
        fs::write(&source, b"ra,dec,mag,proper\n0,0,5,Example\n").unwrap();
        let dirs = DatasetDirectories {
            data: None,
            cache: Some(root.path().join("cache")),
        };
        let dataset = Dataset::Path(source.clone());
        let mut notices = Vec::new();
        let expected = load_sky_catalog(Some(&dataset), &dirs, &mut notices).unwrap();
        let path = cache_path(&source, dirs.cache.as_ref().unwrap()).unwrap();
        assert_eq!(load_cached_catalog(&path, &super::super::loading::fingerprint_source(&Some(path.clone()))).unwrap(), expected);
        assert_eq!(load_sky_catalog(Some(&dataset), &dirs, &mut notices).unwrap(), expected);
        fs::write(&path, b"broken cache").unwrap();
        let rebuilt = load_sky_catalog(Some(&dataset), &dirs, &mut notices).unwrap();
        assert_eq!(expected, rebuilt);
        assert!(String::from_utf8_lossy(&notices).contains("rebuilding from CSV"));
        let blocked = root.path().join("not-a-directory");
        fs::write(&blocked, b"blocked").unwrap();
        let dirs = DatasetDirectories {
            data: None,
            cache: Some(blocked),
        };
        let uncached = load_sky_catalog(Some(&dataset), &dirs, &mut notices).unwrap();
        assert_eq!(uncached, expected);
        assert!(String::from_utf8_lossy(&notices).contains("continuing without a cache"));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3); // no cache/partial file next to the source
    }
    #[test]
    fn path_size_mtime_and_fingerprint_invalidate_stale_caches() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        fs::write(&source, b"one").unwrap();
        let first = cache_path(&source, dir.path()).unwrap();
        fs::write(&source, b"longer").unwrap();
        assert_ne!(first, cache_path(&source, dir.path()).unwrap());
        let before = cache_path(&source, dir.path()).unwrap();
        let file = fs::File::options().write(true).open(&source).unwrap();
        file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(123456789))
            .unwrap();
        assert_ne!(before, cache_path(&source, dir.path()).unwrap());
        let path = dir.path().join("cache");
        let mut fingerprint = catalog_fingerprint();
        write_cached_catalog(&path, &prepared(), &fingerprint).unwrap();
        fingerprint[0] ^= 1;
        assert!(load_cached_catalog(&path, &fingerprint).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn read_only_source_and_cache_folders_are_supported() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let cache = root.path().join("cache");
        fs::create_dir(&data).unwrap();
        fs::create_dir(&cache).unwrap();
        let source = data.join("stars.csv");
        fs::write(&source, b"ra,dec,mag\n0,0,5\n").unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o555)).unwrap();
        let dirs = DatasetDirectories {
            data: None,
            cache: Some(cache.clone()),
        };
        let mut notices = Vec::new();
        let result = load_sky_catalog(Some(&Dataset::Path(source)), &dirs, &mut notices);
        fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o755)).unwrap();
        let sky = result.unwrap();
        assert_eq!(sky.stars.len(), 1);
        assert!(String::from_utf8_lossy(&notices).contains("continuing without a cache"));
        assert_eq!(fs::read_dir(&data).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);
    }

    #[test]
    fn precision_exceptions_names_and_all_designations_survive_cached_loading() {
        use crate::astro::{Equatorial, Vector3};
        use crate::catalog::{CatalogStar, Designation, SpaceMotion, StarId};
        let mut input = load_embedded_catalog().unwrap();
        input.stars.clear();
        input.constellations.clear();
        input.hr_representatives.clear();
        let u = Equatorial {
            right_ascension: 0.7,
            declination: 0.4,
        }
        .to_unit_vector();
        let v = u * -0.001
            + Vector3 {
                x: -0.7_f64.sin(),
                y: 0.7_f64.cos(),
                z: 0.0,
            } * 1.01e-6;
        let name = input.names.insert("A unicode name: α星");
        for (i, designation) in [
            None,
            Some(Designation::Bayer {
                letter: 0,
                component: 1,
                constellation: *b"Ori",
            }),
            Some(Designation::Flamsteed {
                number: 61,
                constellation: *b"Cyg",
            }),
            Some(Designation::Hr(123)),
            Some(Designation::Hip(987)),
            Some(Designation::Tycho {
                region: 12,
                number: 34,
                component: 1,
            }),
            Some(Designation::Gaia(u64::MAX)),
        ]
        .into_iter()
        .enumerate()
        {
            input.stars.push(CatalogStar {
                id: StarId(i as u64),
                hr: None,
                name: Some(name),
                designation,
                space_motion: Some(SpaceMotion {
                    distance_pc: 1.0,
                    position: u,
                    velocity: v,
                }),
                right_ascension: 0.7,
                declination: 0.4,
                ra_motion: 0.0,
                ra_motion_cos_dec: 0.0,
                dec_motion: 0.0,
                magnitude: 5.0,
                spectral_type: *b"G2",
                color_index: Some(0.0),
                has_data: true,
            });
        }
        let catalog = crate::sky::prepare_owned_catalog(input);
        assert_eq!(catalog.stars.precise_count(), 7);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache");
        let fingerprint = catalog_fingerprint();
        write_cached_catalog(&path, &catalog, &fingerprint).unwrap();
        let cached = load_cached_catalog(&path, &fingerprint).unwrap();
        assert_eq!(cached, catalog);
        for storage in [&catalog.stars, &cached.stars] {
            let trajectories = storage.borrow_trajectory_fields();
            for i in 0..storage.len() {
                let full = storage.get(i);
                assert_eq!(trajectories.motion(i), full.motion);
                assert_eq!(storage.designation(i).resolve(), full.designation);
            }
        }

        assert_eq!(render(cached, 5.0, J2000), render(catalog, 5.0, J2000));
    }

    #[test]
    fn concurrent_writers_publish_complete_equivalent_catalogs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache");
        let source = prepared();
        let fingerprint = catalog_fingerprint();
        std::thread::scope(|scope| {
            let first = scope.spawn(|| write_cached_catalog(&path, &source, &fingerprint));
            let second = scope.spawn(|| write_cached_catalog(&path, &source, &fingerprint));
            first.join().unwrap().unwrap();
            second.join().unwrap().unwrap();
        });
        assert_eq!(load_cached_catalog(&path, &fingerprint).unwrap(), source);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_valid_cache_for_another_source_is_not_reused() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a.csv");
        let b = root.path().join("b.csv");
        fs::write(&a, b"ra,dec,mag\n0,0,1\n").unwrap();
        fs::write(&b, b"ra,dec,mag\n1,2,3\n").unwrap();
        let dirs = DatasetDirectories {
            data: None,
            cache: Some(root.path().join("cache")),
        };
        let mut notices = Vec::new();
        load_sky_catalog(Some(&Dataset::Path(a.clone())), &dirs, &mut notices).unwrap();
        fs::copy(
            cache_path(&a, dirs.cache.as_ref().unwrap()).unwrap(),
            cache_path(&b, dirs.cache.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        let actual = load_sky_catalog(Some(&Dataset::Path(b)), &dirs, &mut notices).unwrap();
        assert_eq!(actual.stars.magnitude(0), 3.0);
        assert!(String::from_utf8_lossy(&notices).contains("fingerprint mismatch"));
    }

    #[test]
    fn invalid_prepared_data_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache");
        let mut source = prepared();
        source.always_checked = vec![usize::MAX].into();
        assert!(write_cached_catalog(&path, &source, &catalog_fingerprint()).is_err());
        assert!(!path.exists());
    }
}

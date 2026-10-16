//! Per-row accessors over the validated columns that the loaders and the table builder read by index.
use super::StarStorage;
use crate::catalog::NameId;

impl StarStorage {
    pub(crate) fn brightness_keys(&self) -> &[u16] {
        self.rows.brightness_key.as_slice()
    }
    pub fn name(&self, index: usize) -> Option<NameId> {
        let name = self.name_entries().get(index).copied().unwrap_or(0);
        NameId::from_entry(name)
    }
    pub fn display_color(&self, index: usize) -> crate::model::StarColor {
        crate::model::StarColor::from_index(self.display_color_index(index)).expect("validated color index")
    }
    /// The stored palette index itself (validated on load), for records that keep the compact form.
    pub fn display_color_index(&self, index: usize) -> u8 { self.rows.display_color.as_slice()[index] }
}

#[cfg(test)]
mod tests {
    use super::super::expand;
    use crate::formulas::Vector3;
    use crate::sky::{catalog_fingerprint, load_cached_catalog, write_cached_catalog};

    #[test]
    fn source_and_cached_rows_match_full_records() {
        let mut parsed = crate::catalog::load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap();
        parsed.stars[0].space_motion = Some(crate::catalog::SpaceMotion {
            distance_pc: 1.0,
            position: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            velocity: Vector3 { x: 0.0, y: 1e-7, z: 0.0 },
        });
        let owned = crate::sky::prepare_owned_catalog(parsed, &mut crate::progress::Report::quiet()).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog");
        let fingerprint = catalog_fingerprint();
        write_cached_catalog(&path, &owned, &fingerprint, &mut crate::progress::Report::quiet()).unwrap();
        let cached = load_cached_catalog(&path, &fingerprint, &mut crate::progress::Report::quiet()).unwrap();
        for catalog in [&owned.catalog, &cached.catalog] {
            let columns = catalog.stars.columns();
            for index in 0..catalog.stars.len() {
                let full = catalog.stars.get(index);
                assert_eq!(catalog.names.get(catalog.stars.name(index)), catalog.names.get(full.name));
                assert_eq!(catalog.stars.motion(index), full.motion);
                assert_eq!(expand(columns.u0[index]), catalog.stars.stored_direction(index));
                assert_eq!(catalog.stars.display_color(index), full.display_color);
            }
        }
    }
}

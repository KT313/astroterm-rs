//! Copy one column into sorted order at a time, retaining its planned capacity and exact stored bits.
use super::{StarStorage, StarLabels};
use crate::{catalog::cache::invalid, progress::DatasetProgress};
use std::io;

impl StarStorage {
    /// The caller supplies a permutation from unpublished preparation data. On failure, discard that catalog.
    pub(crate) fn gather_ordered(&mut self, order: impl ExactSizeIterator<Item = usize> + Clone, bounds: &mut Vec<f32>, progress: &mut dyn DatasetProgress) -> io::Result<()> {
        let n = self.len();
        if order.len() != n || bounds.len() != n || order.clone().any(|index| index >= n) { return Err(invalid("invalid catalog permutation")); }
        let mut completed = 0;
        let rows = &mut self.rows;
        gather_column(&mut rows.u0, order.clone(), progress, &mut completed)?;
        gather_column(&mut rows.w, order.clone(), progress, &mut completed)?;
        gather_column(&mut rows.magnitude, order.clone(), progress, &mut completed)?;
        gather_column(&mut rows.brightness_key, order.clone(), progress, &mut completed)?;
        gather_column(&mut rows.distance, order.clone(), progress, &mut completed)?;
        gather_column(&mut rows.id, order.clone(), progress, &mut completed)?;
        gather_column(&mut rows.display_color, order.clone(), progress, &mut completed)?;
        match &mut self.labels {
            StarLabels::Stored(entries) => gather_column(entries, order.clone(), progress, &mut completed)?,
            StarLabels::Gaia(ids) => gather_column(ids, order.clone(), progress, &mut completed)?,
        }
        gather_column(bounds, order, progress, &mut completed)
    }
}

fn gather_column<T: Copy>(values: &mut Vec<T>, order: impl ExactSizeIterator<Item = usize>, progress: &mut dyn DatasetProgress, completed: &mut u64) -> io::Result<()> {
    if values.len() != order.len() { return Err(invalid("column length differs from catalog permutation")); }
    let mut output = Vec::new();
    output.try_reserve_exact(values.capacity()).map_err(io::Error::other)?;                       // preserve the retained capacity chosen during loading
    for index in order {
        output.push(values[index]);
        *completed += 1;
        if completed.is_multiple_of(crate::constants::STARTUP_PROGRESS_ROWS) { progress.set_position(*completed); }
    }
    *values = output;                                                                          // drop this old column before allocating its replacement for the next field
    progress.set_position(*completed);
    Ok(())
}

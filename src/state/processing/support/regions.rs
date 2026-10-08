//! Aggregate regional payload sizes without adding one inventory row per region.
#[cfg(feature = "memory-diagnostics")]
pub(in crate::state::processing) fn report_region_storage<T>(sink: &mut dyn crate::cache::BufferSink, name: &str, entries: &Vec<T>, nested: impl Fn(&T) -> Option<(usize, usize)>) {
    use crate::cache::Quality;
    if !sink.enter(name, std::mem::size_of_val(entries)) { return; }
    sink.payload(entries.len(), entries.capacity(), std::mem::size_of::<T>(), Quality::ExactPayload, "original regional slots; result allocations counted separately");
    let sizes = entries.iter().try_fold((0usize, 0usize), |(used, reserved), entry| {
        let (extra_used, extra_reserved) = nested(entry)?;
        Some((used.checked_add(extra_used)?, reserved.checked_add(extra_reserved)?))
    });
    if sink.enter("results", 0) {
        if let Some((used, reserved)) = sizes { sink.payload(used, reserved, 1, Quality::ExactPayload, "nested regional result bytes, including offscreen and invalidated entries"); }
        else { sink.unknown("regional payload size overflow"); }
        sink.leave();
    }
    sink.leave();
}

#[cfg(feature = "memory-diagnostics")]
pub(in crate::state::processing) fn cached_vector_bytes<K, T>(entry: &crate::cache::Cache<K, Vec<T>>) -> Option<(usize, usize)> {
    assert!(!std::mem::needs_drop::<K>() && !std::mem::needs_drop::<T>(), "regional keys and vector elements must not hide allocations");
    entry.stored().map_or(Some((0, 0)), |values| Some((values.len().checked_mul(std::mem::size_of::<T>())?, values.capacity().checked_mul(std::mem::size_of::<T>())?)))
}

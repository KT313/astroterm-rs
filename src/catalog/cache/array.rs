//! Owned catalog arrays. Prepared disk sections are decoded into these vectors before the catalog is published.
use bytemuck::Pod;
use std::ops::Deref;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogArray<T: Pod>(Vec<T>);
impl<T: Pod> Default for CatalogArray<T> {
    fn default() -> Self { Self(Vec::new()) }
}
impl<T: Pod> From<Vec<T>> for CatalogArray<T> {
    fn from(value: Vec<T>) -> Self { Self(value) }
}
impl<T: Pod> Deref for CatalogArray<T> {
    type Target = [T];
    fn deref(&self) -> &[T] { &self.0 }
}
impl<T: Pod> CatalogArray<T> {
    pub fn capacity(&self) -> usize { self.0.capacity() }
    pub(crate) fn shrink_to_fit(&mut self) { self.0.shrink_to_fit(); }
    pub(crate) fn bytes(&self) -> &[u8] { bytemuck::cast_slice(&self.0) }
    pub(crate) fn push(&mut self, value: T) { self.0.push(value); }
}
#[cfg(feature = "memory-diagnostics")]
impl<T: Pod + crate::cache::ReportBuffers> crate::cache::ReportBuffers for CatalogArray<T> {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        crate::cache::report_field(sink, "owned", &self.0);
    }
}

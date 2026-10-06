//! Owned construction arrays and immutable slices of a validated catalog mapping. No borrowed field outlives its
//! owner: every mapped array holds an Arc to the mapping and a section number, never a self-referential slice.
use super::MappedCatalog;
use bytemuck::Pod;
use std::{fmt, ops::Deref, sync::Arc};

#[derive(Clone)]
pub enum CatalogArray<T: Pod> {
    Owned(Vec<T>),
    Mapped {
        catalog: Arc<MappedCatalog>,
        section: usize,
        marker: std::marker::PhantomData<T>,
    },
}
impl<T: Pod> Default for CatalogArray<T> {
    fn default() -> Self {
        Self::Owned(Vec::new())
    }
}
impl<T: Pod + PartialEq> PartialEq for CatalogArray<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl<T: Pod + Eq> Eq for CatalogArray<T> {}
impl<T: Pod + fmt::Debug> fmt::Debug for CatalogArray<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(f)
    }
}
impl<T: Pod> From<Vec<T>> for CatalogArray<T> {
    fn from(value: Vec<T>) -> Self {
        Self::Owned(value)
    }
}
impl<T: Pod> Deref for CatalogArray<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        match self {
            Self::Owned(values) => values,
            Self::Mapped { catalog, section, .. } => catalog.slice(*section).expect("validated mapped array"),
        }
    }
}
impl<T: Pod> CatalogArray<T> {
    pub(crate) fn from_mapping(catalog: &Arc<MappedCatalog>, section: usize) -> std::io::Result<Self> {
        catalog.slice::<T>(section)?;
        Ok(Self::Mapped {
            catalog: catalog.clone(),
            section,
            marker: std::marker::PhantomData,
        })
    }
    pub fn is_mapped(&self) -> bool {
        matches!(self, Self::Mapped { .. })
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        bytemuck::cast_slice(self)
    }
    fn owned_mut(&mut self) -> &mut Vec<T> {
        if let Self::Mapped { .. } = self {
            *self = Self::Owned(self.to_vec());
        }
        match self {
            Self::Owned(v) => v,
            Self::Mapped { .. } => unreachable!(),
        }
    }
    pub(crate) fn push(&mut self, value: T) {
        self.owned_mut().push(value);
    }
    pub(crate) fn swap(&mut self, a: usize, b: usize) {
        self.owned_mut().swap(a, b);
    }
    pub(crate) fn reserve(&mut self, capacity: usize) {
        self.owned_mut().reserve(capacity);
    }
    pub(crate) fn shrink_to_fit(&mut self) {
        self.owned_mut().shrink_to_fit();
    }
}

#[cfg(feature = "memory-diagnostics")]
impl<T: Pod + crate::cache::ReportBuffers> crate::cache::ReportBuffers for CatalogArray<T> {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        use crate::cache::report_field;
        match self {
            Self::Owned(values) => report_field(sink, "owned", values),
            Self::Mapped { catalog, .. } => {
                sink.borrowed(self.len(), std::mem::size_of::<T>(), "validated mapped section; view bytes already belong to the shared mapping");
                report_field(sink, "mapped_owner", catalog);
            },
        }
    }
}

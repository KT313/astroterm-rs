//! Catalog-owned UTF-8 names in one contiguous block. Offsets survive star reordering.

/// Byte range within its owning catalog's name block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NameId {
    start: usize,
    end: usize,
}

impl NameId {
    pub(crate) fn range(self) -> [u64; 2] {
        [self.start as u64, self.end as u64]
    }
    pub(crate) fn from_range(range: [u64; 2]) -> Self {
        Self {
            start: range[0] as usize,
            end: range[1] as usize,
        }
    }
}

/// Owned string block, shared in format by parsed catalogs and skies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StarNames {
    text: super::cache::CatalogArray<u8>,
}

impl StarNames {
    pub(crate) fn capacity(&self) -> usize { self.text.capacity() }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.text
    }
    pub(crate) fn from_array(text: super::cache::CatalogArray<u8>) -> std::io::Result<Self> {
        std::str::from_utf8(&text).map_err(|_| super::cache::invalid("invalid UTF-8 name block"))?;
        Ok(Self { text })
    }
    pub fn insert(&mut self, name: &str) -> NameId {
        let start = self.text.len();
        for &byte in name.as_bytes() {
            self.text.push(byte);
        }
        NameId {
            start,
            end: self.text.len(),
        }
    }

    pub fn get(&self, name: Option<NameId>) -> Option<&str> {
        let name = name?;
        std::str::from_utf8(self.text.get(name.start..name.end)?).ok()
    }
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StarNames { text });

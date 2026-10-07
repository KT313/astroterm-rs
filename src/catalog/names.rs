//! Shared UTF-8 display labels, compact boundaries and sparse ASCII alternatives. References survive star sorting.
use super::cache::{CatalogArray, invalid};
use std::{collections::HashMap, io};

/// One-based label entry; zero in a stored star column means no label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NameId(u32);
impl NameId {
    pub(crate) fn entry(self) -> u32 { self.0 }
    pub(crate) fn from_entry(entry: u32) -> Option<Self> { (entry != 0).then_some(Self(entry)) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StarNames {
    text: CatalogArray<u8>,
    boundaries: CatalogArray<u32>,
    ascii_alternatives: CatalogArray<[u32; 2]>, // sorted [Unicode entry, ASCII entry]; most labels need no row
}
impl Default for StarNames {
    fn default() -> Self { Self { text: Default::default(), boundaries: vec![0].into(), ascii_alternatives: Default::default() } }
}
impl StarNames {
    pub(crate) fn capacity(&self) -> usize { self.text.capacity() }
    pub(crate) fn bytes(&self) -> &[u8] { &self.text }
    pub(crate) fn boundaries(&self) -> &CatalogArray<u32> { &self.boundaries }
    pub(crate) fn ascii_alternatives(&self) -> &CatalogArray<[u32; 2]> { &self.ascii_alternatives }
    pub(crate) fn contains(&self, entry: u32) -> bool { entry == 0 || (entry as usize) < self.boundaries.len() }

    pub(crate) fn from_arrays(text: CatalogArray<u8>, boundaries: CatalogArray<u32>, ascii_alternatives: CatalogArray<[u32; 2]>) -> io::Result<Self> {
        let names = Self { text, boundaries, ascii_alternatives };
        names.validate()?;
        Ok(names)
    }
    pub(crate) fn validate(&self) -> io::Result<()> {
        let text = std::str::from_utf8(&self.text).map_err(|_| invalid("invalid UTF-8 label buffer"))?;
        let end = u32::try_from(text.len()).map_err(|_| invalid("label text exceeds u32 byte range"))?;
        if self.boundaries.first() != Some(&0) || self.boundaries.last() != Some(&end)
            || self.boundaries.len().saturating_sub(1) as u64 > u64::from(u32::MAX)
            || self.boundaries.windows(2).any(|p| p[0] > p[1])
            || self.boundaries.iter().any(|&b| !text.is_char_boundary(b as usize)) {
            return Err(invalid("invalid label boundaries"));
        }
        let mut previous = 0;
        for &[primary, alternative] in self.ascii_alternatives.iter() {
            if primary <= previous || alternative == 0 || !self.contains(primary) || !self.contains(alternative)
                || primary == alternative {
                return Err(invalid("invalid ASCII label alternative"));
            }
            previous = primary;
        }
        Ok(())
    }
    pub fn insert(&mut self, name: &str) -> io::Result<NameId> {
        let end = checked_text_end(self.text.len(), name.len())?;
        let entry = u32::try_from(self.boundaries.len()).map_err(|_| invalid("label count exceeds u32 range"))?;
        for &byte in name.as_bytes() { self.text.push(byte); }
        self.boundaries.push(end);
        Ok(NameId(entry))
    }
    pub fn get(&self, name: Option<NameId>) -> Option<&str> {
        let index = name?.0 as usize;
        let start = *self.boundaries.get(index - 1)? as usize;
        let end = *self.boundaries.get(index)? as usize;
        std::str::from_utf8(self.text.get(start..end)?).ok()
    }
    pub fn get_for_mode(&self, name: Option<NameId>, unicode: bool) -> Option<&str> {
        let mut name = name?;
        if !unicode && let Ok(index) = self.ascii_alternatives.binary_search_by_key(&name.0, |pair| pair[0]) {
            name = NameId(self.ascii_alternatives[index][1]);
        }
        self.get(Some(name))
    }
}
fn checked_text_end(length: usize, added: usize) -> io::Result<u32> {
    length.checked_add(added).and_then(|end| u32::try_from(end).ok())
        .ok_or_else(|| invalid("label text exceeds u32 byte range (4,294,967,295 bytes)"))
}

/// Preparation-only dictionary. Equal text with equal ASCII behavior shares one entry; the dictionary drops on finish.
#[derive(Default)]
pub(crate) struct LabelBuilder {
    names: StarNames,
    entries: HashMap<(String, Option<String>), NameId>,
}
impl LabelBuilder {
    pub fn insert(&mut self, text: String, ascii: Option<String>) -> io::Result<NameId> {
        let ascii = ascii.filter(|a| a != &text);
        let key = (text, ascii);
        if let Some(&id) = self.entries.get(&key) { return Ok(id); }
        let id = self.names.insert(&key.0)?;
        if let Some(ascii) = &key.1 {
            let alternative = self.names.insert(ascii)?;
            self.names.ascii_alternatives.push([id.0, alternative.0]);
        }
        self.entries.insert(key, id);
        Ok(id)
    }
    pub fn finish(mut self) -> StarNames {
        self.names.text.shrink_to_fit();
        self.names.boundaries.shrink_to_fit();
        self.names.ascii_alternatives.shrink_to_fit();
        self.names
    }
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StarNames { text, boundaries, ascii_alternatives });

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_labels_preserve_alternatives_and_literal_proper_names() {
        let mut builder = LabelBuilder::default();
        let id = builder.insert("α² Cen".into(), Some("Alp2 Cen".into())).unwrap();
        assert_eq!(id, builder.insert("α² Cen".into(), Some("Alp2 Cen".into())).unwrap());
        let proper = builder.insert("α² Cen".into(), None).unwrap();
        let names = builder.finish();
        names.validate().unwrap();
        assert_eq!(names.get_for_mode(Some(id), true), Some("α² Cen"));
        assert_eq!(names.get_for_mode(Some(id), false), Some("Alp2 Cen"));
        assert_eq!(names.get_for_mode(Some(proper), false), Some("α² Cen"));
        assert_eq!(names.ascii_alternatives.len(), 1);
        assert_eq!(names.get(None), None);
    }
    #[test]
    fn boundaries_and_alternatives_are_validated_without_large_allocations() {
        StarNames::default().validate().unwrap();
        assert_eq!(checked_text_end(u32::MAX as usize - 1, 1).unwrap(), u32::MAX);
        assert!(checked_text_end(u32::MAX as usize, 1).is_err());
        assert!(checked_text_end(usize::MAX, 1).is_err());
        for boundaries in [vec![], vec![1, 2], vec![0, 1, 2], vec![0, 3], vec![0, 2, 1, 2]] {
            assert!(StarNames::from_arrays("α".as_bytes().to_vec().into(), boundaries.into(), vec![].into()).is_err());
        }
        for pairs in [vec![[0, 1]], vec![[1, 0]], vec![[1, 3]], vec![[1, 1]], vec![[2, 1], [1, 2]]] {
            assert!(StarNames::from_arrays(b"ab".to_vec().into(), vec![0, 1, 2].into(), pairs.into()).is_err());
        }
    }
}

//! Catalog-owned UTF-8 names in one contiguous block. Offsets survive star reordering.

/// Byte range within its owning catalog's name block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NameId {
    start: usize,
    end: usize,
}

/// Owned string block, shared in format by parsed catalogs and skies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StarNames {
    text: String,
}

impl StarNames {
    pub fn insert(&mut self, name: &str) -> NameId {
        let start = self.text.len();
        self.text.push_str(name);
        NameId {
            start,
            end: self.text.len(),
        }
    }

    pub fn get(&self, name: Option<NameId>) -> Option<&str> {
        let name = name?;
        self.text.get(name.start..name.end)
    }
}

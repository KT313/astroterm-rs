//! Owned cache arrays and the streaming, checksummed prepared-catalog container.
mod array;
mod codec;
pub use array::CatalogArray;
pub(crate) use codec::{PreparedCatalogReader, write_sections_with_times};

pub(crate) fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}
pub(crate) fn supported() -> bool { cfg!(target_endian = "little") && usize::BITS == 64 }

#[cfg(test)]
pub(crate) use codec::write_sections;

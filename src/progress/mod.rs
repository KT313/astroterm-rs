//! Dataset startup reporting without terminal or catalog dependencies. Headless callers retain their writer.
use std::io::{self, Read, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage { Resolve, Verify, Connect, Download, Install, Parse, Convert, Sort, Index, ReadCache, ValidateCache, WriteCache, FinalizeCache }
impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Resolve => "Locating dataset", Self::Verify => "Verifying source", Self::Connect => "Waiting for server", Self::Download => "Downloading",
            Self::Install => "Installing dataset", Self::Parse => "Loading stars", Self::Convert => "Preparing stars", Self::Sort => "Sorting stars by region",
            Self::Index => "Building sky index", Self::ReadCache => "Reading prepared catalog", Self::ValidateCache => "Validating prepared catalog",
            Self::WriteCache => "Saving prepared catalog", Self::FinalizeCache => "Finalizing prepared catalog",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit { Bytes, Rows, Stars, Work }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome { Complete, Failed }

pub trait DatasetProgress: Write {
    fn begin_stage(&mut self, _stage: Stage, _detail: &str, _unit: Unit, _total: Option<u64>) {}
    fn set_position(&mut self, _completed: u64) {}
    fn set_retained(&mut self, _retained: u64) {}
    fn set_files(&mut self, _ready: u64, _total: u64) {}
    fn finish_stage(&mut self, _outcome: Outcome) {}
}

pub struct NoProgress<W: Write>(pub W);
impl<W: Write> Write for NoProgress<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.0.write(bytes) }
    fn flush(&mut self) -> io::Result<()> { self.0.flush() }
}
impl<W: Write> DatasetProgress for NoProgress<W> {}

/// Report an existing read without adding another pass or changing its error behavior.
pub struct ProgressReader<'a, R> { reader: R, progress: &'a mut dyn DatasetProgress, bytes: u64 }
impl<'a, R: Read> ProgressReader<'a, R> {
    pub fn new(reader: R, progress: &'a mut dyn DatasetProgress) -> Self { Self { reader, progress, bytes: 0 } }
}
impl<R: Read> Read for ProgressReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let read = self.reader.read(bytes)?;
        self.bytes += read as u64;
        self.progress.set_position(self.bytes);
        Ok(read)
    }
}

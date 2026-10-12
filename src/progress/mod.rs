//! Dataset startup reporting without terminal or catalog dependencies. Headless callers retain their writer.
use std::io::{self, Read, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage { Resolve, Verify, Connect, Download, Install, Parse, Convert, PlanDownloads, CheckIds, AssignRegions, Reorder, Sort, Index, Group, ReadCache, ValidateCache, WriteCache, FinalizeCache }
impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Resolve => "Locating dataset", Self::Verify => "Verifying source", Self::Connect => "Waiting for server", Self::Download => "Downloading",
            Self::Install => "Installing dataset", Self::Parse => "Loading stars", Self::Convert => "Preparing stars", Self::Sort => "Sorting stars by region",
            Self::PlanDownloads => "Planning dataset downloads", Self::CheckIds => "Checking Gaia source IDs",
            Self::AssignRegions => "Assigning star regions", Self::Reorder => "Reordering star fields",
            Self::Index => "Building sky index", Self::Group => "Grouping stars by speed", Self::ReadCache => "Reading prepared catalog", Self::ValidateCache => "Validating prepared catalog",
            Self::WriteCache => "Saving prepared catalog", Self::FinalizeCache => "Finalizing prepared catalog",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit { Bytes, Rows, Stars, Blocks, Elements, Work }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome { Complete, Failed }

pub trait DatasetProgress: Write {
    fn begin_stage(&mut self, _stage: Stage, _detail: &str, _unit: Unit, _total: Option<u64>) {}
    fn set_position(&mut self, _completed: u64) {}
    /// Change the activity text without resetting the parent counter, timer or ETA baseline.
    fn set_detail(&mut self, _detail: &str) {}
    /// Expose a known work total after an indeterminate substep, retaining the same stage and elapsed time.
    fn set_work(&mut self, _unit: Unit, _total: Option<u64>) {}
    /// Actual bytes consumed within a block; never advance the parent completed-block counter.
    fn set_read_progress(&mut self, _bytes: u64, _total: u64) {}
    /// Count existing blocks as ready without treating them as work completed during this download.
    fn set_cached_blocks(&mut self, ready: u64) { self.set_position(ready); }
    /// Received download payload, including retries, and elapsed time for this download phase.
    fn set_download_bytes(&mut self, _received: u64, _elapsed: std::time::Duration) {}
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

/// Report payload reads inside a parent block stage without replacing its progress bar.
pub(crate) struct ActivityReader<'a, R> { reader: R, progress: &'a mut dyn DatasetProgress, bytes: u64, total: u64 }
impl<'a, R: Read> ActivityReader<'a, R> {
    pub(crate) fn new(reader: R, progress: &'a mut dyn DatasetProgress, total: u64) -> Self {
        progress.set_read_progress(0, total);
        Self { reader, progress, bytes: 0, total }
    }
}
impl<R: Read> Read for ActivityReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let read = self.reader.read(bytes)?;
        self.bytes += read as u64;
        self.progress.set_read_progress(self.bytes, self.total);
        Ok(read)
    }
}

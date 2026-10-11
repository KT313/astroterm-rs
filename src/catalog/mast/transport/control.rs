//! Shared download cancellation and received payload bytes. Active HTTP reads keep their timeout.
use std::{io, sync::{Condvar, Mutex, atomic::{AtomicU64, Ordering}}, time::{Duration, Instant}};

#[derive(Default)]
pub(in crate::catalog::mast) struct TransferControl { stopped: Mutex<bool>, wake: Condvar, received: AtomicU64 }
impl TransferControl {
    pub fn add_received(&self, bytes: usize) { self.received.fetch_add(bytes as u64, Ordering::Relaxed); }
    pub fn received_bytes(&self) -> u64 { self.received.load(Ordering::Relaxed) }
    pub fn stop(&self) {
        *self.stopped.lock().unwrap_or_else(|error| error.into_inner()) = true;
        self.wake.notify_all();
    }
    pub fn check(&self) -> io::Result<()> {
        if *self.stopped.lock().unwrap_or_else(|error| error.into_inner()) { return Err(io::Error::new(io::ErrorKind::Interrupted, "MAST download cancelled")); }
        Ok(())
    }
    pub fn pause(&self, delay: Duration, deadline: Instant) -> io::Result<()> {
        self.check()?;
        if Instant::now() + delay >= deadline { return Err(io::Error::new(io::ErrorKind::TimedOut, "MAST retry deadline exceeded")); }
        let stopped = self.stopped.lock().unwrap_or_else(|error| error.into_inner());
        let _ = self.wake.wait_timeout_while(stopped, delay, |stopped| !*stopped).unwrap_or_else(|error| error.into_inner());
        self.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopping_wakes_retry_backoff_and_blocks_new_requests() {
        let cancel = std::sync::Arc::new(TransferControl::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            let result = worker_cancel.pause(Duration::from_secs(60), Instant::now() + Duration::from_secs(120));
            sender.send(result.unwrap_err().kind()).unwrap();
        });
        cancel.stop();
        assert_eq!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(), io::ErrorKind::Interrupted);
        worker.join().unwrap();
        assert_eq!(cancel.check().unwrap_err().kind(), io::ErrorKind::Interrupted);
    }
}

//! Bounded HTTP reads with exact range and object-identity checks; HTTP decompression stays disabled.
use crate::constants::{MAST_REQUEST_SECONDS, MAST_REQUEST_RETRIES};
use super::storage::invalid;
use std::{io::{self, Read}, time::{Duration, Instant}};
use serde::{Serialize, Deserialize};

pub(super) const BASE: &str = "https://stpubdata.s3.amazonaws.com/gaia/gaia_dr3/public/hats/gaia/";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Identity { pub etag: String, pub size: u64 }
pub(super) struct Response { pub bytes: Vec<u8>, pub identity: Option<Identity>, pub start: u64 }
pub(super) trait Fetch: Sync {
    fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant) -> io::Result<Response>;
}
pub(super) struct Http { agent: ureq::Agent }
impl Http {
    pub fn new() -> Self {
        Self { agent: ureq::Agent::config_builder().max_redirects(0).timeout_connect(Some(Duration::from_secs(30)))
            .timeout_global(Some(Duration::from_secs(MAST_REQUEST_SECONDS))).build().new_agent() }
    }
}
impl Fetch for Http {
    fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant) -> io::Result<Response> {
        for attempt in 0..=MAST_REQUEST_RETRIES {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return Err(io::Error::new(io::ErrorKind::TimedOut, "MAST block deadline exceeded")); }
            let mut request = self.agent.get(format!("{BASE}{path}")).header("Accept-Encoding", "identity")
                .config().http_status_as_error(false).timeout_global(Some(remaining.min(Duration::from_secs(MAST_REQUEST_SECONDS)))).build();
            if let Some(range) = range { request = request.header("Range", range); }
            if let Some(id) = identity { request = request.header("If-Match", &id.etag); }
            let response = request.call();
            let mut response = match response {
                Ok(r) => r,
                Err(e) if attempt == MAST_REQUEST_RETRIES => return Err(io::Error::other(e)),
                Err(_) => { pause(attempt, None, deadline)?; continue; }
            };
            let status = response.status().as_u16();
            if status == 429 || status >= 500 {
                if attempt == MAST_REQUEST_RETRIES { return Err(io::Error::other(format!("MAST HTTP {status}"))); }
                let retry = response.headers().get("Retry-After").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
                pause(attempt, retry, deadline)?; continue;
            }
            if status != if range.is_some() { 206 } else { 200 } { return Err(invalid(format!("MAST HTTP {status}; expected a complete metadata response or exact partial range"))); }
            if response.headers().get("Content-Encoding").is_some_and(|v| v != "identity") { return Err(invalid("unexpected HTTP compression on Parquet input")); }
            let mut start = 0;
            let mut length = None;
            let id = if let Some(range) = range {
                let text = response.headers().get("Content-Range").and_then(|v| v.to_str().ok()).ok_or_else(|| invalid("missing Content-Range"))?;
                let (first, last, total) = parse_range(text)?;
                let (expected_first, expected_last) = requested_range(range, total)?;
                if (first, last) != (expected_first, expected_last) { return Err(invalid("MAST returned the wrong byte range")); }
                start = first; length = Some(last - first + 1);
                let etag = response.headers().get("ETag").and_then(|v| v.to_str().ok()).ok_or_else(|| invalid("missing MAST ETag"))?.to_string();
                let id = Identity { etag, size: total };
                if id.etag.len() > 256 || id.etag.starts_with("W/") || identity.is_some_and(|expected| expected != &id) { return Err(invalid("MAST object identity changed")); }
                Some(id)
            } else { None };
            if length.is_some_and(|n| n > limit as u64) { return Err(invalid("MAST response exceeds byte budget")); }
            let mut bytes = Vec::new();
            match response.body_mut().as_reader().take(limit as u64 + 1).read_to_end(&mut bytes) {
                Ok(_) => {},
                Err(e) if attempt == MAST_REQUEST_RETRIES => return Err(e),
                Err(_) => { pause(attempt, None, deadline)?; continue; }
            }
            if bytes.len() > limit || length.is_some_and(|n| n != bytes.len() as u64) { return Err(invalid("truncated or oversized MAST response")); }
            return Ok(Response { bytes, identity: id, start });
        }
        unreachable!()
    }
}
fn pause(attempt: usize, retry: Option<u64>, deadline: Instant) -> io::Result<()> {
    let delay = Duration::from_secs(retry.unwrap_or(1 << attempt).min(60));
    if Instant::now() + delay >= deadline { return Err(io::Error::new(io::ErrorKind::TimedOut, "MAST retry deadline exceeded")); }
    std::thread::sleep(delay); Ok(())
}
fn parse_range(text: &str) -> io::Result<(u64, u64, u64)> {
    let (span, total) = text.strip_prefix("bytes ").and_then(|s| s.split_once('/')).ok_or_else(|| invalid("invalid Content-Range"))?;
    let (first, last) = span.split_once('-').ok_or_else(|| invalid("invalid Content-Range"))?;
    let parse = |s: &str| s.parse::<u64>().map_err(|_| invalid("invalid Content-Range number"));
    let (first, last, total) = (parse(first)?, parse(last)?, parse(total)?);
    if first > last || last >= total { return Err(invalid("invalid Content-Range bounds")); }
    Ok((first, last, total))
}
fn requested_range(range: &str, size: u64) -> io::Result<(u64, u64)> {
    let (first, last) = range.strip_prefix("bytes=").and_then(|s| s.split_once('-')).ok_or_else(|| invalid("invalid request range"))?;
    if first.is_empty() {
        let count = last.parse::<u64>().map_err(io::Error::other)?;
        return Ok((size.saturating_sub(count), size.checked_sub(1).ok_or_else(|| invalid("empty remote file"))?));
    }
    Ok((first.parse().map_err(io::Error::other)?, last.parse().map_err(io::Error::other)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_and_suffix_ranges_reject_invalid_response_bounds() {
        assert_eq!(parse_range("bytes 40-49/50").unwrap(), (40,49,50));
        assert_eq!(requested_range("bytes=-262144",50).unwrap(),(0,49));
        assert_eq!(requested_range("bytes=40-49",50).unwrap(),(40,49));
        for bad in ["bytes 5-4/10","bytes 0-10/10","bytes */10","items 0-1/2","bytes 0-1/*"] {assert!(parse_range(bad).is_err());}
    }
}

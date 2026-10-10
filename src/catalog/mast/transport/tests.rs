use super::*;
use std::{io::Write, net::TcpListener, thread};

/// Local HTTP replies exercise ureq and the production validation; no external dataset is contacted.
fn serve(replies: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/object", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                    Err(error) => panic!("HTTP test did not receive expected request: {error}"),
                }
            };
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
                assert!(bytes.len() < 16 * 1024);
            }
            requests.push(String::from_utf8(bytes).unwrap());
            let _ = stream.write_all(reply.as_bytes());                                          // rejecting a header may close the client before the body is written
        }
        requests
    });
    (url, worker)
}
fn reply(status: u16, headers: &str, body: &str) -> String {
    format!("HTTP/1.1 {status} Test\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}", body.len())
}
fn identity() -> Identity { Identity { etag: "\"v1\"".into(), size: 3 } }
fn range_headers() -> &'static str { "Content-Range: bytes 0-2/3\r\nETag: \"v1\"\r\n" }
fn deadline() -> Instant { Instant::now() + Duration::from_secs(5) }

#[test]
fn http_range_sends_identity_and_preserves_raw_body_bytes() {
    let (url, server) = serve(vec![reply(206, range_headers(), "abc")]);
    let response = Http::new().fetch_url(&url, Some("bytes=0-2"), Some(&identity()), 3, deadline()).unwrap();
    assert_eq!(response.bytes, b"abc");
    assert_eq!(response.identity, Some(identity()));
    let requests = server.join().unwrap();
    let request = requests[0].to_ascii_lowercase();
    assert!(request.contains("range: bytes=0-2\r\n"));
    assert!(request.contains("if-match: \"v1\"\r\n"));
    assert!(request.contains("accept-encoding: identity\r\n"));
}

#[test]
fn http_rejects_full_fallback_wrong_range_changed_identity_and_encoding() {
    for (response, message) in [
        (reply(200, "", "abc"), "HTTP 200"),
        (reply(206, "Content-Range: bytes 1-2/3\r\nETag: \"v1\"\r\n", "bc"), "wrong byte range"),
        (reply(412, "", ""), "HTTP 412"),
        (reply(206, "Content-Range: bytes 0-2/3\r\nETag: \"v2\"\r\n", "abc"), "identity changed"),
        (reply(206, &format!("{}Content-Encoding: gzip\r\n", range_headers()), "abc"), "unexpected HTTP compression"),
        (reply(206, "ETag: \"v1\"\r\n", "abc"), "missing Content-Range"),
    ] {
        let (url, server) = serve(vec![response]);
        let error = Http::new().fetch_url(&url, Some("bytes=0-2"), Some(&identity()), 3, deadline()).err().unwrap();
        assert!(error.to_string().contains(message), "{error}");
        assert_eq!(server.join().unwrap().len(), 1);
    }
}

#[test]
fn http_enforces_metadata_body_and_declared_range_budgets() {
    let (url, server) = serve(vec![reply(200, "", "abcd")]);
    let error = Http::new().fetch_url(&url, None, None, 3, deadline()).err().unwrap();
    assert!(error.to_string().contains("oversized"));
    server.join().unwrap();

    let (url, server) = serve(vec![reply(206, range_headers(), "abc")]);
    let error = Http::new().fetch_url(&url, Some("bytes=0-2"), None, 2, deadline()).err().unwrap();
    assert!(error.to_string().contains("byte budget"));
    server.join().unwrap();
}

#[test]
fn http_retries_transient_statuses_without_changing_range_or_pin() {
    let (url, server) = serve(vec![reply(429, "Retry-After: 0\r\n", ""), reply(503, "Retry-After: 0\r\n", ""), reply(206, range_headers(), "abc")]);
    let result = Http::new().fetch_url(&url, Some("bytes=0-2"), Some(&identity()), 3, deadline()).unwrap();
    assert_eq!(result.bytes, b"abc");
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn http_retry_limit_and_backoff_respect_one_deadline() {
    let (url, server) = serve(vec![reply(503, "Retry-After: 0\r\n", ""); MAST_REQUEST_RETRIES + 1]);
    assert!(Http::new().fetch_url(&url, None, None, 3, deadline()).err().unwrap().to_string().contains("HTTP 503"));
    assert_eq!(server.join().unwrap().len(), MAST_REQUEST_RETRIES + 1);
    for (attempt, expected) in [(0, 1), (1, 2), (2, 4)] { assert_eq!(retry_delay(attempt, None), Duration::from_secs(expected)); }
    assert_eq!(retry_delay(0, Some(100)), Duration::from_secs(60));
    assert_eq!(pause(0, Some(2), Instant::now() + Duration::from_millis(50)).unwrap_err().kind(), io::ErrorKind::TimedOut);

    let (url, server) = serve(vec![reply(503, "Retry-After: 2\r\n", "")]);
    let error = Http::new().fetch_url(&url, None, None, 3, Instant::now() + Duration::from_secs(1)).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(server.join().unwrap().len(), 1);
}

#[test]
fn exact_and_suffix_ranges_reject_invalid_response_bounds() {
    assert_eq!(parse_range("bytes 40-49/50").unwrap(), (40, 49, 50));
    assert_eq!(requested_range("bytes=-262144", 50).unwrap(), (0, 49));
    assert_eq!(requested_range("bytes=40-49", 50).unwrap(), (40, 49));
    for bad in ["bytes 5-4/10", "bytes 0-10/10", "bytes */10", "items 0-1/2", "bytes 0-1/*"] { assert!(parse_range(bad).is_err()); }
}

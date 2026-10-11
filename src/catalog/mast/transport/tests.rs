use super::*;
use std::{io::Write, net::TcpListener, thread};

/// Local HTTP replies exercise ureq and the production validation; no external dataset is contacted.
fn serve(replies: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
    serve_parts(replies.into_iter().map(|reply| vec![(Duration::ZERO, reply)]).collect())
}

/// Delays can target headers or body separately; request timeouts are tested with milliseconds, not minute-long sleeps.
fn serve_parts(replies: Vec<Vec<(Duration, String)>>) -> (String, thread::JoinHandle<Vec<String>>) {
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
            for (delay, part) in reply {
                thread::sleep(delay);
                if stream.write_all(part.as_bytes()).is_err() { break; }                         // a timed-out or rejected response may close the client early
            }
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
fn http_retains_connections_and_reuses_a_fully_read_response() {
    let http = Http::new();
    assert_eq!(http.agent.config().max_idle_connections(), COLUMNS.len() * usize::from(MAST_MAX_PARALLEL_BLOCKS));
    assert_eq!(http.agent.config().max_idle_connections_per_host(), COLUMNS.len() * usize::from(MAST_MAX_PARALLEL_BLOCKS));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/object", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let until = deadline();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < until => thread::sleep(Duration::from_millis(5)),
                Err(error) => panic!("HTTP test did not receive a connection: {error}"),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        for _ in 0..2 {
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 16 * 1024);
            }
            stream.write_all(format!("HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\n{}\r\nabc", range_headers()).as_bytes()).unwrap();
        }
    });

    let responses: Vec<_> = (0..2).map(|_| http.fetch_url(&url, Some("bytes=0-2"), Some(&identity()), 3, deadline())).collect();
    server.join().unwrap();                                                                     // both requests must arrive on the same accepted connection
    for response in responses { assert_eq!(response.unwrap().bytes, b"abc"); }
}

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

#[test]
fn received_payload_is_counted_during_reading_and_includes_retried_bytes() {
    struct Chunks<'a> { control: &'a TransferControl, reads: usize }
    impl Read for Chunks<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            assert_eq!(self.control.received_bytes(), self.reads as u64 * 3);                    // each prior read is visible before EOF
            if self.reads == 2 { return Ok(0); }
            bytes[..3].copy_from_slice(b"abc");
            self.reads += 1;
            Ok(3)
        }
    }
    let control = TransferControl::default();
    let mut reader = ReceivedReader { inner: Chunks { control: &control, reads: 0 }, control: &control };
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"abcabc");
    assert_eq!(control.received_bytes(), 6);

    let partial = "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 6\r\n\r\nabc".to_owned();
    let (url, server) = serve(vec![partial, reply(200, "", "abcdef")]);
    let control = TransferControl::default();
    let response = Http::new().fetch_url_cancellable(&url, None, None, 6, deadline(), &control).unwrap();
    server.join().unwrap();
    assert_eq!(response.bytes, b"abcdef");
    assert_eq!(control.received_bytes(), 9);                                                     // three discarded bytes plus the six-byte retry, counted once each
}

#[test]
fn http_separates_connection_and_header_limits_from_the_body_deadline() {
    let timeouts = Http::new().agent.config().timeouts();
    assert_eq!(timeouts.connect, Some(Duration::from_secs(30)));
    assert_eq!(timeouts.recv_response, Some(Duration::from_secs(MAST_RESPONSE_HEADER_SECONDS)));
    assert_eq!(timeouts.global, None);
    assert_eq!(timeouts.recv_body, None);

    let http = Http { agent: ureq::Agent::config_builder().timeout_recv_response(Some(Duration::from_millis(100))).build().new_agent() };
    let (url, server) = serve_parts(vec![vec![
        (Duration::ZERO, format!("HTTP/1.1 206 OK\r\nConnection: close\r\nContent-Length: 3\r\n{}\r\n", range_headers())),
        (Duration::from_millis(400), "abc".into()),
    ]]);
    let response = http.fetch_url(&url, Some("bytes=0-2"), Some(&identity()), 3, Instant::now() + Duration::from_secs(2)).unwrap();
    assert_eq!(response.bytes, b"abc");                                                         // the body may take longer than the header limit
    assert_eq!(server.join().unwrap().len(), 1);
}

#[test]
fn http_stalled_headers_and_bodies_still_obey_their_deadlines() {
    for delayed_headers in [true, false] {
        let http = Http { agent: ureq::Agent::config_builder().timeout_recv_response(Some(Duration::from_millis(100))).build().new_agent() };
        let headers = format!("HTTP/1.1 206 OK\r\nConnection: close\r\nContent-Length: 3\r\n{}\r\n", range_headers());
        let parts = if delayed_headers { vec![(Duration::from_millis(400), format!("{headers}abc"))] }
            else { vec![(Duration::ZERO, headers), (Duration::from_millis(400), "abc".into())] };
        let (url, server) = serve_parts(vec![parts]);
        let control = TransferControl::default();
        let duration = if delayed_headers { Duration::from_millis(750) } else { Duration::from_millis(150) };
        let error = http.fetch_url_cancellable(&url, Some("bytes=0-2"), Some(&identity()), 3, Instant::now() + duration, &control).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(control.received_bytes(), 0);
        assert_eq!(server.join().unwrap().len(), 1);                                             // insufficient remaining time cannot start a fresh retry budget
    }
}

//! Just enough HTTP to serve one page and a handful of JSON endpoints to a browser on this
//! machine — parsing in, bytes out, and nothing about what the endpoints mean.
//!
//! Hand-rolled rather than a dependency for the reason the rest of this binary is: it ships
//! as one file people install with `brew install`, and a web framework is a large amount of
//! surface for four routes that never leave the loopback interface.
//!
//! A leaf: standard library and its own input.

use std::io::{self, BufRead, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

/// A body larger than this is refused unread. Nothing this serves takes a large upload, and
/// a `Content-Length` header is a number a caller chose — allocating it before reading a
/// byte is how a one-line request turns into a gigabyte of memory.
pub const MAX_BODY: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// The path with any query string removed.
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// Header lookup is case-insensitive because the field name is
    /// (RFC 9110 §5.1), and browsers differ on how they spell `Origin` versus `origin`.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The last segment of the path, for the routes that end in an id.
    pub fn tail(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or("")
    }
}

/// Read one request. `Ok(None)` is a connection that closed without sending one, which is
/// ordinary — browsers open speculative connections and drop them.
pub fn read_request(reader: &mut impl BufRead) -> io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(None);
    };
    let method = method.to_string();
    let (path, query) = match target.split_once('?') {
        Some((path, rest)) => (path.to_string(), parse_query(rest)),
        None => (target.to_string(), Vec::new()),
    };

    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let trimmed = header.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }

    let length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = Vec::new();
    if length > MAX_BODY {
        // Say so rather than reading it: the caller gets 413 from the router, and this
        // process does not allocate a number a stranger picked.
        return Ok(Some(Request {
            method,
            path,
            query,
            headers,
            body: Vec::new(),
        }));
    }
    if length > 0 {
        body.resize(length, 0);
        reader.read_exact(&mut body)?;
    }
    Ok(Some(Request {
        method,
        path,
        query,
        headers,
        body,
    }))
}

fn parse_query(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

/// `%XX` and `+`. Invalid escapes are left as written rather than dropped — a token that
/// silently loses a character fails authentication for a reason nobody can see.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    None => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn respond(
    out: &mut impl Write,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    respond_with(out, status, content_type, &[], body)
}

/// `respond` with more header lines, for an answer that has to say how it may be used: the
/// caller names the headers, and none of them is checked here.
pub fn respond_with(
    out: &mut impl Write,
    status: u16,
    content_type: &str,
    extra: &[(&str, &str)],
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    // `no-store` because every one of these answers is about state that changes under the
    // reader, and a cached board is a board that lies. `nosniff` because a JSON body a
    // browser decides to treat as HTML is a script execution.
    write!(
        out,
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n",
        body.len()
    )?;
    for (name, value) in extra {
        write!(out, "{name}: {value}\r\n")?;
    }
    out.write_all(b"\r\n")?;
    out.write_all(body)?;
    out.flush()
}

pub fn json(out: &mut impl Write, status: u16, body: &str) -> io::Result<()> {
    respond(
        out,
        status,
        "application/json; charset=utf-8",
        body.as_bytes(),
    )
}

pub fn html(out: &mut impl Write, body: &str) -> io::Result<()> {
    respond(out, 200, "text/html; charset=utf-8", body.as_bytes())
}

/// An answer larger than this is refused: `get` reads to the end of the connection, and the one
/// it is written for answers with a document a page draws, not a download.
const MAX_RESPONSE: usize = 32 << 20;

/// One `GET` of `path` from the server on `port` of this machine, with the dashboard token in
/// `X-Adjutant-Token`: the status and the body as text.
///
/// This binary's own server is the only one it is written for, and `respond` is how that server
/// answers: it always sends `Content-Length` and `Connection: close`, and never
/// `Transfer-Encoding: chunked`. So the body is what follows the header, checked against the
/// length; a chunked answer is refused rather than parsed, since reading one as if it were plain
/// would hand back a document with the chunk sizes in it. `timeout` bounds the connect and each
/// read and write.
pub fn get(port: u16, path: &str, token: &str, timeout: Duration) -> Result<(u16, String), String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("cannot connect to 127.0.0.1:{port}: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|e| format!("cannot set a timeout on the connection: {e}"))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Adjutant-Token: {token}\r\nConnection: close\r\n\r\n"
    )
    .and_then(|()| stream.flush())
    .map_err(|e| format!("cannot send the request: {e}"))?;
    let mut raw = Vec::new();
    stream
        .take(MAX_RESPONSE as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| format!("cannot read the answer: {e}"))?;
    if raw.len() > MAX_RESPONSE {
        return Err(format!(
            "the answer is larger than {} MiB",
            MAX_RESPONSE >> 20
        ));
    }
    parse_response(&raw)
}

/// The status and body of an answer `respond` wrote.
fn parse_response(raw: &[u8]) -> Result<(u16, String), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("the answer has no header")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let body = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("the answer has no status")?;
    let header = |name: &str| {
        lines.clone().find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
        })
    };
    if header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        return Err("the answer is chunked, which this client does not read".to_string());
    }
    if let Some(length) = header("content-length").and_then(|v| v.parse::<usize>().ok())
        && body.len() != length
    {
        return Err(format!(
            "the answer is cut short: {} of {length} bytes",
            body.len()
        ));
    }
    Ok((status, String::from_utf8_lossy(body).into_owned()))
}

/// Compare two secrets without letting the time taken say how much of the guess was right.
pub fn secret_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Option<Request> {
        let mut reader = io::BufReader::new(raw.as_bytes());
        read_request(&mut reader).unwrap()
    }

    #[test]
    fn a_get_is_split_into_path_and_query() {
        let req = parse("GET /api/state?token=abc&repo=x HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("a request");
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/api/state");
        assert_eq!(req.param("token"), Some("abc"));
        assert_eq!(req.param("repo"), Some("x"));
        assert_eq!(req.param("nope"), None);
    }

    #[test]
    fn a_body_is_read_to_its_declared_length() {
        let req = parse("POST /api/tasks HTTP/1.1\r\nContent-Length: 7\r\n\r\n{\"a\":1}")
            .expect("a request");
        assert_eq!(req.body, b"{\"a\":1}");
    }

    /// The header is a number the caller picked. Believing it far enough to allocate is the
    /// bug; the router turns this empty body into a 413.
    #[test]
    fn an_oversized_body_is_refused_unread() {
        let raw = format!(
            "POST /api/tasks HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        let req = parse(&raw).expect("a request");
        assert!(req.body.is_empty());
    }

    #[test]
    fn headers_are_found_however_they_are_spelled() {
        let req = parse("GET / HTTP/1.1\r\nOrigin: http://127.0.0.1:4577\r\n\r\n").unwrap();
        assert_eq!(req.header("origin"), Some("http://127.0.0.1:4577"));
        assert_eq!(req.header("ORIGIN"), Some("http://127.0.0.1:4577"));
    }

    #[test]
    fn percent_escapes_and_plus_signs_come_back_as_characters() {
        assert_eq!(percent_decode("a%2Fb+c"), "a/b c");
        assert_eq!(percent_decode("%E6%97%A5%E6%9C%AC"), "日本");
    }

    /// A truncated escape is kept as written rather than dropped: a token that quietly
    /// loses a character fails for a reason the person cannot see.
    #[test]
    fn a_broken_escape_is_left_alone() {
        assert_eq!(percent_decode("a%2"), "a%2");
    }

    #[test]
    fn the_last_path_segment_is_the_id() {
        let req = parse("POST /api/tasks/20260922T041233Z-x/hand HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(req.tail(), "hand");
        assert_eq!(req.path, "/api/tasks/20260922T041233Z-x/hand");
    }

    #[test]
    fn a_connection_that_says_nothing_is_not_an_error() {
        assert_eq!(parse(""), None);
    }

    #[test]
    fn secrets_match_only_when_they_are_the_same() {
        assert!(secret_eq("abc", "abc"));
        assert!(!secret_eq("abc", "abd"));
        assert!(!secret_eq("abc", "ab"));
    }

    #[test]
    fn a_response_carries_its_length_and_the_guards() {
        let mut out = Vec::new();
        json(&mut out, 200, "{\"ok\":true}").unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(text.contains("Content-Length: 11"), "{text}");
        assert!(text.contains("X-Content-Type-Options: nosniff"), "{text}");
        assert!(text.ends_with("{\"ok\":true}"), "{text}");
    }

    /// The server answers through `respond`; so does this test, which is what keeps `get` honest
    /// about the framing the real one writes.
    fn serve_once(status: u16, body: &'static str) -> (u16, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = io::BufReader::new(stream.try_clone().unwrap());
            let request = read_request(&mut reader).unwrap().unwrap();
            let mut stream = stream;
            json(&mut stream, status, body).unwrap();
            format!(
                "{} {} {:?}",
                request.method,
                request.path,
                request.header("x-adjutant-token")
            )
        });
        (port, handle)
    }

    #[test]
    fn a_get_sends_the_token_and_reads_what_respond_wrote() {
        let (port, server) = serve_once(200, "{\"ok\":true}");
        let got = get(port, "/api/work", "tok", Duration::from_secs(5)).unwrap();
        assert_eq!(got, (200, "{\"ok\":true}".to_string()));
        assert_eq!(server.join().unwrap(), "GET /api/work Some(\"tok\")");
    }

    #[test]
    fn a_get_gives_the_status_of_a_refusal_with_its_body() {
        let (port, _server) = serve_once(403, "{\"error\":\"no\"}");
        let got = get(port, "/api/work", "x", Duration::from_secs(5)).unwrap();
        assert_eq!(got, (403, "{\"error\":\"no\"}".to_string()));
    }

    #[test]
    fn a_get_with_nothing_listening_says_it_cannot_connect() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let err = get(port, "/", "x", Duration::from_secs(2)).unwrap_err();
        assert!(err.starts_with("cannot connect"), "{err}");
    }

    #[test]
    fn an_answer_cut_short_or_chunked_is_not_read_as_a_document() {
        let short = b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n{\"a\"";
        assert!(parse_response(short).unwrap_err().contains("cut short"));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n";
        assert!(parse_response(chunked).unwrap_err().contains("chunked"));
        assert!(parse_response(b"nonsense").is_err());
    }
}

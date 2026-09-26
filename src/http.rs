//! Just enough HTTP to serve one page and a handful of JSON endpoints to a browser on this
//! machine — parsing in, bytes out, and nothing about what the endpoints mean.
//!
//! Hand-rolled rather than a dependency for the reason the rest of this binary is: it ships
//! as one file people install with `brew install`, and a web framework is a large amount of
//! surface for four routes that never leave the loopback interface.
//!
//! A leaf: standard library and its own input.

use std::io::{self, BufRead, Write};

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
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
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
         Connection: close\r\n\r\n",
        body.len()
    )?;
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
}

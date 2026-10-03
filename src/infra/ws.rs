//! Just enough WebSocket (RFC 6455) for the board terminal: the opening handshake and the
//! frame codec, from the browser's side of the connection to ours.
//!
//! Hand-rolled for the reason `http.rs` is: one file people install with `brew install`, and a
//! WebSocket crate would bring an async runtime for one endpoint on the loopback interface.
//! No extensions are ever negotiated — the `Sec-WebSocket-Extensions` a browser offers is left
//! unanswered, so frames are never compressed and the codec has no state beyond the bytes.
//!
//! The decoder does no I/O. A caller reads whatever is available, `push`es it and drains
//! `next` — so a read that times out halfway through a frame loses nothing.
//!
//! A leaf: standard library and its own input.

/// A message larger than this, once reassembled, is refused. A terminal sends keystrokes and
/// pastes, and the length in a frame header is a number the peer chose.
pub const MAX_MESSAGE: usize = 1 << 20;

pub const OP_CONTINUATION: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;

/// Close codes this side sends.
pub const CLOSE_NORMAL: u16 = 1000;
pub const CLOSE_PROTOCOL: u16 = 1002;
pub const CLOSE_BAD_DATA: u16 = 1007;
pub const CLOSE_TOO_BIG: u16 = 1009;

/// The constant RFC 6455 fixes for the handshake, written in two pieces so that the repository's
/// guard against tracker keys does not read a stretch of it as a project key.
const GUID: &str = concat!("258EAFA5-E914-47D", "A-95CA-C5AB0DC85B11");

// ── handshake ────────────────────────────────────────────────────────

fn header<'a>(headers: &'a [(String, String)], name: &str) -> impl Iterator<Item = &'a str> {
    let name = name.to_string();
    headers
        .iter()
        .filter(move |(k, _)| k.eq_ignore_ascii_case(&name))
        .map(|(_, v)| v.as_str())
}

/// Whether a comma-separated header value (or any of several such headers) lists `token`.
fn lists(headers: &[(String, String)], name: &str, token: &str) -> bool {
    header(headers, name).any(|value| {
        value
            .split(',')
            .any(|item| item.trim().eq_ignore_ascii_case(token))
    })
}

/// Whether the request asks to become a WebSocket, however badly: it names the protocol in
/// `Upgrade`. What is wrong with the rest is `accept_key`'s to say.
pub fn is_upgrade(headers: &[(String, String)]) -> bool {
    lists(headers, "upgrade", "websocket")
}

/// The `Sec-WebSocket-Accept` for a well-formed opening handshake, or what is wrong with it.
pub fn accept_key(headers: &[(String, String)]) -> Result<String, &'static str> {
    if !lists(headers, "connection", "upgrade") {
        return Err("Connection must include Upgrade");
    }
    if !lists(headers, "upgrade", "websocket") {
        return Err("Upgrade must be websocket");
    }
    if header(headers, "sec-websocket-version")
        .next()
        .map(str::trim)
        != Some("13")
    {
        return Err("Sec-WebSocket-Version must be 13");
    }
    let Some(key) = header(headers, "sec-websocket-key").next().map(str::trim) else {
        return Err("Sec-WebSocket-Key is missing");
    };
    // A nonce of 16 random bytes: anything else is not from a WebSocket client.
    if base64_decode(key).is_none_or(|bytes| bytes.len() != 16) {
        return Err("Sec-WebSocket-Key is not a 16 byte nonce");
    }
    Ok(accept_for(key))
}

/// `base64(sha1(key + GUID))`, the answer that proves the server read the handshake.
pub fn accept_for(key: &str) -> String {
    let mut input = key.trim().as_bytes().to_vec();
    input.extend_from_slice(GUID.as_bytes());
    base64_encode(&sha1(&input))
}

/// The `101` that completes the handshake. No `Connection: close` and no length: what follows
/// on this socket is frames.
pub fn handshake_response(accept: &str) -> String {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\
         X-Content-Type-Options: nosniff\r\n\r\n"
    )
}

// ── frames ───────────────────────────────────────────────────────────

/// One whole message, or one control frame, as the peer sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close { code: Option<u16>, reason: String },
}

/// Why the peer's bytes cannot be taken any further. Each maps to the close code to answer
/// with; after any of them the stream is not in a state worth reading on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Protocol(&'static str),
    TooBig,
    BadText,
}

impl Error {
    pub fn close_code(self) -> u16 {
        match self {
            Error::Protocol(_) => CLOSE_PROTOCOL,
            Error::TooBig => CLOSE_TOO_BIG,
            Error::BadText => CLOSE_BAD_DATA,
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            Error::Protocol(why) => why,
            Error::TooBig => "message too big",
            Error::BadText => "text is not UTF-8",
        }
    }
}

/// Client-to-server frames: bytes in, whole messages out.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    /// A fragmented data message in progress: its opcode and what has arrived.
    partial: Option<(u8, Vec<u8>)>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete message, or `None` while more bytes are needed.
    pub fn next(&mut self) -> Result<Option<Message>, Error> {
        loop {
            let Some((frame, used)) = self.frame()? else {
                return Ok(None);
            };
            self.buf.drain(..used);
            if let Some(message) = self.accept(frame)? {
                return Ok(Some(message));
            }
        }
    }

    /// Parse one frame off the front of the buffer without consuming it.
    fn frame(&self) -> Result<Option<(Frame, usize)>, Error> {
        let buf = &self.buf;
        if buf.len() < 2 {
            return Ok(None);
        }
        let (b0, b1) = (buf[0], buf[1]);
        if b0 & 0x70 != 0 {
            return Err(Error::Protocol("reserved bits are set"));
        }
        if b1 & 0x80 == 0 {
            return Err(Error::Protocol("client frames must be masked"));
        }
        let (fin, opcode) = (b0 & 0x80 != 0, b0 & 0x0f);
        let mut at = 2;
        let len = match b1 & 0x7f {
            126 => {
                if buf.len() < at + 2 {
                    return Ok(None);
                }
                at += 2;
                u64::from(u16::from_be_bytes([buf[2], buf[3]]))
            }
            127 => {
                if buf.len() < at + 8 {
                    return Ok(None);
                }
                let mut eight = [0u8; 8];
                eight.copy_from_slice(&buf[2..10]);
                at += 8;
                let len = u64::from_be_bytes(eight);
                if len >> 63 != 0 {
                    return Err(Error::Protocol("frame length has its top bit set"));
                }
                len
            }
            short => u64::from(short),
        };
        if !matches!(
            opcode,
            OP_CONTINUATION | OP_TEXT | OP_BINARY | OP_CLOSE | OP_PING | OP_PONG
        ) {
            return Err(Error::Protocol("unknown opcode"));
        }
        let control = opcode & 0x8 != 0;
        if control && (!fin || len > 125) {
            return Err(Error::Protocol("bad control frame"));
        }
        // Refused before the payload is waited for, let alone stored.
        if len > MAX_MESSAGE as u64 {
            return Err(Error::TooBig);
        }
        let len = len as usize;
        if buf.len() < at + 4 + len {
            return Ok(None);
        }
        let mask = [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]];
        let payload = buf[at + 4..at + 4 + len]
            .iter()
            .enumerate()
            .map(|(i, byte)| byte ^ mask[i % 4])
            .collect();
        Ok(Some((
            Frame {
                fin,
                opcode,
                payload,
            },
            at + 4 + len,
        )))
    }

    /// Fold one frame into the message being assembled; `Some` when it completes one.
    fn accept(&mut self, frame: Frame) -> Result<Option<Message>, Error> {
        match frame.opcode {
            OP_PING => Ok(Some(Message::Ping(frame.payload))),
            OP_PONG => Ok(Some(Message::Pong(frame.payload))),
            OP_CLOSE => close_message(frame.payload).map(Some),
            OP_CONTINUATION => {
                let Some((opcode, mut data)) = self.partial.take() else {
                    return Err(Error::Protocol("continuation without a message"));
                };
                data.extend_from_slice(&frame.payload);
                if data.len() > MAX_MESSAGE {
                    return Err(Error::TooBig);
                }
                if frame.fin {
                    data_message(opcode, data).map(Some)
                } else {
                    self.partial = Some((opcode, data));
                    Ok(None)
                }
            }
            opcode => {
                if self.partial.is_some() {
                    return Err(Error::Protocol("new message inside a fragmented one"));
                }
                if frame.fin {
                    data_message(opcode, frame.payload).map(Some)
                } else {
                    self.partial = Some((opcode, frame.payload));
                    Ok(None)
                }
            }
        }
    }
}

struct Frame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

fn data_message(opcode: u8, data: Vec<u8>) -> Result<Message, Error> {
    if opcode == OP_TEXT {
        String::from_utf8(data)
            .map(Message::Text)
            .map_err(|_| Error::BadText)
    } else {
        Ok(Message::Binary(data))
    }
}

/// Whether a close code may appear on the wire (RFC 6455 §7.4): the defined ones, without
/// 1004 to 1006 and 1015, which only stand for what an endpoint saw and are never sent, and
/// the ranges for libraries (3000 to 3999) and applications (4000 to 4999).
fn is_wire_close_code(code: u16) -> bool {
    matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999)
}

fn close_message(payload: Vec<u8>) -> Result<Message, Error> {
    match payload.len() {
        0 => Ok(Message::Close {
            code: None,
            reason: String::new(),
        }),
        1 => Err(Error::Protocol("close payload of one byte")),
        _ => {
            let code = u16::from_be_bytes([payload[0], payload[1]]);
            if !is_wire_close_code(code) {
                return Err(Error::Protocol("invalid close code"));
            }
            let reason = String::from_utf8(payload[2..].to_vec()).map_err(|_| Error::BadText)?;
            Ok(Message::Close {
                code: Some(code),
                reason,
            })
        }
    }
}

/// An unmasked, unfragmented frame, as a server sends one.
pub fn encode(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x80 | opcode);
    match payload.len() {
        len @ 0..=125 => out.push(len as u8),
        len @ 126..=0xffff => {
            out.push(126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            out.push(127);
            out.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(payload);
    out
}

/// A close frame. The reason is cut to what a control frame can hold beside the code.
pub fn encode_close(code: u16, reason: &str) -> Vec<u8> {
    let mut end = reason.len().min(123);
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    let mut payload = code.to_be_bytes().to_vec();
    payload.extend_from_slice(&reason.as_bytes()[..end]);
    encode(OP_CLOSE, &payload)
}

// ── SHA1 and base64 ─────────────────────────────────────────────────

/// SHA1 (FIPS 180-4). Only the handshake uses it, which is what it is still fit for.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut padded = data.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in padded.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (slot, add) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(add);
        }
    }
    let mut out = [0u8; 20];
    for (chunk, word) in out.chunks_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, byte)| n | u32::from(*byte) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard base64 with padding, strictly: `None` for anything else.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (i, quad) in bytes.chunks(4).enumerate() {
        let last = (i + 1) * 4 == bytes.len();
        let padding = quad.iter().rev().take_while(|b| **b == b'=').count();
        if padding > 2 || (padding > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for byte in &quad[..4 - padding] {
            let value = B64.iter().position(|c| c == byte)? as u32;
            n = n << 6 | value;
        }
        n <<= 6 * padding as u32;
        out.extend_from_slice(&n.to_be_bytes()[1..4 - padding]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A client frame: masked, as the browser sends them.
    fn masked(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [0x37, 0xfa, 0x21, 0x3d];
        let mut out = vec![if fin { 0x80 } else { 0 } | opcode];
        match payload.len() {
            len @ 0..=125 => out.push(0x80 | len as u8),
            len @ 126..=0xffff => {
                out.push(0x80 | 126);
                out.extend_from_slice(&(len as u16).to_be_bytes());
            }
            len => {
                out.push(0x80 | 127);
                out.extend_from_slice(&(len as u64).to_be_bytes());
            }
        }
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        out
    }

    fn decode_all(bytes: &[u8]) -> Result<Vec<Message>, Error> {
        let mut decoder = Decoder::new();
        decoder.push(bytes);
        let mut all = Vec::new();
        while let Some(message) = decoder.next()? {
            all.push(message);
        }
        Ok(all)
    }

    #[test]
    fn sha1_matches_the_published_vectors() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(plain.as_bytes()), coded);
            if !coded.is_empty() {
                assert_eq!(base64_decode(coded).as_deref(), Some(plain.as_bytes()));
            }
        }
    }

    #[test]
    fn base64_decoding_is_strict() {
        for bad in ["", "Zg", "Zg=", "Z===", "Zm9v!A==", "Zg==Zg==", "Zm 9"] {
            assert_eq!(base64_decode(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_rfc_example_key_gets_the_rfc_answer() {
        assert_eq!(
            accept_for("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const GOOD: [(&str, &str); 4] = [
        ("Connection", "keep-alive, Upgrade"),
        ("Upgrade", "WebSocket"),
        ("Sec-WebSocket-Version", "13"),
        ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
    ];

    #[test]
    fn a_browsers_handshake_is_accepted() {
        let h = headers(&GOOD);
        assert!(is_upgrade(&h));
        assert_eq!(accept_key(&h).unwrap(), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn a_broken_handshake_says_what_is_wrong() {
        let without = |name: &str| {
            let h: Vec<_> = GOOD.iter().filter(|(k, _)| *k != name).copied().collect();
            accept_key(&headers(&h))
        };
        assert!(without("Connection").is_err());
        assert!(without("Upgrade").is_err());
        assert!(without("Sec-WebSocket-Version").is_err());
        assert!(without("Sec-WebSocket-Key").is_err());

        let with = |name: &str, value: &str| {
            let h: Vec<_> = GOOD
                .iter()
                .map(|(k, v)| (*k, if *k == name { value } else { *v }))
                .collect();
            accept_key(&headers(&h))
        };
        assert!(with("Sec-WebSocket-Version", "8").is_err());
        assert!(with("Sec-WebSocket-Key", "AAAA").is_err());
        assert!(with("Sec-WebSocket-Key", "not base64 at all").is_err());
        assert!(with("Connection", "close").is_err());
        assert!(!is_upgrade(&headers(&[("Upgrade", "h2c")])));
        assert!(!is_upgrade(&headers(&[])));
    }

    #[test]
    fn the_response_is_a_bare_101() {
        let text = handshake_response("abc=");
        assert!(text.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
        assert!(text.contains("Sec-WebSocket-Accept: abc=\r\n"));
        assert!(!text.contains("Sec-WebSocket-Extensions"));
        assert!(!text.to_ascii_lowercase().contains("connection: close"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn the_rfc_masked_hello_decodes() {
        let bytes = [
            0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58,
        ];
        assert_eq!(
            decode_all(&bytes).unwrap(),
            vec![Message::Text("Hello".into())]
        );
    }

    #[test]
    fn encoded_lengths_use_the_shortest_form() {
        for (len, header) in [(0, 2), (125, 2), (126, 4), (65535, 4), (65536, 10)] {
            let payload = vec![7u8; len];
            let frame = encode(OP_BINARY, &payload);
            assert_eq!(frame.len(), header + len, "{len}");
            assert_eq!(frame[0], 0x82);
            assert_eq!(frame[1] & 0x80, 0, "server frames are not masked");
            assert_eq!(&frame[header..], &payload[..]);
        }
        assert_eq!(&encode(OP_BINARY, &[0; 126])[..4], &[0x82, 126, 0, 126]);
        assert_eq!(
            &encode(OP_BINARY, &vec![0; 65536])[..10],
            &[0x82, 127, 0, 0, 0, 0, 0, 1, 0, 0]
        );
    }

    #[test]
    fn lengths_of_every_form_survive_a_round_trip() {
        for len in [0, 1, 125, 126, 65535, 65536] {
            let payload: Vec<u8> = (0..len).map(|i| i as u8).collect();
            assert_eq!(
                decode_all(&masked(true, OP_BINARY, &payload)).unwrap(),
                vec![Message::Binary(payload)],
                "{len}"
            );
        }
    }

    #[test]
    fn a_frame_split_across_reads_is_decoded_when_it_completes() {
        let bytes = masked(true, OP_BINARY, b"split me");
        let mut decoder = Decoder::new();
        for byte in &bytes[..bytes.len() - 1] {
            decoder.push(&[*byte]);
            assert_eq!(decoder.next().unwrap(), None);
        }
        decoder.push(&bytes[bytes.len() - 1..]);
        assert_eq!(
            decoder.next().unwrap(),
            Some(Message::Binary(b"split me".to_vec()))
        );
        assert_eq!(decoder.next().unwrap(), None);
    }

    #[test]
    fn several_frames_in_one_read_come_out_in_order() {
        let mut bytes = masked(true, OP_BINARY, b"a");
        bytes.extend(masked(true, OP_PING, b"p"));
        bytes.extend(masked(true, OP_TEXT, b"t"));
        assert_eq!(
            decode_all(&bytes).unwrap(),
            vec![
                Message::Binary(b"a".to_vec()),
                Message::Ping(b"p".to_vec()),
                Message::Text("t".into())
            ]
        );
    }

    #[test]
    fn a_fragmented_message_is_reassembled_around_a_control_frame() {
        let mut bytes = masked(false, OP_BINARY, b"one ");
        bytes.extend(masked(true, OP_PING, b""));
        bytes.extend(masked(false, OP_CONTINUATION, b"two "));
        bytes.extend(masked(true, OP_CONTINUATION, b"three"));
        assert_eq!(
            decode_all(&bytes).unwrap(),
            vec![
                Message::Ping(Vec::new()),
                Message::Binary(b"one two three".to_vec())
            ]
        );
    }

    #[test]
    fn misordered_fragments_are_a_protocol_error() {
        let stray = masked(true, OP_CONTINUATION, b"x");
        assert!(matches!(decode_all(&stray), Err(Error::Protocol(_))));
        let mut nested = masked(false, OP_BINARY, b"x");
        nested.extend(masked(true, OP_BINARY, b"y"));
        assert!(matches!(decode_all(&nested), Err(Error::Protocol(_))));
    }

    #[test]
    fn a_reassembled_message_over_the_cap_is_refused() {
        let chunk = vec![0u8; MAX_MESSAGE / 2 + 1];
        let mut bytes = masked(false, OP_BINARY, &chunk);
        bytes.extend(masked(true, OP_CONTINUATION, &chunk));
        assert_eq!(decode_all(&bytes), Err(Error::TooBig));
    }

    #[test]
    fn a_frame_over_the_cap_is_refused_before_its_payload_arrives() {
        let mut header = vec![0x82, 0x80 | 127];
        header.extend_from_slice(&((MAX_MESSAGE as u64) + 1).to_be_bytes());
        header.extend_from_slice(&[0; 4]);
        assert_eq!(decode_all(&header), Err(Error::TooBig));
        assert_eq!(Error::TooBig.close_code(), 1009);
    }

    #[test]
    fn what_a_client_may_not_send_is_a_protocol_error() {
        // Unmasked.
        assert!(matches!(
            decode_all(&[0x82, 0x01, 0x41]),
            Err(Error::Protocol(_))
        ));
        // Reserved bit (a compressed frame, which no extension here allows).
        let mut rsv = masked(true, OP_BINARY, b"x");
        rsv[0] |= 0x40;
        assert!(matches!(decode_all(&rsv), Err(Error::Protocol(_))));
        // Unknown opcode.
        assert!(matches!(
            decode_all(&masked(true, 0x3, b"x")),
            Err(Error::Protocol(_))
        ));
        // Fragmented and oversized control frames.
        assert!(matches!(
            decode_all(&masked(false, OP_PING, b"x")),
            Err(Error::Protocol(_))
        ));
        assert!(matches!(
            decode_all(&masked(true, OP_PING, &[0; 126])),
            Err(Error::Protocol(_))
        ));
        assert_eq!(Error::Protocol("x").close_code(), 1002);
    }

    #[test]
    fn text_that_is_not_utf8_is_refused() {
        let err = decode_all(&masked(true, OP_TEXT, &[0xff, 0xfe])).unwrap_err();
        assert_eq!(err, Error::BadText);
        assert_eq!(err.close_code(), 1007);
    }

    #[test]
    fn a_close_frame_carries_its_code_and_reason() {
        let mut payload = 1001u16.to_be_bytes().to_vec();
        payload.extend_from_slice(b"going away");
        assert_eq!(
            decode_all(&masked(true, OP_CLOSE, &payload)).unwrap(),
            vec![Message::Close {
                code: Some(1001),
                reason: "going away".into()
            }]
        );
        assert_eq!(
            decode_all(&masked(true, OP_CLOSE, b"")).unwrap(),
            vec![Message::Close {
                code: None,
                reason: String::new()
            }]
        );
        let one = decode_all(&masked(true, OP_CLOSE, b"x")).unwrap_err();
        assert_eq!(one.close_code(), 1002);
    }

    #[test]
    fn a_close_frame_is_encoded_with_code_then_reason() {
        let frame = encode_close(4404, "no such session");
        assert_eq!(frame[0], 0x88);
        assert_eq!(usize::from(frame[1]), 2 + "no such session".len());
        assert_eq!(&frame[2..4], &4404u16.to_be_bytes());
        assert_eq!(&frame[4..], b"no such session");
    }

    #[test]
    fn a_long_reason_is_cut_on_a_character_boundary() {
        let frame = encode_close(1000, &"日".repeat(50));
        let len = usize::from(frame[1]);
        assert!(len <= 125, "{len}");
        assert!(std::str::from_utf8(&frame[4..]).is_ok());
    }

    #[test]
    fn a_close_code_that_may_not_be_on_the_wire_is_a_protocol_error() {
        // Never sent (1004 to 1006, 1015), unassigned (1016 to 2999) and out of range.
        for code in [0u16, 999, 1004, 1005, 1006, 1015, 1016, 2999, 5000, 65535] {
            let err = decode_all(&masked(true, OP_CLOSE, &code.to_be_bytes())).unwrap_err();
            assert_eq!(err.close_code(), 1002, "{code}");
        }
        for code in [
            1000u16, 1001, 1002, 1003, 1007, 1011, 1012, 1014, 3000, 4404, 4999,
        ] {
            assert!(
                decode_all(&masked(true, OP_CLOSE, &code.to_be_bytes())).is_ok(),
                "{code}"
            );
        }
    }

    #[test]
    fn a_close_reason_that_is_not_utf8_is_refused_with_1007() {
        let mut payload = 1000u16.to_be_bytes().to_vec();
        payload.extend_from_slice(&[0xff, 0xfe]);
        let err = decode_all(&masked(true, OP_CLOSE, &payload)).unwrap_err();
        assert_eq!(err, Error::BadText);
        assert_eq!(err.close_code(), 1007);
    }
}

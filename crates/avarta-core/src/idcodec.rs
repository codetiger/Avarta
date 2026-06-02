//! Dependency-free bit-packing + base64url, the low-level half of the share-id
//! codec (see [`crate::params::encode_id`]).
//!
//! [`BitWriter`]/[`BitReader`] pack unsigned integer fields of arbitrary width
//! (≤ 32 bits) MSB-first into a byte buffer; [`base64url_encode`]/
//! [`base64url_decode`] convert that buffer to/from the URL-safe base64 alphabet
//! (`A–Z a–z 0–9 - _`) with no padding. Together they turn a list of quantised
//! parameter indices into a compact, URL-hash-safe string and back.

/// Packs unsigned bit-fields MSB-first into a growing byte buffer.
#[derive(Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    /// Bits already written into the final byte (0..8). 0 ⇒ byte-aligned, so the
    /// next write pushes a fresh byte first.
    bit_pos: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append the low `bits` bits of `value`, most-significant bit first.
    /// `bits` must be ≤ 32; higher bits of `value` are ignored.
    pub fn write(&mut self, value: u32, bits: u8) {
        for i in (0..bits).rev() {
            if self.bit_pos == 0 {
                self.bytes.push(0);
            }
            let bit = ((value >> i) & 1) as u8;
            if bit != 0 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= bit << (7 - self.bit_pos);
            }
            self.bit_pos = (self.bit_pos + 1) % 8;
        }
    }

    /// The packed bytes (the final byte is zero-padded to a byte boundary).
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Reads unsigned bit-fields MSB-first from a byte slice, the inverse of
/// [`BitWriter`].
pub struct BitReader<'a> {
    bytes: &'a [u8],
    /// Absolute bit offset from the start of `bytes`.
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit_pos: 0 }
    }

    /// Read the next `bits` bits (MSB-first) as an unsigned integer, or `None`
    /// if fewer than `bits` bits remain. `bits` must be ≤ 32.
    pub fn read(&mut self, bits: u8) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..bits {
            let byte = self.bytes.get(self.bit_pos / 8)?;
            let bit = (byte >> (7 - (self.bit_pos % 8))) & 1;
            v = (v << 1) | bit as u32;
            self.bit_pos += 1;
        }
        Some(v)
    }

    /// Bits consumed so far — `div_ceil(8)` is the number of payload bytes, which
    /// a decoder can compare against the input length to reject trailing junk.
    pub fn bits_read(&self) -> usize {
        self.bit_pos
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encode bytes as base64url (URL-safe alphabet, no `=` padding).
pub fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = ((chunk[0] as u32) << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(B64[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(B64[n as usize & 63] as char);
        }
    }
    out
}

/// Decode a base64url (no-padding) string. Returns `None` on any character
/// outside the alphabet or a malformed (length ≡ 1 mod 4) tail.
pub fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        })
    }
    let s = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    for chunk in s.chunks(4) {
        if chunk.len() == 1 {
            return None; // a lone trailing char can't encode any byte
        }
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() >= 3 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() >= 4 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_round_trip_across_byte_boundaries() {
        let fields = [(5u32, 3u8), (700, 10), (0, 1), (9999, 14), (1, 1), (63, 6)];
        let mut w = BitWriter::new();
        for &(v, b) in &fields {
            w.write(v, b);
        }
        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes);
        for &(v, b) in &fields {
            assert_eq!(r.read(b), Some(v));
        }
    }

    #[test]
    fn bitreader_reports_overrun() {
        let bytes = [0xABu8];
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read(8), Some(0xAB));
        assert_eq!(r.read(1), None);
    }

    #[test]
    fn base64url_round_trips_every_length() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let s = base64url_encode(&data);
            assert!(s
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
            assert!(!s.contains('='), "no padding");
            assert_eq!(base64url_decode(&s).as_deref(), Some(data.as_slice()));
        }
    }

    #[test]
    fn base64url_rejects_bad_input() {
        assert_eq!(base64url_decode("abc="), None); // '=' not in alphabet
        assert_eq!(base64url_decode("a"), None); // length ≡ 1 mod 4
        assert_eq!(base64url_decode("!!"), None);
    }
}

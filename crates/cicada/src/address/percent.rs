//! Percent-encoding of actor URI segments.
//!
//! Unreserved characters (`A-Z a-z 0-9 . _ ~ -`) are kept as they are; every other character is encoded as the UTF-8
//! bytes of the character, each written as `%XX` with upper-case hex digits. The encoded form is canonical: `decode`
//! followed by `encode` always yields the same string for the same segment value.

use std::fmt::Write;

use super::AddressError;

/// True when `c` may appear unencoded in a segment.
pub fn is_unreserved(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-')
}

/// Encodes a raw segment value into its canonical percent-encoded form.
pub fn encode(value: &str) -> String {
    if value.chars().all(is_unreserved) {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len() * 3);
    for b in value.bytes() {
        let c = b as char;
        if b.is_ascii() && is_unreserved(c) {
            out.push(c);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Decodes a percent-encoded segment.
///
/// Fails when the segment contains a character that is neither unreserved nor part of a well-formed `%XX` escape, or
/// when the escapes do not form valid UTF-8.
pub fn decode(encoded: &str) -> Result<String, AddressError> {
    if encoded.chars().all(is_unreserved) {
        return Ok(encoded.to_owned());
    }
    let mut bytes = Vec::with_capacity(encoded.len());
    let raw = encoded.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        if c == b'%' {
            if i + 2 >= raw.len() {
                return Err(AddressError::new(format!("Truncated percent escape in segment '{encoded}'")));
            }
            let hi = (raw[i + 1] as char).to_digit(16);
            let lo = (raw[i + 2] as char).to_digit(16);
            match (hi, lo) {
                (Some(hi), Some(lo)) => bytes.push((hi * 16 + lo) as u8),
                _ => return Err(AddressError::new(format!("Invalid percent escape in segment '{encoded}'"))),
            }
            i += 3;
        } else if c.is_ascii() && is_unreserved(c as char) {
            bytes.push(c);
            i += 1;
        } else {
            let ch = encoded[i..].chars().next().unwrap_or('?');
            return Err(AddressError::new(format!(
                "Character '{ch}' must be percent-encoded in segment '{encoded}'"
            )));
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| AddressError::new(format!("Percent escapes in segment '{encoded}' are not valid UTF-8")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for value in ["acme", "a b", "ü", "a/b", "%", "~-._", "日本"] {
            assert_eq!(decode(&encode(value)).unwrap(), value);
        }
        assert_eq!(encode("a b"), "a%20b");
        assert_eq!(encode("ü"), "%C3%BC");
        assert_eq!(decode("%7e").unwrap(), "~");
        assert_eq!(encode(&decode("%7e").unwrap()), "~");
    }

    #[test]
    fn rejects_malformed() {
        assert!(decode("%").is_err());
        assert!(decode("%2").is_err());
        assert!(decode("%zz").is_err());
        assert!(decode("a b").is_err());
        assert!(decode("%C3").is_err());
    }
}

//! Minimal, canonical (RFC 8949 §4.2 deterministic) CBOR codec for the broker
//! wire protocol.
//!
//! linux-os-control-production **Task 1.5**, design §12 (OSC-001, OSC-007).
//!
//! # Why a hand-rolled codec
//!
//! The broker protocol requires *canonical* length-prefixed CBOR: the exact same
//! logical request must always encode to the exact same bytes, and a decoder
//! must **reject** every non-canonical, ambiguous, or malformed frame *before*
//! dispatch (design §12: "Unknown versions, operation tags, required fields,
//! duplicate map keys, non-canonical encodings, and trailing frames fail before
//! dispatch"). No CBOR crate is present in the workspace, and pulling one in
//! would not give us strict control over *canonical rejection* on the decode
//! path. This module implements exactly the subset the protocol needs, with a
//! strict decoder that is the security boundary.
//!
//! # Canonical rules enforced
//!
//! * **Definite lengths only.** Indefinite-length byte/text/array/map encodings
//!   (additional-info `31`) are rejected.
//! * **Minimal integer encoding.** A value that could fit in a shorter head is
//!   rejected (e.g. `0x18 0x00` for `0` is non-canonical).
//! * **Sorted, unique map keys.** Map keys must appear in strictly increasing
//!   bytewise-lexicographic order of their *encoded* form; a duplicate (equal)
//!   or out-of-order key is rejected.
//! * **No trailing data.** [`decode_canonical`] requires the input to be fully
//!   consumed by exactly one top-level value.
//!
//! The value model is intentionally small: unsigned/negative integers, byte and
//! text strings, arrays, maps, booleans, and null. That is all the protocol
//! uses; there is deliberately no float, tag, or indefinite support to abuse.

use std::fmt;

/// A decoded canonical CBOR value (the small subset the protocol uses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CborValue {
    /// Major type 0 — an unsigned integer.
    Uint(u64),
    /// Major type 1 — a negative integer, stored as `n` where the value is
    /// `-1 - n` (so `n = 0` is `-1`).
    Nint(u64),
    /// Major type 2 — a byte string.
    Bytes(Vec<u8>),
    /// Major type 3 — a UTF-8 text string.
    Text(String),
    /// Major type 4 — a definite-length array.
    Array(Vec<CborValue>),
    /// Major type 5 — a definite-length map with canonically ordered keys.
    Map(Vec<(CborValue, CborValue)>),
    /// Major type 7 — the boolean simple values.
    Bool(bool),
    /// Major type 7 — the null simple value.
    Null,
}

/// The maximum encoded frame size (design §12: 64 KiB request/response max).
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// The fixed length-prefix width in bytes (big-endian `u32`).
pub const LENGTH_PREFIX_BYTES: usize = 4;

/// A strict CBOR / framing decode error. Its presence proves a frame was
/// rejected *before* any interpretation of the request's authority — i.e. no
/// dispatch occurred (design §12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CborError {
    /// The declared or actual frame exceeds [`MAX_FRAME_BYTES`].
    FrameTooLarge {
        /// The offending size in bytes.
        size: usize,
    },
    /// The length prefix is missing, malformed, or does not match the payload.
    BadFrameLength,
    /// Extra bytes remained after exactly one top-level value / frame.
    TrailingData,
    /// The input ended before a complete value was read.
    UnexpectedEof,
    /// A reserved / unsupported additional-info value was used (28–30).
    ReservedAdditionalInfo,
    /// An indefinite-length item was used (additional info 31); forbidden.
    IndefiniteLength,
    /// An integer used a longer head than its value requires.
    NonMinimalInt,
    /// A map's keys were not in strictly increasing canonical order.
    NonCanonicalMapOrder,
    /// A map contained a duplicate key.
    DuplicateMapKey,
    /// A major type or simple value outside the supported subset was seen.
    UnsupportedItem,
    /// A text string was not valid UTF-8.
    InvalidUtf8,
    /// A length field would overflow addressable memory / exceed the frame cap.
    LengthOverflow,
}

impl fmt::Display for CborError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            CborError::FrameTooLarge { size } => {
                return write!(f, "cbor frame too large: {size} bytes")
            }
            CborError::BadFrameLength => "malformed frame length prefix",
            CborError::TrailingData => "trailing data after a complete cbor value",
            CborError::UnexpectedEof => "unexpected end of cbor input",
            CborError::ReservedAdditionalInfo => "reserved additional-info value",
            CborError::IndefiniteLength => "indefinite-length items are forbidden",
            CborError::NonMinimalInt => "non-minimal integer encoding",
            CborError::NonCanonicalMapOrder => "map keys are not in canonical order",
            CborError::DuplicateMapKey => "duplicate map key",
            CborError::UnsupportedItem => "unsupported cbor item",
            CborError::InvalidUtf8 => "invalid utf-8 text string",
            CborError::LengthOverflow => "cbor length overflow",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for CborError {}

// ─────────────────────────────────────────────────────────────────────────────
// Encoding (always canonical)
// ─────────────────────────────────────────────────────────────────────────────

impl CborValue {
    /// Encode this value to canonical CBOR bytes (no framing).
    #[must_use]
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            CborValue::Uint(v) => encode_head(out, 0, *v),
            CborValue::Nint(n) => encode_head(out, 1, *n),
            CborValue::Bytes(b) => {
                encode_head(out, 2, b.len() as u64);
                out.extend_from_slice(b);
            }
            CborValue::Text(s) => {
                encode_head(out, 3, s.len() as u64);
                out.extend_from_slice(s.as_bytes());
            }
            CborValue::Array(items) => {
                encode_head(out, 4, items.len() as u64);
                for item in items {
                    item.encode_into(out);
                }
            }
            CborValue::Map(entries) => {
                encode_head(out, 5, entries.len() as u64);
                // Canonical: emit keys in strictly increasing bytewise-lex order
                // of their encoded form.
                let mut sorted: Vec<&(CborValue, CborValue)> = entries.iter().collect();
                sorted.sort_by_cached_key(|(k, _)| k.to_canonical_bytes());
                for (k, v) in sorted {
                    k.encode_into(out);
                    v.encode_into(out);
                }
            }
            CborValue::Bool(false) => out.push(0xf4),
            CborValue::Bool(true) => out.push(0xf5),
            CborValue::Null => out.push(0xf6),
        }
    }
}

/// Emit a canonical (minimal-length) head for `major` (0..=7) carrying `value`.
fn encode_head(out: &mut Vec<u8>, major: u8, value: u64) {
    let mt = major << 5;
    if value < 24 {
        out.push(mt | (value as u8));
    } else if value <= u64::from(u8::MAX) {
        out.push(mt | 24);
        out.push(value as u8);
    } else if value <= u64::from(u16::MAX) {
        out.push(mt | 25);
        out.extend_from_slice(&(value as u16).to_be_bytes());
    } else if value <= u64::from(u32::MAX) {
        out.push(mt | 26);
        out.extend_from_slice(&(value as u32).to_be_bytes());
    } else {
        out.push(mt | 27);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Framing
// ─────────────────────────────────────────────────────────────────────────────

/// Wrap canonical CBOR `payload` in a big-endian `u32` length prefix, enforcing
/// the 64 KiB maximum (design §12).
pub fn frame(payload: &[u8]) -> Result<Vec<u8>, CborError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(CborError::FrameTooLarge {
            size: payload.len(),
        });
    }
    let mut out = Vec::with_capacity(LENGTH_PREFIX_BYTES + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Strip the length prefix, verifying it matches the payload exactly and that no
/// trailing frame follows (design §12: "one request per authenticated local
/// connection", trailing frames rejected).
pub fn unframe(frame: &[u8]) -> Result<&[u8], CborError> {
    if frame.len() < LENGTH_PREFIX_BYTES {
        return Err(CborError::BadFrameLength);
    }
    let declared = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if declared > MAX_FRAME_BYTES {
        return Err(CborError::FrameTooLarge { size: declared });
    }
    let body = &frame[LENGTH_PREFIX_BYTES..];
    if body.len() < declared {
        return Err(CborError::BadFrameLength);
    }
    if body.len() > declared {
        // Bytes beyond the single declared frame are a trailing frame.
        return Err(CborError::TrailingData);
    }
    Ok(body)
}

// ─────────────────────────────────────────────────────────────────────────────
// Strict canonical decoding
// ─────────────────────────────────────────────────────────────────────────────

/// Decode exactly one canonical CBOR value from `bytes`, requiring the whole
/// slice to be consumed. This is the protocol's decode security boundary: any
/// non-canonical or malformed encoding is rejected here.
pub fn decode_canonical(bytes: &[u8]) -> Result<CborValue, CborError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(CborError::FrameTooLarge { size: bytes.len() });
    }
    let mut dec = Decoder { buf: bytes, pos: 0 };
    let value = dec.read_value()?;
    if dec.pos != bytes.len() {
        return Err(CborError::TrailingData);
    }
    Ok(value)
}

/// Decode a framed canonical CBOR value: unframe then strict-decode.
pub fn decode_frame(frame_bytes: &[u8]) -> Result<CborValue, CborError> {
    let body = unframe(frame_bytes)?;
    decode_canonical(body)
}

struct Decoder<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Decoder<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], CborError> {
        let end = self.pos.checked_add(n).ok_or(CborError::LengthOverflow)?;
        if end > self.buf.len() {
            return Err(CborError::UnexpectedEof);
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, CborError> {
        Ok(self.take(1)?[0])
    }

    /// Read a canonical head, returning `(major, additional_info, argument)`.
    /// Enforces minimal-length integer encoding and rejects reserved / indefinite
    /// additional-info values.
    fn read_head(&mut self) -> Result<(u8, u64), CborError> {
        let initial = self.read_u8()?;
        let major = initial >> 5;
        let ai = initial & 0x1f;
        let argument = match ai {
            0..=23 => u64::from(ai),
            24 => {
                let b = self.read_u8()?;
                if b < 24 {
                    return Err(CborError::NonMinimalInt);
                }
                u64::from(b)
            }
            25 => {
                let bytes = self.take(2)?;
                let v = u64::from(u16::from_be_bytes([bytes[0], bytes[1]]));
                if v <= u64::from(u8::MAX) {
                    return Err(CborError::NonMinimalInt);
                }
                v
            }
            26 => {
                let bytes = self.take(4)?;
                let v = u64::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                if v <= u64::from(u16::MAX) {
                    return Err(CborError::NonMinimalInt);
                }
                v
            }
            27 => {
                let bytes = self.take(8)?;
                let mut arr = [0u8; 8];
                arr.copy_from_slice(bytes);
                let v = u64::from_be_bytes(arr);
                if v <= u64::from(u32::MAX) {
                    return Err(CborError::NonMinimalInt);
                }
                v
            }
            28..=30 => return Err(CborError::ReservedAdditionalInfo),
            31 => return Err(CborError::IndefiniteLength),
            _ => unreachable!("ai is masked to 5 bits"),
        };
        Ok((major, argument))
    }

    fn checked_len(&self, len: u64) -> Result<usize, CborError> {
        let len = usize::try_from(len).map_err(|_| CborError::LengthOverflow)?;
        if len > MAX_FRAME_BYTES {
            return Err(CborError::LengthOverflow);
        }
        Ok(len)
    }

    fn read_value(&mut self) -> Result<CborValue, CborError> {
        let initial = *self.buf.get(self.pos).ok_or(CborError::UnexpectedEof)?;
        let major = initial >> 5;
        match major {
            0 => {
                let (_, v) = self.read_head()?;
                Ok(CborValue::Uint(v))
            }
            1 => {
                let (_, v) = self.read_head()?;
                Ok(CborValue::Nint(v))
            }
            2 => {
                let (_, len) = self.read_head()?;
                let len = self.checked_len(len)?;
                Ok(CborValue::Bytes(self.take(len)?.to_vec()))
            }
            3 => {
                let (_, len) = self.read_head()?;
                let len = self.checked_len(len)?;
                let bytes = self.take(len)?.to_vec();
                let text = String::from_utf8(bytes).map_err(|_| CborError::InvalidUtf8)?;
                Ok(CborValue::Text(text))
            }
            4 => {
                let (_, len) = self.read_head()?;
                let len = self.checked_len(len)?;
                let mut items = Vec::with_capacity(len.min(64));
                for _ in 0..len {
                    items.push(self.read_value()?);
                }
                Ok(CborValue::Array(items))
            }
            5 => {
                let (_, len) = self.read_head()?;
                let len = self.checked_len(len)?;
                let mut entries: Vec<(CborValue, CborValue)> = Vec::with_capacity(len.min(64));
                let mut prev_key: Option<Vec<u8>> = None;
                for _ in 0..len {
                    let key = self.read_value()?;
                    let key_bytes = key.to_canonical_bytes();
                    if let Some(prev) = &prev_key {
                        match key_bytes.as_slice().cmp(prev.as_slice()) {
                            std::cmp::Ordering::Less => {
                                return Err(CborError::NonCanonicalMapOrder)
                            }
                            std::cmp::Ordering::Equal => return Err(CborError::DuplicateMapKey),
                            std::cmp::Ordering::Greater => {}
                        }
                    }
                    prev_key = Some(key_bytes);
                    let value = self.read_value()?;
                    entries.push((key, value));
                }
                Ok(CborValue::Map(entries))
            }
            7 => {
                let b = self.read_u8()?;
                match b {
                    0xf4 => Ok(CborValue::Bool(false)),
                    0xf5 => Ok(CborValue::Bool(true)),
                    0xf6 => Ok(CborValue::Null),
                    _ => Err(CborError::UnsupportedItem),
                }
            }
            6 => Err(CborError::UnsupportedItem), // tags unsupported
            _ => Err(CborError::UnsupportedItem),
        }
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn round(v: &CborValue) -> CborValue {
        let bytes = v.to_canonical_bytes();
        decode_canonical(&bytes).expect("round-trip decode")
    }

    #[test]
    fn integers_round_trip_minimally() {
        for v in [
            0u64,
            1,
            23,
            24,
            255,
            256,
            65_535,
            65_536,
            u64::from(u32::MAX) + 1,
        ] {
            assert_eq!(round(&CborValue::Uint(v)), CborValue::Uint(v));
        }
    }

    #[test]
    fn minimal_encoding_is_enforced_on_decode() {
        // 0 encoded with a 1-byte argument (0x18 0x00) is non-canonical.
        assert_eq!(
            decode_canonical(&[0x18, 0x00]),
            Err(CborError::NonMinimalInt)
        );
        // 24 canonically is 0x18 0x18.
        assert_eq!(decode_canonical(&[0x18, 0x18]), Ok(CborValue::Uint(24)));
    }

    #[test]
    fn indefinite_length_is_rejected() {
        // 0x5f = byte string, indefinite length.
        assert_eq!(decode_canonical(&[0x5f]), Err(CborError::IndefiniteLength));
    }

    #[test]
    fn text_and_bytes_round_trip() {
        assert_eq!(
            round(&CborValue::Text("hello".into())),
            CborValue::Text("hello".into())
        );
        assert_eq!(
            round(&CborValue::Bytes(vec![1, 2, 3])),
            CborValue::Bytes(vec![1, 2, 3])
        );
    }

    #[test]
    fn map_encodes_in_canonical_order_and_rejects_dupes() {
        let m = CborValue::Map(vec![
            (CborValue::Uint(2), CborValue::Uint(20)),
            (CborValue::Uint(0), CborValue::Uint(0)),
            (CborValue::Uint(1), CborValue::Uint(10)),
        ]);
        let bytes = m.to_canonical_bytes();
        // Keys must be emitted 0,1,2.
        let decoded = decode_canonical(&bytes).expect("decode");
        if let CborValue::Map(entries) = decoded {
            let keys: Vec<&CborValue> = entries.iter().map(|(k, _)| k).collect();
            assert_eq!(
                keys,
                vec![
                    &CborValue::Uint(0),
                    &CborValue::Uint(1),
                    &CborValue::Uint(2)
                ]
            );
        } else {
            panic!("expected map");
        }

        // A hand-built out-of-order map is rejected.
        let mut bad = Vec::new();
        bad.push(0xa2); // map(2)
        bad.extend_from_slice(&CborValue::Uint(2).to_canonical_bytes());
        bad.extend_from_slice(&CborValue::Uint(0).to_canonical_bytes());
        bad.extend_from_slice(&CborValue::Uint(1).to_canonical_bytes());
        bad.extend_from_slice(&CborValue::Uint(0).to_canonical_bytes());
        assert_eq!(decode_canonical(&bad), Err(CborError::NonCanonicalMapOrder));

        // A duplicate key is rejected.
        let mut dup = Vec::new();
        dup.push(0xa2); // map(2)
        dup.extend_from_slice(&CborValue::Uint(1).to_canonical_bytes());
        dup.extend_from_slice(&CborValue::Uint(0).to_canonical_bytes());
        dup.extend_from_slice(&CborValue::Uint(1).to_canonical_bytes());
        dup.extend_from_slice(&CborValue::Uint(0).to_canonical_bytes());
        assert_eq!(decode_canonical(&dup), Err(CborError::DuplicateMapKey));
    }

    #[test]
    fn trailing_data_is_rejected() {
        let mut bytes = CborValue::Uint(1).to_canonical_bytes();
        bytes.push(0x01); // extra
        assert_eq!(decode_canonical(&bytes), Err(CborError::TrailingData));
    }

    #[test]
    fn framing_round_trips_and_rejects_oversize_and_trailing() {
        let payload = CborValue::Text("x".into()).to_canonical_bytes();
        let framed = frame(&payload).expect("frame");
        assert_eq!(unframe(&framed).expect("unframe"), payload.as_slice());

        // Oversize declared length.
        let mut big = Vec::new();
        big.extend_from_slice(&((MAX_FRAME_BYTES as u32) + 1).to_be_bytes());
        assert!(matches!(
            unframe(&big),
            Err(CborError::FrameTooLarge { .. })
        ));

        // Trailing frame after the declared payload.
        let mut trailing = framed.clone();
        trailing.push(0xff);
        assert_eq!(unframe(&trailing), Err(CborError::TrailingData));
    }

    #[test]
    fn reserved_additional_info_is_rejected() {
        assert_eq!(
            decode_canonical(&[0x1c]),
            Err(CborError::ReservedAdditionalInfo)
        );
    }
}

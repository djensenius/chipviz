//! Data-free validation of complete offline catalog-v1 recordings.
//!
//! This is not a renderer availability or safety check. Like the host
//! validator, it rejects Starfield after structural validation because its
//! required external inputs and implementation cannot be verified here.
//! Other families are structurally valid only, not necessarily available.
//! The caller supplies an already acquired byte slice; no paths or devices are
//! opened. The caller must bound acquisition separately and must not mutate
//! the slice's backing storage during validation.

use std::fmt;

const HEADER_SIZE: usize = 20;
const RECORD_SIZE: usize = 145;
const MAX_RECORDS: u64 = 1_000_000;
const MAX_VARIANT: [u8; 14] = [0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0];
const MIN_EPOCH: i64 = 946_684_800_000;
const MAX_EPOCH: i64 = 4_102_444_799_999;

/// Failure leaves no snapshot or records available to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    Invalid,
    StarfieldUnavailable,
    AllocationFailed,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ValidationError {}

/// A private, owned copy of the exact validated file, including its header.
pub struct Snapshot {
    bytes: Vec<u8>,
}

/// An immutable view of one complete envelope in a validated snapshot.
pub struct Record<'a> {
    envelope: &'a [u8],
}

impl Snapshot {
    /// Validate the complete extended-mode file before exposing any records.
    ///
    /// At most 145,000,020 bytes are copied into the snapshot. The caller's
    /// input and any file-reading buffers are separate from that bound.
    pub fn validate(input: &[u8]) -> Result<Self, ValidationError> {
        if input.len() < HEADER_SIZE
            || &input[..8] != b"CVXE\x01\x01\0\0"
            || crc32(&input[..16]) != le_u32(&input[16..20])
        {
            return Err(ValidationError::Invalid);
        }
        let count = le_u64(&input[8..16]);
        if !(1..=MAX_RECORDS).contains(&count) {
            return Err(ValidationError::Invalid);
        }
        let size = usize::try_from(count)
            .ok()
            .and_then(|n| n.checked_mul(RECORD_SIZE))
            .and_then(|n| n.checked_add(HEADER_SIZE))
            .ok_or(ValidationError::Invalid)?;
        if input.len() != size {
            return Err(ValidationError::Invalid);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| ValidationError::AllocationFailed)?;
        bytes.extend_from_slice(input);

        let mut star_metadata: Option<&[u8]> = None;
        for (index, wire) in bytes[HEADER_SIZE..].chunks_exact(RECORD_SIZE).enumerate() {
            star_metadata = validate_record(wire, index as u64, star_metadata)?;
        }
        if star_metadata.is_some() {
            return Err(ValidationError::StarfieldUnavailable);
        }
        Ok(Self { bytes })
    }

    pub fn header(&self) -> &[u8] {
        &self.bytes[..HEADER_SIZE]
    }

    pub fn count(&self) -> usize {
        (self.bytes.len() - HEADER_SIZE) / RECORD_SIZE
    }

    pub fn record(&self, index: usize) -> Option<Record<'_>> {
        if index >= self.count() {
            return None;
        }
        let start = HEADER_SIZE + index * RECORD_SIZE;
        Some(Record {
            envelope: &self.bytes[start..start + RECORD_SIZE],
        })
    }
}

impl Record<'_> {
    pub fn envelope(&self) -> &[u8] {
        self.envelope
    }

    pub fn sequence(&self) -> u64 {
        le_u64(&self.envelope[8..16])
    }

    pub fn tick(&self) -> u64 {
        le_u64(&self.envelope[16..24])
    }

    pub fn family(&self) -> u8 {
        self.envelope[44]
    }

    pub fn variant(&self) -> u8 {
        self.envelope[6]
    }

    pub fn metadata(&self) -> &[u8] {
        &self.envelope[57..141]
    }
}

fn validate_record<'a>(
    wire: &'a [u8],
    expected: u64,
    star_metadata: Option<&'a [u8]>,
) -> Result<Option<&'a [u8]>, ValidationError> {
    let invalid = ValidationError::Invalid;
    if &wire[..6] != b"CVX1\x01\x01"
        || wire[7] != 0
        || crc32(&wire[..141]) != le_u32(&wire[141..145])
        || le_u64(&wire[8..16]) != expected
        || le_u64(&wire[16..24]) != expected
    {
        return Err(invalid);
    }
    let frame = &wire[24..57];
    if &frame[..2] != b"\xc7\0"
        || frame.iter().fold(0, |xor, byte| xor ^ byte) != 0
        || u16::from_le_bytes([frame[2], frame[3]]) != expected as u16
        || frame[22] & 0xe0 != 0
        || frame[23] > 4
        || (0..usize::from(frame[23])).any(|i| frame[24 + 2 * i] > 127)
    {
        return Err(invalid);
    }
    let family = usize::from(frame[20]);
    if family >= MAX_VARIANT.len() || wire[6] > MAX_VARIANT[family] {
        return Err(invalid);
    }
    let metadata = &wire[57..141];
    if family != 6 {
        if metadata.iter().any(|&byte| byte != 0) {
            return Err(invalid);
        }
        return Ok(star_metadata);
    }
    let epoch = i64::from_le_bytes(metadata[32..40].try_into().unwrap());
    let latitude = i32::from_le_bytes(metadata[40..44].try_into().unwrap());
    let longitude = i32::from_le_bytes(metadata[44..48].try_into().unwrap());
    let elevation = i32::from_le_bytes(metadata[48..52].try_into().unwrap());
    if !metadata[..32].iter().any(|&byte| byte != 0)
        || !metadata[52..].iter().any(|&byte| byte != 0)
        || !(MIN_EPOCH..=MAX_EPOCH).contains(&epoch)
        || !(-900_000_000..=900_000_000).contains(&latitude)
        || !(-1_800_000_000..=1_800_000_000).contains(&longitude)
        || !(-12_000_000..=100_000_000).contains(&elevation)
        || expected > ((MAX_EPOCH - epoch) as u64 * 60) / 1000
        || star_metadata.is_some_and(|previous| previous != metadata)
    {
        return Err(invalid);
    }
    Ok(Some(metadata))
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

fn le_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().unwrap())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

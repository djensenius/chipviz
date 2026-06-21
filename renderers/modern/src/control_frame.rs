pub const MAGIC: u8 = 0xC7;
pub const VERSION: u8 = 0;
pub const SPECTRUM_BANDS: usize = 8;
pub const MAX_NOTES: usize = 4;
pub const WIRE_SIZE: usize = 33;

#[derive(Clone, Copy, Debug, Default)]
pub struct Note {
    pub note: u8,
    pub velocity: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct ControlFrame {
    pub frame: u16,
    pub bpm: u8,
    pub beat_phase: u8,
    pub beat_count: u16,
    pub energy: u8,
    pub bass: u8,
    pub mid: u8,
    pub treble: u8,
    pub spectrum: [u8; SPECTRUM_BANDS],
    pub scene: u8,
    pub palette: u8,
    pub flags: u8,
    pub note_count: u8,
    pub notes: [Note; MAX_NOTES],
}

impl ControlFrame {
    pub fn procedural(frame_index: u64) -> Self {
        let phase = ((frame_index * 17) & 0xFF) as u8;
        let beat_count = (frame_index / 16) as u16;
        let flags = if phase < 17 { 0x01 } else { 0x00 };
        let energy = 80u8.saturating_add(phase / 2);
        let mut spectrum = [0; SPECTRUM_BANDS];

        for (band, value) in spectrum.iter_mut().enumerate() {
            *value = phase.wrapping_add((band as u8).wrapping_mul(29));
        }

        let mut notes = [Note::default(); MAX_NOTES];
        let note_count = if flags & 0x01 != 0 {
            notes[0] = Note {
                note: 48 + (beat_count as u8 % 24),
                velocity: energy,
            };
            1
        } else {
            0
        };

        Self {
            frame: frame_index as u16,
            bpm: 120,
            beat_phase: phase,
            beat_count,
            energy,
            bass: 255u8.wrapping_sub(phase),
            mid: 64u8.saturating_add(((frame_index * 9) & 0x7F) as u8),
            treble: ((frame_index * 23) & 0xFF) as u8,
            spectrum,
            scene: ((frame_index / 96) % 3) as u8,
            palette: ((frame_index / 32) % 8) as u8,
            flags,
            note_count,
            notes,
        }
    }

    pub fn parse_wire(bytes: &[u8]) -> Result<Self, FrameParseError> {
        if bytes.len() != WIRE_SIZE {
            return Err(FrameParseError::WrongSize {
                actual: bytes.len(),
                expected: WIRE_SIZE,
            });
        }
        if bytes[0] != MAGIC {
            return Err(FrameParseError::BadMagic(bytes[0]));
        }
        if bytes[1] != VERSION {
            return Err(FrameParseError::BadVersion(bytes[1]));
        }
        if checksum(&bytes[..WIRE_SIZE - 1]) != bytes[WIRE_SIZE - 1] {
            return Err(FrameParseError::BadChecksum);
        }
        if bytes[22] & 0xE0 != 0 {
            return Err(FrameParseError::ReservedFlags(bytes[22]));
        }
        if bytes[23] as usize > MAX_NOTES {
            return Err(FrameParseError::TooManyNotes(bytes[23]));
        }

        let mut spectrum = [0; SPECTRUM_BANDS];
        spectrum.copy_from_slice(&bytes[12..20]);

        let mut notes = [Note::default(); MAX_NOTES];
        for (index, note) in notes.iter_mut().enumerate() {
            let offset = 24 + index * 2;
            *note = Note {
                note: bytes[offset],
                velocity: bytes[offset + 1],
            };
        }

        Ok(Self {
            frame: u16::from_le_bytes([bytes[2], bytes[3]]),
            bpm: bytes[4],
            beat_phase: bytes[5],
            beat_count: u16::from_le_bytes([bytes[6], bytes[7]]),
            energy: bytes[8],
            bass: bytes[9],
            mid: bytes[10],
            treble: bytes[11],
            spectrum,
            scene: bytes[20],
            palette: bytes[21],
            flags: bytes[22],
            note_count: bytes[23],
            notes,
        })
    }

    pub fn beat_onset(self) -> bool {
        self.flags & 0x01 != 0
    }
}

#[derive(Debug)]
pub enum FrameParseError {
    WrongSize { actual: usize, expected: usize },
    BadMagic(u8),
    BadVersion(u8),
    BadChecksum,
    ReservedFlags(u8),
    TooManyNotes(u8),
}

impl std::fmt::Display for FrameParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongSize { actual, expected } => {
                write!(
                    formatter,
                    "wrong frame size: got {actual}, expected {expected}"
                )
            }
            Self::BadMagic(value) => write!(formatter, "bad frame magic: {value:#04x}"),
            Self::BadVersion(value) => write!(formatter, "unsupported frame version: {value}"),
            Self::BadChecksum => formatter.write_str("bad frame checksum"),
            Self::ReservedFlags(value) => {
                write!(formatter, "reserved frame flags are set: {value:#04x}")
            }
            Self::TooManyNotes(value) => write!(formatter, "too many notes in frame: {value}"),
        }
    }
}

impl std::error::Error for FrameParseError {}

fn checksum(payload: &[u8]) -> u8 {
    payload
        .iter()
        .fold(0, |accumulator, byte| accumulator ^ byte)
}

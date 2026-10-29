//! One MIDI channel-voice message, as midir and Web MIDI deliver it (whole, no running status).

/// `ch` is the MIDI channel 0…15; data values are 0…127.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiMsg {
    NoteOn {
        ch: u8,
        key: u8,
        vel: u8,
    },
    /// Also a note-on with velocity 0.
    NoteOff {
        ch: u8,
        key: u8,
        vel: u8,
    },
    Cc {
        ch: u8,
        cc: u8,
        value: u8,
    },
    /// 14 bits, 8192 = centre.
    PitchBend {
        ch: u8,
        value: u16,
    },
    ChannelPressure {
        ch: u8,
        value: u8,
    },
    Program {
        ch: u8,
        program: u8,
    },
}

/// The data-byte count of a channel-voice status byte (0x80…0xEF, poly pressure included); `None` for data and
/// system bytes.
pub fn data_len(status: u8) -> Option<usize> {
    match status >> 4 {
        0x8..=0xB | 0xE => Some(2),
        0xC | 0xD => Some(1),
        _ => None,
    }
}

impl MidiMsg {
    /// One complete message: a status byte and exactly its data bytes. Poly pressure, system messages (sysex,
    /// clock, active sensing …) and malformed bytes are `None`.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let (&status, data) = bytes.split_first()?;
        if data_len(status)? != data.len() || data.iter().any(|&b| b >= 0x80) {
            return None;
        }
        let ch = status & 0x0F;
        Some(match status >> 4 {
            0x8 => Self::NoteOff {
                ch,
                key: data[0],
                vel: data[1],
            },
            0x9 if data[1] == 0 => Self::NoteOff {
                ch,
                key: data[0],
                vel: 0,
            },
            0x9 => Self::NoteOn {
                ch,
                key: data[0],
                vel: data[1],
            },
            0xB => Self::Cc {
                ch,
                cc: data[0],
                value: data[1],
            },
            0xC => Self::Program {
                ch,
                program: data[0],
            },
            0xD => Self::ChannelPressure { ch, value: data[0] },
            0xE => Self::PitchBend {
                ch,
                value: u16::from(data[0]) | u16::from(data[1]) << 7,
            },
            _ => return None,
        })
    }

    /// The MIDI channel, 0…15.
    pub fn channel(self) -> u8 {
        match self {
            Self::NoteOn { ch, .. }
            | Self::NoteOff { ch, .. }
            | Self::Cc { ch, .. }
            | Self::PitchBend { ch, .. }
            | Self::ChannelPressure { ch, .. }
            | Self::Program { ch, .. } => ch,
        }
    }
}

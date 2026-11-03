// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! A hosted instrument's voices as the host can know them: from the notes it sends and what the instrument reports
//! back. The plugin hosts keep one list per instance and give its count as `Instrument::voices`.

/// Most voices a list keeps.
pub const MAX_VOICES: usize = 512;
/// How long a released voice lasts when the instrument does not report voice ends and states no tail.
const DEFAULT_TAIL_SECONDS: f64 = 1.0;
/// The longest a released voice lasts without a report, also for instruments that do report.
const MAX_TAIL_SECONDS: f64 = 30.0;

#[derive(Debug, Clone, Copy)]
struct Entry {
    key: u8,
    /// Note id the host gave (-1: none).
    id: i32,
    /// Seconds a released voice still lasts; `None` while its note is held.
    left: Option<f64>,
}

/// The voices of one instance, oldest first.
#[derive(Debug, Clone)]
pub struct VoiceLedger {
    voices: Vec<Entry>,
    /// The instrument has reported a voice end: its released voices last until reported.
    reports: bool,
    /// The plugin's voice capacity, when it states one.
    capacity: Option<usize>,
    /// Seconds a released voice lasts while the instrument does not report ends.
    tail: f64,
}

impl Default for VoiceLedger {
    fn default() -> Self {
        VoiceLedger { voices: Vec::with_capacity(64), reports: false, capacity: None, tail: DEFAULT_TAIL_SECONDS }
    }
}

impl VoiceLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// The plugin's voice capacity (CLAP voice-info), if it states one.
    pub fn set_capacity(&mut self, capacity: Option<usize>) {
        self.capacity = capacity.filter(|&c| c > 0);
    }

    /// The tail the plugin states, in seconds (VST 3 tail samples / rate); none or 0 = the default second.
    pub fn set_tail(&mut self, seconds: Option<f64>) {
        self.tail = match seconds {
            Some(s) if s.is_finite() && s > 0.0 => s.min(MAX_TAIL_SECONDS),
            _ => DEFAULT_TAIL_SECONDS,
        };
    }

    /// The instrument reports its voice ends (a Lemon plugin, or a CLAP plugin after its first NOTE_END).
    pub fn reports(&self) -> bool {
        self.reports
    }

    /// A note on: a new voice.
    pub fn start(&mut self, key: u8, id: i32) {
        self.voices.push(Entry { key, id, left: None });
        let limit = self.capacity.unwrap_or(MAX_VOICES).min(MAX_VOICES);
        if self.voices.len() > limit {
            let extra = self.voices.len() - limit;
            self.voices.drain(..extra);
        }
    }

    fn release_left(&self) -> f64 {
        if self.reports {
            MAX_TAIL_SECONDS
        } else {
            self.tail
        }
    }

    /// A note off: the held voices of `key` ring out.
    pub fn release(&mut self, key: u8) {
        let left = self.release_left();
        for v in self.voices.iter_mut().filter(|v| v.key == key && v.left.is_none()) {
            v.left = Some(left);
        }
    }

    /// A choke: the voices of `key` close quickly. An instrument that reports ends keeps them until it does.
    pub fn choke(&mut self, key: u8) {
        if self.reports {
            self.release(key);
        } else {
            self.voices.retain(|v| v.key != key);
        }
    }

    /// A choke of the voice with note id `id` alone (an audio clip's end; the key's other voices play on).
    pub fn choke_voice(&mut self, id: i32) {
        if self.reports {
            let left = self.release_left();
            for v in self.voices.iter_mut().filter(|v| v.id == id && v.left.is_none()) {
                v.left = Some(left);
            }
        } else {
            self.voices.retain(|v| v.id != id);
        }
    }

    /// The instrument reports that a voice ended: by note id when the host gave one, else the oldest voice of the key,
    /// released before held. A report that matches nothing is ignored.
    pub fn ended(&mut self, key: i16, id: i32) {
        if !self.reports {
            // From the first report on, released voices wait for theirs.
            self.reports = true;
            for v in self.voices.iter_mut().filter(|v| v.left.is_some()) {
                v.left = Some(MAX_TAIL_SECONDS);
            }
        }
        let by_id = (id >= 0).then(|| self.voices.iter().position(|v| v.id == id)).flatten();
        let at = by_id.or_else(|| {
            let key = u8::try_from(key).ok()?;
            self.voices.iter().position(|v| v.key == key && v.left.is_some()).or_else(|| self.voices.iter().position(|v| v.key == key))
        });
        if let Some(i) = at {
            self.voices.remove(i);
        }
    }

    /// `seconds` of audio went by: released voices that have outlasted their time end.
    pub fn advance(&mut self, seconds: f64) {
        if self.voices.iter().all(|v| v.left.is_none()) {
            return;
        }
        for v in &mut self.voices {
            if let Some(l) = &mut v.left {
                *l -= seconds;
            }
        }
        self.voices.retain(|v| v.left.is_none_or(|l| l > 0.0));
    }

    /// The instrument says it has nothing more to play: no voices left (`all`), or only the held ones.
    pub fn quiet(&mut self, all: bool) {
        if all {
            self.voices.clear();
        } else {
            self.voices.retain(|v| v.left.is_none());
        }
    }

    /// Reset (Stop): no voices.
    pub fn clear(&mut self) {
        self.voices.clear();
    }

    /// Keys of the voices whose notes are still held, oldest first (one per voice).
    pub fn held(&self) -> impl Iterator<Item = u8> + '_ {
        self.voices.iter().filter(|v| v.left.is_none()).map(|v| v.key)
    }

    /// Voices now, within the plugin's capacity.
    pub fn count(&self) -> usize {
        self.voices.len().min(self.capacity.unwrap_or(MAX_VOICES))
    }
}


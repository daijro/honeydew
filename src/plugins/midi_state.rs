//! Fixed-size MIDI state retained while an asynchronous instrument loads.
//! Replays currently held keys, never stale note-on/off pairs or an unbounded queue.
#[derive(Clone)]
pub(super) struct MidiState {
    notes: [u8; 128],
    controllers: [Option<u8>; 128],
    bend: Option<[u8; 2]>,
    pressure: Option<u8>,
}
impl Default for MidiState {
    fn default() -> Self {
        Self {
            notes: [0; 128],
            controllers: [None; 128],
            bend: None,
            pressure: None,
        }
    }
}
impl MidiState {
    pub fn remember(&mut self, message: [u8; 3]) {
        let [status, key, value] = message;
        if key > 127 || value > 127 {
            return;
        }
        match status & 0xF0 {
            0x90 => self.notes[key as usize] = value,
            0x80 => self.notes[key as usize] = 0,
            0xB0 => match key {
                120 | 123 => self.notes.fill(0),
                121 => {
                    self.controllers = [None; 128];
                    self.controllers[64] = Some(0);
                    self.bend = Some([0, 64]);
                    self.pressure = Some(0);
                }
                _ => self.controllers[key as usize] = Some(value),
            },
            0xD0 => self.pressure = Some(key),
            0xE0 => self.bend = Some([key, value]),
            _ => {}
        }
    }
    pub fn replay(&self, mut send: impl FnMut([u8; 3])) {
        // Restore bank/controllers/pitch before notes, without changing controls
        // the song never sent (in particular expression and volume defaults).
        for (cc, value) in self.controllers.iter().enumerate() {
            if let Some(value) = value {
                send([0xB0, cc as u8, *value]);
            }
        }
        if let Some([lo, hi]) = self.bend {
            send([0xE0, lo, hi]);
        }
        if let Some(value) = self.pressure {
            send([0xD0, value, 0]);
        }
        for (key, velocity) in self.notes.iter().enumerate() {
            if *velocity > 0 {
                send([0x90, key as u8, *velocity]);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn messages(state: &MidiState) -> Vec<[u8; 3]> {
        let mut result = Vec::new();
        state.replay(|m| result.push(m));
        result
    }
    #[test]
    fn loading_replays_only_held_keys_and_restores_controllers_first() {
        let mut state = MidiState::default();
        for m in [
            [0x90, 60, 100],
            [0x80, 60, 0],
            [0x90, 64, 77],
            [0xB0, 1, 50],
            [0xE0, 3, 70],
            [0xD0, 42, 0],
        ] {
            state.remember(m);
        }
        assert_eq!(
            messages(&state),
            [[0xB0, 1, 50], [0xE0, 3, 70], [0xD0, 42, 0], [0x90, 64, 77]]
        );
    }
    #[test]
    fn zero_velocity_and_panic_remove_pending_notes() {
        let mut state = MidiState::default();
        state.remember([0x90, 60, 100]);
        state.remember([0x90, 60, 0]);
        assert!(messages(&state).is_empty());
        for control in [120, 123] {
            state.remember([0x90, 62, 127]);
            state.remember([0xB0, control, 0]);
            assert!(messages(&state).is_empty());
        }
    }
    #[test]
    fn reset_controllers_releases_sustain_without_replaying_old_controls() {
        let mut state = MidiState::default();
        state.remember([0xB0, 64, 127]);
        state.remember([0xB0, 11, 0]);
        state.remember([0xB0, 121, 0]);
        assert_eq!(
            messages(&state),
            [[0xB0, 64, 0], [0xE0, 0, 64], [0xD0, 0, 0]]
        );
        assert!(messages(&MidiState::default()).is_empty());
    }
}

//! Pure preparation helpers for a future sample renderer. These do not synthesize or activate a bank.
//! Unknown words deliberately do not guess an unrelated vowel/sample.

use super::{OtoEntry, VoicebankIndex};
use unicode_normalization::UnicodeNormalization;

/// Decode script differences only; this is not arbitrary-language grapheme-to-phoneme conversion.
fn normalized_alias(text: &str) -> String {
    text.trim()
        .nfc()
        .map(|c| {
            if ('ァ'..='ヶ').contains(&c) {
                char::from_u32(c as u32 - 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

impl VoicebankIndex {
    /// Safe minimal phonemizer: exact alias first, then kana-script equivalence,
    /// then a small explicit romaji CV table. Unmatched words return `None`.
    /// Language-specific word dictionaries and VCV context selection remain separate work.
    pub fn resolve_alias(&self, lyric: &str) -> Option<&OtoEntry> {
        let text: String = lyric.trim().nfc().collect();
        if text.is_empty() {
            return None;
        }
        if let Some(entry) = self.entries.iter().find(|e| e.alias == text) {
            return Some(entry);
        }
        let kana = normalized_alias(&text);
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| normalized_alias(&e.alias) == kana)
        {
            return Some(entry);
        }
        let mapped = match text.to_ascii_lowercase().as_str() {
            "a" => "あ",
            "i" => "い",
            "u" => "う",
            "e" => "え",
            "o" => "お",
            "ka" => "か",
            "ki" => "き",
            "ku" => "く",
            "ke" => "け",
            "ko" => "こ",
            "sa" => "さ",
            "shi" | "si" => "し",
            "su" => "す",
            "se" => "せ",
            "so" => "そ",
            "ta" => "た",
            "chi" | "ti" => "ち",
            "tsu" | "tu" => "つ",
            "te" => "て",
            "to" => "と",
            "na" => "な",
            "ni" => "に",
            "nu" => "ぬ",
            "ne" => "ね",
            "no" => "の",
            "ha" => "は",
            "hi" => "ひ",
            "fu" | "hu" => "ふ",
            "he" => "へ",
            "ho" => "ほ",
            "ma" => "ま",
            "mi" => "み",
            "mu" => "む",
            "me" => "め",
            "mo" => "も",
            "ya" => "や",
            "yu" => "ゆ",
            "yo" => "よ",
            "ra" => "ら",
            "ri" => "り",
            "ru" => "る",
            "re" => "れ",
            "ro" => "ろ",
            "wa" => "わ",
            "wo" => "を",
            "n" => "ん",
            _ => return None,
        };
        self.entries
            .iter()
            .find(|e| normalized_alias(&e.alias) == mapped)
    }
}

/// Millisecond-domain source/destination landmarks, not an audio render result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimingPlan {
    pub source_start: f64,
    pub source_end: f64,
    pub fixed_consonant_end: f64,
    /// Can be negative: a worker must prepend silence/shift the phrase, never drop consonants.
    pub render_start: f64,
    pub note_start: f64,
    pub note_end: f64,
    pub overlap: f64,
}

impl OtoEntry {
    /// Validate and preserve OTO timing before decoding/resampling on a worker.
    /// UTAU positive cutoff trims from WAV end; negative cutoff gives a fixed
    /// duration measured from offset. This sign convention must not be reversed.
    pub fn timing_plan(
        &self,
        note_start: f64,
        note_duration: f64,
        wav_duration: f64,
    ) -> Option<TimingPlan> {
        if ![
            self.offset,
            self.consonant,
            self.cutoff,
            self.preutterance,
            self.overlap,
            note_start,
            note_duration,
            wav_duration,
        ]
        .into_iter()
        .all(f64::is_finite)
            || note_duration <= 0.0
            || wav_duration <= 0.0
        {
            return None;
        }
        let start = self.offset.max(0.0);
        let end = if self.cutoff < 0.0 {
            start - self.cutoff
        } else {
            wav_duration - self.cutoff
        }
        .min(wav_duration);
        if end <= start {
            return None;
        }
        let preutter = self.preutterance.max(0.0);
        Some(TimingPlan {
            source_start: start,
            source_end: end,
            fixed_consonant_end: (start + self.consonant.max(0.0)).min(end),
            render_start: note_start - preutter,
            note_start,
            note_end: note_start + note_duration,
            overlap: self.overlap.clamp(-note_duration, preutter),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bank() -> VoicebankIndex {
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        bytes.extend_from_slice("あ.wav=あ,10,40,-200,80,20\nか.wav=か,0,0,0,0,0".as_bytes());
        super::super::index_files(&[crate::zip::Entry {
            path: "Bank/oto.ini".into(),
            data: bytes,
        }])
        .unwrap()
    }
    #[test]
    fn vocal_basic_alias_resolution_never_turns_hello_into_an_arbitrary_vowel() {
        let bank = bank();
        for input in ["あ", "ア", "a", " A "] {
            assert_eq!(bank.resolve_alias(input).unwrap().alias, "あ");
        }
        assert_eq!(bank.resolve_alias("ka").unwrap().alias, "か");
        for unknown in ["hello", "bonjour", "你好", ""] {
            assert!(bank.resolve_alias(unknown).is_none());
        }
    }
    #[test]
    fn vocal_oto_plan_preserves_leading_consonants_and_signed_cutoff() {
        let bank = bank();
        let mut oto = bank.entries[0].clone();
        let plan = oto.timing_plan(0.0, 500.0, 1000.0).unwrap();
        assert_eq!(
            (
                plan.render_start,
                plan.source_start,
                plan.source_end,
                plan.fixed_consonant_end,
                plan.overlap
            ),
            (-80.0, 10.0, 210.0, 50.0, 20.0)
        );
        oto.cutoff = 100.0;
        assert_eq!(
            oto.timing_plan(0.0, 500.0, 1000.0).unwrap().source_end,
            900.0
        );
        oto.offset = 1001.0;
        assert!(oto.timing_plan(0.0, 500.0, 1000.0).is_none());
    }
}

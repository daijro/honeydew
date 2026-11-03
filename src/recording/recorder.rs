//! The recorder: which channels record, their takes, the files the takes write and the clips they leave.
//!
//! The engine places the takes: it records each rendered segment into the input's [`RecordTap`], starts a take at Play
//! (after any count-in), at arming during playback and at any jump or loop wrap, and drops each take's first L frames
//! ([`latency`]). The app makes a [`Recorder`] at Play with REC armed, which arms the tap; it polls the tap every
//! frame, writing each take's frames to the take's own file, and stops at Stop, which leaves a clip per take. Recording
//! makes no undo step; that is the app's.
//!
//! The song tempo can change under a take (the tempo editor and the Project tab work while the transport plays): a
//! take's live length counts each poll's frames at the tempo the engine plays at in that poll. A take is never longer
//! than `MAX_FRAMES` frames, what the sample decoder takes: it ends there, and the recorder takes no more input
//! ([`Recorder::full`]).

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use honeydew_audio::sample::MAX_FRAMES;
use honeydew_audio::sample_edit::speed_ratio;

use super::files::{TakeWriter, create_take};
use super::input::{InputChoice, InputReader};
use crate::audio_engine::RecordTap;
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::project_files::RECORDINGS_FOLDER;
use crate::song::{Channel, ChannelKind, Clip, LaneKind, PPQ, SamplerSlot, Song};

/// Silence written for frames the recorder lost, a chunk at a time.
const SILENCE: [f32; 4096] = [0.0; 4096];

/// The channels a recording takes, as indices: with `Enable multitrack recording` on and an input of more than one port
/// (`ports`), every Audio Record channel whose arm button is on (CHHD@1060); otherwise the current channel, when it is
/// An Audio Record one.
pub fn recording_channels(
    song: &Song,
    current: usize,
    multitrack: bool,
    ports: usize,
) -> Vec<usize> {
    let records = |c: &Channel| c.kind == ChannelKind::AudioRecord;
    if multitrack && ports > 1 {
        let armed = |(_, c): &(usize, &Channel)| records(c) && c.file.record_armed;
        return song
            .channels
            .iter()
            .enumerate()
            .filter(armed)
            .map(|(i, _)| i)
            .collect();
    }
    (song.channels.get(current).filter(|c| records(c)))
        .map(|_| current)
        .into_iter()
        .collect()
}

/// Whether the latency drop applies, from the stored `Enable audio recording latency compensation` toggle: exactly when
/// The toggle is off; an unset key (`None`) shows the toggle on and still compensates.
pub fn compensates(toggle: Option<bool>) -> bool {
    toggle != Some(true)
}

/// L, the frames each take drops when compensating: this session's calibration result if nonzero, else
/// [`RecordTap::BUFFER`], the engine's audio buffer size in frames, which the engine resolves. Never an OS-reported
/// latency.
pub fn latency(calibration: Option<u32>) -> u32 {
    calibration.filter(|&l| l > 0).unwrap_or(RecordTap::BUFFER)
}

/// One take of one channel: a file, and the clip it becomes at [`Recorder::stop`].
pub struct Take {
    /// The channel's id ([`Channel::id`]), which outlives an index shift.
    pub channel_id: i32,
    /// The lane the clip goes to, as the song was when the take began: an index into the channel's lanes, the
    /// index a new audio lane takes when it is past the channel's last lane that is no automation lane.
    pub lane: usize,
    /// Where the engine started the take, in song ticks.
    pub start_tick: u64,
    /// Frames written: the latency drop is not among them.
    pub frames: u64,
    /// The song ticks the playhead ran while they were written: each poll's frames at the engine's tempo of that poll,
    /// so a tempo change mid-take is kept. An ended take's live bar.
    pub ticks: f64,
    pub path: PathBuf,
    /// `None` once the take has ended and its file is closed.
    writer: Option<TakeWriter>,
}

impl Take {
    /// The take still records: its live bar runs to the playhead.
    pub fn running(&self) -> bool {
        self.writer.is_some()
    }
}

/// A recording under way: from Play to Stop. It reads the takes the engine leaves in the tap, from its arming on.
pub struct Recorder {
    tap: Arc<RecordTap>,
    /// This recording's number ([`RecordTap::arm`]).
    session: u32,
    reader: InputReader,
    /// The tap's next take start to read.
    next: u64,
    /// Each source's channel id and its ports among the input's.
    sources: Vec<(i32, Vec<usize>)>,
    /// Each source's running take, an index into `takes`; `None` until its first frame (its file is made then).
    running: Vec<Option<usize>>,
    takes: Vec<Take>,
    dir: PathBuf,
    title: String,
    /// The engine's tempo at the last poll.
    tempo: f64,
    /// The most frames a take holds: what the sample decoder takes (`MAX_FRAMES`).
    max_frames: u64,
    /// A take reached `max_frames`: the takes are closed and no input is taken.
    full: bool,
    /// The running takes' start tick; `None` until this recording's first take starts.
    start: Option<u64>,
    /// A source's ports of the frames written, reused.
    columns: Vec<f32>,
}

impl Recorder {
    /// Starts recording the channels `sources` (id and input choice; the choice's device is the tap's input) of
    /// `song` into the recordings folder `dir`: arms `tap`, each take dropping its first `skip` frames (L, or 0 without
    /// compensation: [`compensates`], [`latency`]).
    pub fn new(
        tap: Arc<RecordTap>,
        song: &Song,
        sources: &[(i32, InputChoice)],
        dir: PathBuf,
        skip: u32,
    ) -> Self {
        let ports = |c: &InputChoice| std::iter::once(c.first).chain(c.second).collect();
        let reader = InputReader::new(tap.frames().clone());
        let next = tap.started();
        let session = tap.arm(skip);
        Self {
            session,
            reader,
            next,
            tap,
            sources: sources.iter().map(|(id, c)| (*id, ports(c))).collect(),
            running: vec![None; sources.len()],
            takes: Vec::new(),
            dir,
            title: song.title.clone(),
            tempo: song.tempo_bpm,
            max_frames: MAX_FRAMES as u64,
            full: false,
            start: None,
            columns: Vec::new(),
        }
    }

    /// Tests lower the cap ([`MAX_FRAMES`]) to reach it with a few frames.
    #[cfg(test)]
    pub(crate) fn with_max_frames(self, max_frames: u64) -> Self {
        Self { max_frames, ..self }
    }

    /// A take reached the longest length that loads back: the takes are closed, and the recording takes no more input
    /// (the app disarms REC and says so). Stop still leaves the clips.
    pub fn full(&self) -> bool {
        self.full
    }

    /// The longest a take can be, in whole minutes at the input's rate, rounded up: "at most".
    pub fn limit_minutes(&self) -> u64 {
        self.max_frames.div_ceil(u64::from(self.rate()).max(1) * 60)
    }

    /// The takes so far, in the order they began; each source's last one runs.
    pub fn takes(&self) -> &[Take] {
        &self.takes
    }

    fn rate(&self) -> u32 {
        self.tap.frames().rate()
    }

    /// The tap it reads.
    pub fn tap(&self) -> &Arc<RecordTap> {
        &self.tap
    }

    /// Takes what the engine recorded since the last poll, the engine playing at `tempo` BPM
    /// ([`crate::audio_link::AudioLink::engine_tempo`]: not the song's, which a tempo drag edits before the engine is
    /// rebuilt). Each take start of this recording ends the running takes at its frame and starts the next; frames
    /// before the first are another recording's. Frames the recorder lost (it fell more than the tap holds behind)
    /// record as silence, so what follows keeps its place: a gap, not a shift. A take that reaches the cap ends there
    /// ([`Self::full`]).
    pub fn poll(&mut self, song: &Song, tempo: f64) -> io::Result<()> {
        self.tempo = tempo;
        let width = self.tap.frames().channels();
        let ports: Vec<usize> = (0..width).collect();
        let mut buf = vec![0.0; self.reader.available() as usize * width];
        let from = self.reader.position();
        let read = self.reader.read(&ports, &mut buf);
        // The frames read are the last ones up to `to`; those from `from` to `data` were lost.
        let to = self.reader.position();
        let data = to - read as u64;
        // Then the take starts: one the engine marked before frames read here is there too.
        let mut at = from;
        while at < to {
            while let Some(s) = self.next_start().filter(|s| s.frame <= at) {
                self.end_takes()?;
                (self.start, self.next) = (Some(s.tick), self.next + 1);
            }
            let end = self.next_start().map_or(to, |s| s.frame.min(to));
            if self.start.is_some() {
                let mut lost = data.min(end).saturating_sub(at) as usize * width;
                while lost > 0 {
                    let chunk = lost.min(SILENCE.len() / width * width);
                    self.write(song, &SILENCE[..chunk])?;
                    lost -= chunk;
                }
                let [first, last] = [at, end].map(|p| (p.max(data) - data) as usize * width);
                self.write(song, &buf[first..last])?;
            }
            at = end;
        }
        Ok(())
    }

    /// This recording's next take start, past any other's.
    fn next_start(&mut self) -> Option<crate::audio_engine::TakeStart> {
        loop {
            let start = self.tap.start(self.next);
            match start {
                Some(s) if s.session != self.session => self.next += 1,
                // Lapped: the recorder fell more than the tap holds behind; the next it holds.
                None if self.next + 1 < self.tap.started() => self.next += 1,
                _ => return start,
            }
        }
    }

    /// Appends `frames` (every port of the input) to the running takes. A source's first frames begin its take. A
    /// take stops at the cap: the frames past it are left out, the takes end, and nothing is taken after
    /// ([`Self::full`]).
    fn write(&mut self, song: &Song, frames: &[f32]) -> io::Result<()> {
        let width = self.tap.frames().channels();
        if frames.is_empty() || self.full {
            return Ok(());
        }
        let (tempo, rate) = (self.tempo, self.rate());
        let mut columns = std::mem::take(&mut self.columns);
        for source in 0..self.sources.len() {
            let take = match self.running[source] {
                Some(take) => take,
                None => self.begin(song, source)?,
            };
            let ports = &self.sources[source].1;
            let take = &mut self.takes[take];
            let room = (self.max_frames - take.frames) as usize;
            let kept = (frames.len() / width).min(room);
            columns.clear();
            let frames = frames.chunks_exact(width).take(kept);
            columns.extend(
                frames.flat_map(|f| ports.iter().map(|&p| f.get(p).copied().unwrap_or(0.0))),
            );
            take.writer
                .as_mut()
                .expect("a running take")
                .write(&columns)?;
            take.frames += kept as u64;
            take.ticks += ticks_at(tempo, rate, kept as u64);
            self.full |= take.frames >= self.max_frames;
        }
        self.columns = columns;
        if self.full {
            self.end_takes()?;
        }
        Ok(())
    }

    /// Makes `source`'s take at the running start: its file, and its lane after the channel's takes so far.
    fn begin(&mut self, song: &Song, source: usize) -> io::Result<usize> {
        let (id, ref ports) = self.sources[source];
        let start = self.start.unwrap_or(0);
        let (path, writer) = create_take(&self.dir, &self.title, self.rate(), ports.len() as u16)?;
        let pending: Vec<_> = (self.takes.iter().filter(|t| t.channel_id == id))
            .map(|t| (t.lane, t.start_tick, t.start_tick as f64 + t.ticks))
            .collect();
        let channel = song.channels.iter().find(|c| c.id == id);
        self.takes.push(Take {
            channel_id: id,
            lane: channel.map_or(0, |c| pick_lane(c, start, &pending)),
            start_tick: start,
            frames: 0,
            ticks: 0.0,
            path,
            writer: Some(writer),
        });
        self.running[source] = Some(self.takes.len() - 1);
        Ok(self.takes.len() - 1)
    }

    /// Ends the running takes: their files close, sized. The first error comes back after all are closed; a file
    /// that fails to be sized still loads (its sizes say "to the end of the file").
    fn end_takes(&mut self) -> io::Result<()> {
        let mut closed = Ok(());
        for take in self.running.iter_mut().filter_map(Option::take) {
            if let Some(writer) = self.takes[take].writer.take() {
                closed = closed.and(writer.finish().map(drop));
            }
        }
        closed
    }

    /// Stops: disarms the tap, takes what it holds, closes every file and adds each take's clip to `song`, in take
    /// order, on the lane picked again with the song as it is now. The clip starts at the take's start; its sample is
    /// The file, named after it; its length is frames·tempo/(rate·60·pitch ratio) at the song's tempo now, which a
    /// tempo change mid-take left different from the tempo it started at (the clip plays the file at its own speed, and
    /// `fit_audio_clip` finds the same length at any later edit), and its loop follows. Frames the engine records after
    /// this are lost (at most the block it is rendering). A take on a channel gone since leaves its file alone. A file
    /// that fails to close still gets its clip; the first error comes back after.
    pub fn stop(mut self, song: &mut Song) -> io::Result<()> {
        self.tap.disarm();
        let closed = self.poll(song, self.tempo).and(self.end_takes());
        let (b, rate) = (song.time_sig.bar_beats(), f64::from(self.rate()));
        let tempo = song.tempo_bpm;
        for take in self.takes.iter().filter(|t| t.frames > 0) {
            let Some(channel) = song.channels.iter_mut().find(|c| c.id == take.channel_id) else {
                continue;
            };
            let mut lane = pick_lane(channel, take.start_tick, &[]);
            if (channel.lanes.get(lane)).is_none_or(|l| l.kind != LaneKind::Audio) {
                lane = channel.add_lane(LaneKind::Audio);
            }
            let name = (take.path.file_stem().unwrap_or_default())
                .to_string_lossy()
                .into_owned();
            let mut slot = SamplerSlot {
                name: name.clone(),
                folder: RECORDINGS_FOLDER.into(),
                ..SamplerSlot::default()
            };
            slot.file.path = Some(ContentPath::Shorthand {
                root: ContentRoot::User,
                relative: format!("{RECORDINGS_FOLDER}/{name}.wav"),
            });
            let speed = speed_ratio(slot.params[2]);
            let lane = &mut channel.lanes[lane];
            if lane.label.is_empty() {
                lane.label.clone_from(&name);
            }
            let clip = lane.add_clip(u32::try_from(take.start_tick).unwrap_or(u32::MAX), 0.0);
            clip.file.source_len_beats = take.frames as f64 * tempo / (rate * 60.0 * speed);
            clip.file.sample = Some(Box::new(slot));
            clip.fit_loop(b);
        }
        closed
    }
}

/// `frames` of input at `rate` Hz in song ticks at `tempo` BPM.
fn ticks_at(tempo: f64, rate: u32, frames: u64) -> f64 {
    frames as f64 * tempo * f64::from(PPQ) / (60.0 * f64::from(rate))
}

/// The lane a take starting at `start` goes to on `channel`: the first audio lane with no clip covering the start, else
/// A new audio lane, the first after the channel's last lane that is no automation lane. `pending`: the takes not in
/// The song yet, as (lane, start, end ticks); they keep their lanes busy as clips do, new lanes included.
fn pick_lane(channel: &Channel, start: u64, pending: &[(usize, u64, f64)]) -> usize {
    let covers = |from: f64, to: f64| from <= start as f64 && (start as f64) < to;
    let first_new = channel.new_lane_index();
    let usable = |i: usize| match channel.lanes.get(i).filter(|_| i < first_new) {
        Some(lane) => {
            let clear = |c: &Clip| {
                !covers(
                    f64::from(c.start_tick),
                    f64::from(c.start_tick) + c.visible_ticks(),
                )
            };
            lane.kind == LaneKind::Audio && lane.clips.iter().all(clear)
        }
        None => true,
    };
    let free = |i: usize| !(pending.iter()).any(|&(l, s, e)| l == i && covers(s as f64, e));
    (0..).find(|&i| usable(i) && free(i)).unwrap_or(first_new)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::flm::chunk::{ContentPath, ContentRoot};
    use crate::project_files::RECORDINGS_FOLDER;
    use crate::recording::input::InputRing;
    use crate::song::{AutomationTarget, ChannelKind, LaneKind};

    /// 12 800 Hz at 60 BPM: 100 frames a tick, 12 800 a beat.
    const RATE: u32 = 12_800;

    /// A song, an input's tap, and a recordings folder; the tests write into the tap what the engine would.
    struct Rig {
        tap: Arc<RecordTap>,
        song: Song,
        dir: PathBuf,
    }

    impl Rig {
        /// A song titled `Song` at 60 BPM with one Audio Record channel (index 1), the tap of an input of `ports`
        /// ports, and an empty recordings folder for the test `name`.
        fn new(name: &str, ports: usize) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("honeydew-h11-rec-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let mut song = Song {
                title: "Song".into(),
                tempo_bpm: 60.0,
                ..Song::default()
            };
            song.add_channel(ChannelKind::AudioRecord);
            Self {
                tap: Arc::new(RecordTap::new(&InputRing::new(ports, RATE, 1))),
                song,
                dir,
            }
        }

        fn id(&self, index: usize) -> i32 {
            self.song.channels[index].id
        }

        /// Records channel 1 from `Input 1`.
        fn mono(&self) -> Recorder {
            let source = (self.id(1), input(0, None));
            Recorder::new(self.tap.clone(), &self.song, &[source], self.dir.clone(), 0)
        }

        /// The engine starts a take of the armed recording at `tick`.
        fn take(&self, tick: u64) {
            self.tap.mark(self.tap.armed().0, tick);
        }

        /// The engine records `frames` frames whose every port reads `x`.
        fn push(&self, frames: usize, x: f32) {
            let len = frames * self.tap.frames().channels();
            self.tap.frames().push(std::iter::repeat_n(x, len));
        }

        fn poll(&self, recorder: &mut Recorder) {
            recorder.poll(&self.song, self.song.tempo_bpm).unwrap();
        }

        /// The recordings folder's files, by name.
        fn files(&self) -> Vec<String> {
            let mut names: Vec<_> = (std::fs::read_dir(&self.dir).unwrap())
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }

        fn done(self) {
            std::fs::remove_dir_all(&self.dir).unwrap();
        }
    }

    fn input(first: usize, second: Option<usize>) -> InputChoice {
        InputChoice {
            device: 0,
            first,
            second,
        }
    }

    /// A take file's 16-bit samples, interleaved, after checking its 46-byte header: PCM at [`RATE`] with
    /// `channels`, sizes patched.
    fn samples(path: &Path, channels: u16) -> Vec<i16> {
        let bytes = std::fs::read(path).unwrap();
        let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let data = bytes.len() as u32 - 46;
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32_at(4), 38 + data);
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!((u32_at(16), u16_at(20), u16_at(22)), (18, 1, channels));
        assert_eq!(
            (u32_at(24), u32_at(28)),
            (RATE, RATE * 2 * u32::from(channels))
        );
        assert_eq!((u16_at(32), u16_at(34), u16_at(36)), (2 * channels, 16, 0));
        assert_eq!((&bytes[38..42], u32_at(42)), (&b"data"[..], data));
        (bytes[46..].as_chunks::<2>().0.iter())
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect()
    }

    /// trunc(x·32767), the stored sample.
    fn pcm(x: f32) -> i16 {
        (f64::from(x) * 32767.0) as i16
    }

    /// The takes as (lane, start tick, frames).
    fn takes(recorder: &Recorder) -> Vec<(usize, u64, u64)> {
        (recorder.takes().iter())
            .map(|t| (t.lane, t.start_tick, t.frames))
            .collect()
    }

    /// Two passes of the loop (0, 512) ticks = 4 beats = 51 200 frames: 0.25 for the first take, then a wrap and
    /// −0.5 for 28 800 frames of the second, the wrap inside the third of four polls.
    fn two_passes(rig: &mut Rig) -> Recorder {
        rig.song.play_loop = Some((0, 512));
        let mut recorder = rig.mono();
        rig.take(0);
        for (pass_one, pass_two) in [(20_000, 0), (20_000, 0), (11_200, 8_800), (0, 20_000)] {
            rig.push(pass_one, 0.25);
            if pass_one > 0 && pass_two > 0 {
                rig.take(0);
            }
            rig.push(pass_two, -0.5);
            rig.poll(&mut recorder);
        }
        recorder
    }

    #[test]
    fn a_take_ends_exactly_where_the_engine_starts_the_next() {
        let mut rig = Rig::new("wrap", 1);
        let recorder = two_passes(&mut rig);
        // The second pass starts on a new lane: the first take covers the loop start.
        assert_eq!(takes(&recorder), [(0, 0, 51_200), (1, 0, 28_800)]);
        assert_eq!(
            (recorder.takes().iter())
                .map(Take::running)
                .collect::<Vec<_>>(),
            [false, true]
        );
        let paths: Vec<_> = recorder.takes().iter().map(|t| t.path.clone()).collect();
        recorder.stop(&mut rig.song).unwrap();
        let first = samples(&paths[0], 1);
        assert_eq!(first.len(), 51_200);
        assert!(first.iter().all(|&s| s == pcm(0.25)));
        let second = samples(&paths[1], 1);
        assert_eq!(second.len(), 28_800);
        assert!(second.iter().all(|&s| s == pcm(-0.5)));
        rig.done();
    }

    #[test]
    fn two_loop_passes_make_two_files_and_two_clips_on_two_lanes() {
        let mut rig = Rig::new("passes", 1);
        let recorder = two_passes(&mut rig);
        recorder.stop(&mut rig.song).unwrap();
        assert_eq!(rig.files(), ["Song Rec 1.wav", "Song Rec.wav"]);
        let lanes = &rig.song.channels[1].lanes;
        assert_eq!(lanes.len(), 2);
        for (lane, (name, beats)) in lanes.iter().zip([("Song Rec", 4.0), ("Song Rec 1", 2.25)]) {
            assert_eq!(lane.kind, LaneKind::Audio);
            assert_eq!(lane.label, name);
            let [clip] = &lane.clips[..] else {
                panic!("one clip a lane: {:?}", lane.clips);
            };
            assert_eq!((clip.id, clip.start_tick, clip.muted), (1, 0, false));
            // length = frames·tempo/(rate·60·pitch ratio); the loop is the smallest 4·2ᵐ ≥ it (and ≥ b).
            assert_eq!(clip.file.source_len_beats, beats);
            assert_eq!((clip.visible_len_beats, clip.loop_len_beats), (0.0, 4.0));
            assert_eq!(clip.visible_ticks(), beats * 128.0);
            let slot = clip.file.sample.as_deref().unwrap();
            assert_eq!((&*slot.name, &*slot.folder), (name, RECORDINGS_FOLDER));
            let relative = format!("{RECORDINGS_FOLDER}/{name}.wav");
            let path = ContentPath::Shorthand {
                root: ContentRoot::User,
                relative,
            };
            assert_eq!(slot.file.path, Some(path));
        }
        // The clips save and load back as they are, but for the slot ids a load gives out.
        let read = crate::flm::read(&crate::flm::write(&rig.song).unwrap()).unwrap();
        let clips = |song: &Song| -> Vec<_> {
            (song.channels[1].lanes.iter().flat_map(|l| &l.clips))
                .map(|c| {
                    (
                        c.start_tick,
                        c.loop_len_beats,
                        c.file.source_len_beats,
                        c.file
                            .sample
                            .as_ref()
                            .map(|s| (s.name.clone(), s.file.path.clone())),
                    )
                })
                .collect()
        };
        assert_eq!(clips(&read), clips(&rig.song));
        rig.done();
    }

    #[test]
    fn a_take_goes_to_the_first_audio_lane_free_at_its_start_else_a_new_one() {
        let mut channel = Channel::new("Rec".into(), ChannelKind::AudioRecord, 0.0);
        channel.lanes[0].add_clip(0, 4.0); // ticks [0, 512)
        channel.add_lane(LaneKind::Audio);
        channel.add_automation_lane(AutomationTarget {
            instance_id: 0,
            param: 0,
        });
        // Lane 0 is busy at 256 and free from 512 on; lane 1 is free.
        assert_eq!(pick_lane(&channel, 256, &[]), 1);
        assert_eq!(pick_lane(&channel, 512, &[]), 0);
        // A take not in the song yet keeps its lane busy, and a new lane goes before the automation lane.
        assert_eq!(pick_lane(&channel, 256, &[(1, 100, 300.0)]), 2);
        assert_eq!(
            pick_lane(&channel, 256, &[(1, 100, 300.0), (2, 256, 257.0)]),
            3
        );
        assert_eq!(pick_lane(&channel, 300, &[(1, 100, 300.0)]), 1);

        // At stop the new lane is made there, and the automation lane moves after it.
        let mut rig = Rig::new("lanes", 1);
        rig.song.channels[1] = Channel {
            id: rig.id(1),
            ..channel
        };
        rig.song.channels[1].lanes[1].add_clip(0, 4.0);
        let mut recorder = rig.mono();
        rig.take(256);
        rig.push(1_000, 0.5);
        rig.poll(&mut recorder);
        assert_eq!(recorder.takes()[0].lane, 2);
        recorder.stop(&mut rig.song).unwrap();
        let kinds: Vec<_> = rig.song.channels[1].lanes.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [
                LaneKind::Audio,
                LaneKind::Audio,
                LaneKind::Audio,
                LaneKind::Automation
            ]
        );
        assert_eq!(rig.song.channels[1].lanes[2].clips[0].start_tick, 256);
        rig.done();
    }

    #[test]
    fn a_pair_records_stereo_and_a_port_mono_each_into_its_own_file() {
        let mut rig = Rig::new("stereo", 3);
        rig.song.add_channel(ChannelKind::AudioRecord);
        let sources = [(rig.id(1), input(0, None)), (rig.id(2), input(1, Some(2)))];
        let dir = rig.dir.clone();
        let mut recorder = Recorder::new(rig.tap.clone(), &rig.song, &sources, dir, 0);
        rig.take(0);
        rig.tap
            .frames()
            .push([0.1, 0.2, -0.3].repeat(500).into_iter());
        rig.poll(&mut recorder);
        let paths: Vec<_> = recorder.takes().iter().map(|t| t.path.clone()).collect();
        recorder.stop(&mut rig.song).unwrap();
        assert_eq!(rig.files(), ["Song Rec 1.wav", "Song Rec.wav"]);
        assert_eq!(samples(&paths[0], 1), [pcm(0.1)].repeat(500));
        assert_eq!(samples(&paths[1], 2), [pcm(0.2), pcm(-0.3)].repeat(500));
        for channel in [1, 2] {
            let clip = &rig.song.channels[channel].lanes[0].clips[0];
            assert_eq!(clip.file.source_len_beats, 500.0 / f64::from(RATE));
        }
        rig.done();
    }

    #[test]
    fn l_is_the_calibration_else_the_buffer_size_and_the_tap_starts_each_take_with_it() {
        // The stored toggle: on (TRUE) never compensates; off (FALSE), or unset, does.
        assert!(!compensates(Some(true)));
        assert!(compensates(Some(false)));
        assert!(compensates(None));
        // L: the calibration if nonzero, else the engine's buffer size.
        assert_eq!(latency(Some(480)), 480);
        assert_eq!(latency(Some(0)), RecordTap::BUFFER);
        assert_eq!(latency(None), RecordTap::BUFFER);
        // The recorder arms the tap with it, a new recording number each time.
        let rig = Rig::new("latency", 1);
        let source = [(rig.id(1), input(0, None))];
        let first = Recorder::new(rig.tap.clone(), &rig.song, &source, rig.dir.clone(), 480);
        assert_eq!(rig.tap.armed(), (first.session, 480));
        let second = Recorder::new(rig.tap.clone(), &rig.song, &source, rig.dir.clone(), 0);
        assert_eq!(rig.tap.armed(), (first.session + 1, 0));
        let mut song = rig.song.clone();
        second.stop(&mut song).unwrap();
        assert_eq!(rig.tap.armed().0, 0, "stop disarms");
        rig.done();
    }

    #[test]
    fn multitrack_records_the_armed_channels_when_the_input_has_ports_to_spare() {
        let mut song = Song::default();
        for kind in [
            ChannelKind::AudioRecord,
            ChannelKind::AudioRecord,
            ChannelKind::Instrument,
            ChannelKind::AudioRecord,
        ] {
            song.add_channel(kind);
        }
        for index in [2, 3, 4] {
            song.channels[index].file.record_armed = true;
        }
        // Multitrack with a 2-port input: the armed Audio Record channels, whichever is current.
        assert_eq!(recording_channels(&song, 1, true, 2), [2, 4]);
        assert_eq!(recording_channels(&song, 3, true, 2), [2, 4]);
        // A 1-port input, or multitrack off: the current channel alone, when it records audio.
        assert_eq!(recording_channels(&song, 1, true, 1), [1]);
        assert_eq!(recording_channels(&song, 2, false, 2), [2]);
        assert_eq!(recording_channels(&song, 3, false, 2), [0usize; 0]);
        assert_eq!(recording_channels(&song, 9, false, 2), [0usize; 0]);
    }

    #[test]
    fn only_this_recordings_takes_are_its_own() {
        // A take of an earlier recording, still in the tap, and frames before this one's first take start are not
        // its; neither is a take a later recording starts.
        let mut rig = Rig::new("sessions", 1);
        let old = rig.mono();
        rig.take(64);
        rig.push(500, 0.9);
        let old = old.session;
        let mut recorder = rig.mono();
        // The engine still records the earlier recording for a block after this one arms.
        rig.tap.mark(old, 96);
        rig.push(300, 0.9);
        rig.poll(&mut recorder);
        assert_eq!(takes(&recorder), []);
        assert!(rig.files().is_empty());
        rig.take(128);
        rig.push(700, 0.25);
        rig.poll(&mut recorder);
        assert_eq!(takes(&recorder), [(0, 128, 700)]);
        let path = recorder.takes()[0].path.clone();
        recorder.stop(&mut rig.song).unwrap();
        assert_eq!(samples(&path, 1), [pcm(0.25)].repeat(700));
        assert_eq!(rig.song.channels[1].lanes[0].clips[0].start_tick, 128);
        rig.done();

        // Stopped before any take: no file, no clip.
        let mut rig = Rig::new("no-take", 1);
        let recorder = rig.mono();
        rig.push(1_000, 0.9);
        recorder.stop(&mut rig.song).unwrap();
        assert!(rig.files().is_empty());
        assert!(rig.song.channels[1].lanes[0].clips.is_empty());
        rig.done();
    }

    #[test]
    fn take_starts_between_two_polls_split_the_frames_at_their_frames() {
        // Three takes in one poll (a seek, then another): each gets exactly its frames. The first ends at tick 10, so
        // the third, at 40, has its lane.
        let mut rig = Rig::new("seeks", 1);
        let mut recorder = rig.mono();
        rig.take(0);
        rig.push(1_000, 0.25);
        rig.take(1_000);
        rig.push(2_000, -0.25);
        rig.take(40);
        rig.push(300, 0.5);
        rig.poll(&mut recorder);
        assert_eq!(
            takes(&recorder),
            [(0, 0, 1_000), (0, 1_000, 2_000), (0, 40, 300)]
        );
        assert_eq!(recorder.takes()[1].ticks, 20.0);
        // Stop takes what came since the last poll.
        rig.push(500, 0.5);
        let paths: Vec<_> = recorder.takes().iter().map(|t| t.path.clone()).collect();
        recorder.stop(&mut rig.song).unwrap();
        for (path, want) in paths.iter().zip([
            [pcm(0.25)].repeat(1_000),
            [pcm(-0.25)].repeat(2_000),
            [pcm(0.5)].repeat(800),
        ]) {
            assert_eq!(samples(path, 1), want);
        }
        let clips = &rig.song.channels[1].lanes[0].clips;
        let clip = clips.iter().find(|c| c.start_tick == 40).unwrap();
        assert_eq!(clip.file.source_len_beats, 800.0 / f64::from(RATE));
        rig.done();
    }

    #[test]
    fn frames_lost_behind_the_tap_record_as_silence_and_the_rest_keeps_its_place() {
        // The tap holds 2 s, 25 600 frames. After 1 000 polled, 30 000 more come at once: the oldest 4 400 of them
        // are lost. A take starting among those gets its share of the silence.
        let mut rig = Rig::new("overrun", 1);
        let mut recorder = rig.mono();
        rig.take(0);
        rig.push(1_000, 0.25);
        rig.poll(&mut recorder);
        rig.push(2_000, 0.25);
        rig.take(40);
        rig.push(28_000, -0.5);
        rig.poll(&mut recorder);
        assert_eq!(takes(&recorder), [(0, 0, 3_000), (0, 40, 28_000)]);
        let paths: Vec<_> = recorder.takes().iter().map(|t| t.path.clone()).collect();
        recorder.stop(&mut rig.song).unwrap();
        let first = [[pcm(0.25)].repeat(1_000), vec![0; 2_000]].concat();
        assert_eq!(samples(&paths[0], 1), first);
        let second = [vec![0; 2_400], [pcm(-0.5)].repeat(25_600)].concat();
        assert_eq!(samples(&paths[1], 1), second);
        rig.done();
    }

    #[test]
    fn a_take_whose_channel_is_deleted_keeps_its_file_and_leaves_no_clip() {
        let mut rig = Rig::new("deleted", 1);
        rig.song.add_channel(ChannelKind::AudioRecord);
        let mut recorder = rig.mono();
        rig.take(0);
        rig.push(1_000, 0.5);
        rig.poll(&mut recorder);
        rig.song.channels.remove(1);
        recorder.stop(&mut rig.song).unwrap();
        assert_eq!(rig.files(), ["Song Rec.wav"]);
        assert_eq!(samples(&rig.dir.join("Song Rec.wav"), 1).len(), 1_000);
        assert!((rig.song.channels.iter()).all(|c| c.lanes.iter().all(|l| l.clips.is_empty())));
        rig.done();
    }

    // ---- A tempo change mid-take, and a take at the decode cap ----

    #[test]
    fn a_tempo_change_mid_take_keeps_the_take_and_the_clip_is_the_length_at_the_tempo_it_stops_at()
    {
        // 60 BPM is 100 frames a tick, 120 BPM is 50: a block of 12 800 frames (1 s) is 128 ticks at 60 BPM and 256 at
        // 120. One block at the first tempo, two at the second.
        for (first, second) in [(60.0, 120.0), (120.0, 60.0)] {
            let ticks = |tempo: f64| if tempo == 60.0 { 128 } else { 256 };
            let mut rig = Rig::new("tempo", 1);
            rig.song.tempo_bpm = first;
            let mut recorder = rig.mono();
            rig.take(0);
            rig.push(12_800, 0.25);
            rig.poll(&mut recorder);
            rig.song.tempo_bpm = second;
            for x in [-0.5, 0.5] {
                rig.push(12_800, x);
                rig.poll(&mut recorder);
            }
            // The live length counts each poll's frames at its tempo.
            let case = format!("{first} -> {second}");
            assert_eq!(takes(&recorder), [(0, 0, 38_400)], "{case}");
            let want = ticks(first) + 2 * ticks(second);
            assert_eq!(recorder.takes()[0].ticks, f64::from(want), "{case}");
            recorder.stop(&mut rig.song).unwrap();
            // The clip plays the file at its own speed, so it is as long as the file at the song's tempo now: what
            // `fit_audio_clip` finds again at any later edit.
            let clip = &rig.song.channels[1].lanes[0].clips[0];
            let beats = 38_400.0 * second / (f64::from(RATE) * 60.0);
            assert_eq!(clip.file.source_len_beats, beats, "{case}");
            rig.done();
        }
    }

    #[test]
    fn a_take_stops_at_the_decode_cap_and_its_file_loads_back() {
        let mut rig = Rig::new("cap", 1);
        let mut recorder = rig.mono().with_max_frames(1_000);
        assert!(!recorder.full());
        rig.take(0);
        rig.push(700, 0.25);
        rig.poll(&mut recorder);
        assert!(!recorder.full());
        // The poll that passes the cap writes up to it and ends the take, sized.
        rig.push(700, -0.5);
        rig.poll(&mut recorder);
        assert!(recorder.full());
        assert_eq!(takes(&recorder), [(0, 0, 1_000)]);
        let path = recorder.takes()[0].path.clone();
        let bytes = std::fs::read(&path).unwrap();
        let decoded = honeydew_audio::sample::decode_with_limit(&bytes, 1_000).unwrap();
        assert_eq!(decoded.frames(), 1_000);
        let written = samples(&path, 1);
        assert_eq!(written[..700], [pcm(0.25)].repeat(700));
        assert_eq!(written[700..], [pcm(-0.5)].repeat(300));
        // More input, a new take start included, starts no take and adds nothing.
        rig.take(1_000);
        rig.push(700, 0.9);
        rig.poll(&mut recorder);
        assert_eq!(takes(&recorder), [(0, 0, 1_000)]);
        assert_eq!(rig.files(), ["Song Rec.wav"]);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        // Stop leaves the one clip, of the capped file.
        recorder.stop(&mut rig.song).unwrap();
        let clips = &rig.song.channels[1].lanes[0].clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].file.source_len_beats, 1_000.0 / f64::from(RATE));
        rig.done();
    }

    #[test]
    fn every_take_of_a_multitrack_recording_stops_at_the_cap() {
        let mut rig = Rig::new("cap-multi", 3);
        rig.song.add_channel(ChannelKind::AudioRecord);
        let sources = [(rig.id(1), input(0, None)), (rig.id(2), input(1, Some(2)))];
        let dir = rig.dir.clone();
        let mut recorder =
            Recorder::new(rig.tap.clone(), &rig.song, &sources, dir, 0).with_max_frames(300);
        rig.take(0);
        rig.tap
            .frames()
            .push([0.1, 0.2, -0.3].repeat(500).into_iter());
        rig.poll(&mut recorder);
        assert!(recorder.full());
        assert_eq!(takes(&recorder), [(0, 0, 300), (0, 0, 300)]);
        let paths: Vec<_> = recorder.takes().iter().map(|t| t.path.clone()).collect();
        recorder.stop(&mut rig.song).unwrap();
        for (path, channels) in paths.iter().zip([1, 2]) {
            let bytes = std::fs::read(path).unwrap();
            let decoded = honeydew_audio::sample::decode_with_limit(&bytes, 300).unwrap();
            assert_eq!(decoded.frames(), 300);
            assert_eq!(samples(path, channels).len(), 300 * channels as usize);
        }
        rig.done();
    }

    #[test]
    fn the_cap_is_what_the_decoder_takes() {
        let rig = Rig::new("cap-default", 1);
        let recorder = rig.mono();
        assert_eq!(recorder.max_frames, MAX_FRAMES as u64);
        // 2^25 frames at 12 800 Hz are 43.7 minutes: "at most" rounds up.
        assert_eq!(recorder.limit_minutes(), 44);
        rig.done();
    }
}

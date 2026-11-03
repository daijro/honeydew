//! A channel: its mixer side (`RACK`: `RHED`, `RPRM`, modules) and its sequencer side (`CHNL` lanes).

use super::ChannelKind;
use super::drum_kit::DrumKit;
use super::file::ChannelFileState;
use super::lane::{AutomationTarget, Lane, LaneKind};
use super::rack::{Rack, common_index};
use crate::params;

/// The `RPRM` channel params, indexed like channel automation (`AutomationTarget { instance_id: 0, param }`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelParams(pub [f32; 6]);

impl Default for ChannelParams {
    /// The `params::CHANNEL_PARAMS` defaults.
    fn default() -> Self {
        Self(std::array::from_fn(|i| params::CHANNEL_PARAMS[i].default))
    }
}

impl ChannelParams {
    /// Automation names by index: 0–5 are
    /// `params::CHANNEL_PARAMS` (2, 3 and 5 are 0 in the bundled files; the mixer stores Mute and Solo in 2 and 3),
    /// 6 has none and 7–133 are MIDI CC 0 … 126 (there is no CC 127): `CC <n> <name>` by the MIDI 1.0 control name, or
    /// `CC <n>` for an undefined number (issue #19). The send params
    /// (`0x1000` / `0x2000` + a channel id) name their target: [`Self::send_name`].
    pub fn name(index: usize) -> Option<String> {
        let Some(cc) = index.checked_sub(7).filter(|_| index <= 133) else {
            return params::CHANNEL_PARAMS.get(index).map(|p| p.name.to_owned());
        };
        let name = match cc {
            0 => "Bank Select",
            1 => "Modulation",
            2 => "Breath",
            4 => "Foot Controller",
            5 => "Portamento Time",
            6 => "Data Entry",
            7 => "Volume",
            8 => "Balance",
            10 => "Pan",
            11 => "Expression",
            12 => "Effect Control 1",
            13 => "Effect Control 2",
            16..=19 | 80..=83 => "General Purpose",
            64 => "Sustain",
            65 => "Portamento",
            66 => "Sostenuto",
            67 => "Soft Pedal",
            68 => "Legato",
            69 => "Hold 2",
            70 => "Variation",
            71 => "Resonance",
            72 => "Release Time",
            73 => "Attack Time",
            74 => "Brightness",
            75 => "Decay Time",
            76 => "Vibrato Rate",
            77 => "Vibrato Depth",
            78 => "Vibrato Delay",
            84 => "Portamento Control",
            88 => "High Resolution Velocity",
            91 => "Reverb",
            92 => "Tremolo",
            93 => "Chorus",
            94 => "Detune",
            95 => "Phaser",
            96 => "Data Increment",
            97 => "Data Decrement",
            98 => "NRPN LSB",
            99 => "NRPN MSB",
            100 => "RPN LSB",
            101 => "RPN MSB",
            120 => "All Sound Off",
            121 => "Reset All Controllers",
            122 => "Local Control",
            123 => "All Notes Off",
            124 => "Omni Off",
            125 => "Omni On",
            126 => "Mono On",
            _ => return Some(format!("CC {cc}")),
        };
        Some(format!("CC {cc} {name}"))
    }

    /// The name of channel send param `index` for the channel named `target`, the target's id (below 4096) being
    /// in the low bits: `Enable Send (to <target>)` for `0x1000 +` it, `Send Level (to <target>)` for `0x2000 +`
    /// (the send control's labels). `None` for any other index. The caller has the song, so it supplies the target's
    /// name.
    pub fn send_name(index: usize, target: &str) -> Option<String> {
        let what = match index {
            0x1000..=0x1FFF => "Enable Send",
            0x2000..=0x2FFF => "Send Level",
            _ => return None,
        };
        Some(format!("{what} (to {target})"))
    }

    pub fn level(&self) -> f32 {
        self.0[0]
    }

    pub fn pan(&self) -> f32 {
        self.0[1]
    }

    pub fn pitch_bend(&self) -> f32 {
        self.0[4]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub name: String,
    pub kind: ChannelKind,
    /// `CHHD` @1080: unique in the song, 0 on the master ([`super::Song::add_channel`] gives the next one). What
    /// outlives an index shift (a channel deleted, inserted or cloned, an undo): the app re-keys its per-channel state
    /// by it.
    pub id: i32,
    /// `CHHD` @1024: the sub-tracks are hidden (`Hide tracks`), the playlist shows the primary track alone.
    pub zipped: bool,
    /// Channel hue, see `theme::channel_color`.
    pub hue: f32,
    pub params: ChannelParams,
    /// `RHED[8]`: the bus a send owns (unique, ≥ 1); −1 for every other channel.
    pub bus_id: i32,
    /// `RHED[12]`: the bus this channel outputs to, −1 = master.
    pub output_bus: i32,
    pub rack: Rack,
    /// Lane 0 is the Level automation lane on master and send channels, else the main lane.
    pub lanes: Vec<Lane>,
    /// The pads (`RSMP`): a Drums channel's ([`DrumKit::default_kit`] when new, maybe empty when read), `None` on
    /// every other kind unless a file holds a non-default kit there.
    pub drum_kit: Option<DrumKit>,
    /// What only the project file needs (`CHHD` view state, names, `SEND`s, tails).
    pub file: ChannelFileState,
}

impl Channel {
    /// A channel with default params, an empty rack, not a bus, output to the master, and its lane 0.
    pub fn new(name: String, kind: ChannelKind, hue: f32) -> Self {
        let mut channel = Self {
            file: ChannelFileState::default(),
            name,
            kind,
            id: 0,
            zipped: false,
            hue,
            params: ChannelParams::default(),
            bus_id: -1,
            output_bus: -1,
            rack: Rack::default(),
            lanes: Vec::new(),
            drum_kit: (kind == ChannelKind::Drums).then(DrumKit::default_kit),
        };
        let main = match kind {
            ChannelKind::Master | ChannelKind::EffectTrack => {
                channel.add_automation_lane(AutomationTarget {
                    instance_id: 0,
                    param: 0,
                });
                return channel;
            }
            ChannelKind::Drums => LaneKind::Drums,
            ChannelKind::AudioClip | ChannelKind::AudioRecord => LaneKind::Audio,
            ChannelKind::Instrument | ChannelKind::Rack | ChannelKind::Synth(_) => LaneKind::Notes,
        };
        channel.lanes.push(Lane::new(main));
        channel
    }

    /// Where a new track goes: after the last track that is not automation (automation tracks come last). The one
    /// place that rule lives: [`Self::add_lane`], the recorder's lane pick and the app's undo bookkeeping all ask it.
    pub fn new_lane_index(&self) -> usize {
        (self.lanes.iter())
            .rposition(|l| l.kind != LaneKind::Automation)
            .map_or(0, |i| i + 1)
    }

    /// Adds an empty track of `kind` at [`Self::new_lane_index`] and returns its index.
    pub fn add_lane(&mut self, kind: LaneKind) -> usize {
        let at = self.new_lane_index();
        self.lanes.insert(at, Lane::new(kind));
        at
    }

    /// The master channel: hue 0.6 (green under the default theme).
    pub fn master() -> Self {
        Self::new(
            ChannelKind::Master.label().to_owned(),
            ChannelKind::Master,
            0.6,
        )
    }

    /// Equal to `other` but maybe for what is not an edit: its GUID, view and tracks' current clips (see
    /// [`super::Song::keep_unedited`]). Clones `other` only when the two have the same id and differ there.
    pub fn same_edit(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }
        let (a, b) = (&self.file, &other.file);
        let current = |c: &Self| {
            c.lanes
                .iter()
                .map(|l| l.file.current_clip_id)
                .collect::<Vec<_>>()
        };
        if self.id != other.id
            || (a.guid == b.guid && a.view == b.view && current(self) == current(other))
        {
            return false;
        }
        let mut other = other.clone();
        (other.file.guid, other.file.view) = (a.guid, a.view);
        for (lane, mine) in other.lanes.iter_mut().zip(&self.lanes) {
            lane.file.current_clip_id = mine.file.current_clip_id;
        }
        *self == other
    }

    /// The value `target` points at, `None` when it names no param of this channel. Channel params `0x1000` /
    /// `0x2000` + id are the Enable and Level of its send to the channel with id `id`.
    pub fn param(&self, target: AutomationTarget) -> Option<f32> {
        let index = usize::try_from(target.param);
        match target.instance_id {
            0 if target.param >= 0x1000 => {
                let (id, level) = (target.param & 0xFFF, target.param >= 0x2000);
                let send = self.file.sends.iter().find(|s| s.target_channel_id == id)?;
                (target.param < 0x3000).then_some(if level { send.level } else { send.enabled })
            }
            0 => self.params.0.get(index.ok()?).copied(),
            id if id < 0 => {
                let slot = self.drum_kit.as_ref()?.slot_by_id(id.unsigned_abs())?;
                slot.params.get(index.ok()?).copied()
            }
            id => self.rack.get(id)?.param(target.param),
        }
    }

    pub fn param_mut(&mut self, target: AutomationTarget) -> Option<&mut f32> {
        let index = usize::try_from(target.param);
        match target.instance_id {
            0 if target.param >= 0x1000 => {
                let (id, level) = (target.param & 0xFFF, target.param >= 0x2000);
                let sends = self.file.sends.iter_mut();
                let send = sends
                    .filter(|_| target.param < 0x3000)
                    .find(|s| s.target_channel_id == id)?;
                Some(if level {
                    &mut send.level
                } else {
                    &mut send.enabled
                })
            }
            0 => self.params.0.get_mut(index.ok()?),
            id if id < 0 => {
                let kit = self.drum_kit.as_mut()?;
                let slot = kit
                    .slots
                    .iter_mut()
                    .find(|s| s.file.id == id.unsigned_abs())?;
                slot.params.get_mut(index.ok()?)
            }
            id => self.rack.get_mut(id)?.param_mut(target.param),
        }
    }

    /// The lane's auto name (`DESc` @28): `"<Param> : <Module>"` from the registry's
    /// automation name and module display name, and the param name alone for the channel's own params
    /// (`"Level"`, `"Pitch Bend"`). `None` when the target names no param of this channel, or a module type the
    /// registry lacks.
    /// A drum slot's is `"<Param> : <Pad>"`, the pad named as its row header's first line
    /// ([`super::SamplerSlot::display_name`]), else by its sample's name, else `"<Param>"` alone.
    pub fn automation_label(&self, target: AutomationTarget) -> Option<String> {
        if target == super::TEMPO_TARGET && self.kind == ChannelKind::Master {
            return Some("Tempo".to_owned());
        }
        if target.instance_id == 0 {
            let index = usize::try_from(target.param).ok()?;
            return ChannelParams::name(index);
        }
        if target.instance_id < 0 {
            let slot = (self.drum_kit.as_ref())?.slot_by_id(target.instance_id.unsigned_abs())?;
            let param = params::DRUM_SLOT_PARAMS.get(usize::try_from(target.param).ok()?)?;
            // A pad with no name (no custom name, no folder) reads its sample's name, else the param alone.
            let pad = [slot.display_name(), &slot.name]
                .into_iter()
                .find(|n| !n.is_empty());
            return Some(match pad {
                Some(pad) => format!("{} : {pad}", param.name),
                None => param.name.to_owned(),
            });
        }
        let module = self.rack.get(target.instance_id)?;
        let param = match usize::try_from(target.param) {
            Ok(i) => module.param_spec(i)?.name,
            Err(_) => params::COMMON_PARAMS.get(common_index(target.param)?)?.name,
        };
        Some(format!("{param} : {}", module.title()))
    }

    /// Adds an automation lane for `target`, labelled by [`Self::automation_label`] (empty when `None`), where a file
    /// keeps it: after the other lanes and after the automation lanes that sort before or with it
    /// ([`AutomationTarget::file_order`]).
    pub fn add_automation_lane(&mut self, target: AutomationTarget) -> &mut Lane {
        let label = self.automation_label(target).unwrap_or_default();
        let order = target.file_order();
        let at = (self.lanes.iter())
            .rposition(|l| {
                l.kind != LaneKind::Automation || l.target.is_some_and(|t| t.file_order() <= order)
            })
            .map_or(0, |i| i + 1);
        self.lanes.insert(
            at,
            Lane {
                target: Some(target),
                label,
                ..Lane::new(LaneKind::Automation)
            },
        );
        &mut self.lanes[at]
    }
}

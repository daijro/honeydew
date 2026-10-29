//! A song's routing: each channel's `SEND`s as the mixer's sends, its modules' sidechains as the mixer's keys, the
//! channel ids the send params name (0x1000 / 0x2000 + channel id), and which channels a solo keeps audible.

use honeydew_audio::mixer::{ChannelMix, SendMix, send_param};

use crate::song::Song;

pub(super) struct Routing {
    /// Per channel index, its channel id (`CHHD`@1080), which sends and pad targets name.
    pub ids: Vec<i32>,
    /// Per channel, the other channels a path of sends or pad targets joins it to, either way, the master left out:
    /// they stay audible while it is soloed, and it while one of them is.
    pub linked: Vec<Vec<usize>>,
    /// Each sidechain kept: (the keyed module's channel index, its instance id, the source's channel index).
    pub keyed: Vec<(usize, i32, usize)>,
}

impl Routing {
    /// The channel index of channel id `id`.
    pub fn index(&self, id: i32) -> Option<usize> {
        self.ids.iter().position(|&other| other == id)
    }

    /// The send that channel param `index` names: (its target's index, whether it is the level). `None` for any
    /// other param, or a target that is not a channel.
    pub fn send(&self, index: i32) -> Option<(usize, bool)> {
        let (id, level) = send_param(index)?;
        Some((self.index(id)?, level))
    }
}

/// Each channel's strip and sends, and the routing. A send is left out when it names no channel, the channel itself
/// or a target the channel sends to already (a duplicate target is ignored), comes from the master, or would
/// close a cycle. The app never makes a cycle; a file's is broken at the send that closes it, scanning the channels in
/// order. Enable Send does not change the routing, and a pad's send target always
/// makes its drum channel a source of that target, for the solo links only: the pads play into the target's
/// buffer before any channel runs, so they need no mixing order. A sidechain ([`ModuleInstance::key_source`]) makes its
/// source a key of the module's channel; one from the master (whose output is the song's), from the channel itself or
/// from no channel is left out, and so is one that would close a cycle through the sends and the sidechains before it,
/// scanning the channels and their racks in order: the module then detects on its own input.
///
/// [`ModuleInstance::key_source`]: crate::song::ModuleInstance::key_source
pub(super) fn build(song: &Song) -> (Vec<ChannelMix>, Routing) {
    let ids: Vec<i32> = song.channels.iter().map(|c| c.id).collect();
    let count = ids.len();
    let mut targets: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut mixes = Vec::with_capacity(count);
    for (index, channel) in song.channels.iter().enumerate() {
        let mut sends = Vec::new();
        for send in channel.file.sends.iter().filter(|_| index != 0) {
            let target = ids.iter().position(|&id| id == send.target_channel_id);
            let Some(target) = target.filter(|&t| t != index && !targets[index].contains(&t))
            else {
                continue;
            };
            if reaches(&targets, target, index) {
                continue;
            }
            targets[index].push(target);
            sends.push(SendMix {
                target,
                enabled: unit(send.enabled),
                level: unit(send.level),
            });
        }
        mixes.push(ChannelMix {
            level: channel.params.level(),
            pan: channel.params.pan(),
            sends,
            keys: Vec::new(),
        });
    }
    // The sends and the sidechains kept, for the cycles: a key is no solo link, as it hears its source whatever the
    // source's strip does.
    let mut graph = targets.clone();
    let mut keyed = Vec::new();
    for (index, channel) in song.channels.iter().enumerate() {
        for module in &channel.rack.modules {
            let source = module.key_source();
            let Some(source) = source.and_then(|id| ids.iter().position(|&other| other == id))
            else {
                continue;
            };
            if source == 0 || source == index || reaches(&graph, index, source) {
                continue;
            }
            if !graph[source].contains(&index) {
                graph[source].push(index);
            }
            if !mixes[index].keys.contains(&source) {
                mixes[index].keys.push(source);
            }
            keyed.push((index, module.instance_id, source));
        }
    }
    let mut edges = targets;
    for (index, channel) in song.channels.iter().enumerate() {
        let slots = channel.drum_kit.iter().flat_map(|kit| &kit.slots);
        for slot in slots.filter(|slot| slot.file.send_target >= 0) {
            if let Some(target) = ids.iter().position(|&id| id == slot.file.send_target)
                && target != index
                && !edges[index].contains(&target)
            {
                edges[index].push(target);
            }
        }
    }
    let linked = (0..count)
        .map(|index| {
            (1..count)
                .filter(|&other| {
                    other != index
                        && (reaches(&edges, index, other) || reaches(&edges, other, index))
                })
                .collect()
        })
        .collect();
    (mixes, Routing { ids, linked, keyed })
}

/// A file value as a mixer control: clamped to 0..=1, a non-finite one 0 (as the reader reads one elsewhere).
fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Whether a path of `edges` leads from `from` to `to` (or `from` is `to`).
pub(super) fn reaches(edges: &[Vec<usize>], from: usize, to: usize) -> bool {
    let mut seen = vec![false; edges.len()];
    let mut stack = vec![from];
    while let Some(node) = stack.pop() {
        if node == to {
            return true;
        }
        if !std::mem::replace(&mut seen[node], true) {
            stack.extend(&edges[node]);
        }
    }
    false
}

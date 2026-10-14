//! MIDI-CC links, kept in a channel's `CCLK` chunk. The raw bytes (`ChannelFileState::cc_links`) stay the stored truth:
//! they are rewritten only when a link changes, so an untouched file round-trips byte-exact.

use crate::song::{AutomationTarget, Song};

/// CC `cc` sets the param `param` of the channel's module `instance_id` (0 = the channel, < 0 = the drum slot
/// −instance_id), as an [`AutomationTarget`] names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CcLink {
    pub cc: u8,
    pub instance_id: i32,
    pub param: i32,
}

/// The `CCLK` record layout, the one place that knows it: {i32 CC, i32 module id, i32 param} in creation
/// order, no MIDI channel, range or curve; a record with CC < 0 or param < 0 is ignored. A CC above 127 never fires,
/// so it is dropped too. As c < 0 is ignored, a module's common params (model −1…−3) are stored as their
/// positive ids 0x7FC0…0x7FC2.
fn decode([a, b, c]: [i32; 3]) -> Option<CcLink> {
    let cc = u8::try_from(a).ok().filter(|&cc| cc <= 127)?;
    (c >= 0).then(|| CcLink {
        cc,
        instance_id: b,
        param: AutomationTarget::model_param(b, c),
    })
}

fn encode(link: CcLink) -> [i32; 3] {
    let c = match link.param {
        AutomationTarget::POST_GAIN if link.instance_id > 0 => 0x7FC0,
        AutomationTarget::MIX if link.instance_id > 0 => 0x7FC1,
        p => AutomationTarget::file_param(link.instance_id, p),
    };
    [i32::from(link.cc), link.instance_id, c]
}

/// The valid links in `raw`, in file order, duplicates dropped; a partial trailing record is ignored.
pub fn links(raw: &[u8]) -> Vec<CcLink> {
    let mut out = Vec::new();
    for record in raw.chunks_exact(12) {
        let field = |i: usize| i32::from_le_bytes([0, 1, 2, 3].map(|b| record[4 * i + b]));
        if let Some(link) = decode([field(0), field(1), field(2)])
            && !out.contains(&link)
        {
            out.push(link);
        }
    }
    out
}

pub fn set_links(raw: &mut Vec<u8>, links: &[CcLink]) {
    raw.clear();
    raw.extend(
        links
            .iter()
            .flat_map(|&l| encode(l))
            .flat_map(i32::to_le_bytes),
    );
}

/// Links `cc` to the param, after the links it has: a param may have several CCs. The bytes are kept when that exact
/// link is there already, as load drops a duplicate.
pub fn link(raw: &mut Vec<u8>, cc: u8, instance_id: i32, param: i32) {
    let new = CcLink {
        cc,
        instance_id,
        param,
    };
    let mut all = links(raw);
    if !all.contains(&new) {
        all.push(new);
        set_links(raw, &all);
    }
}

/// Removes every link of the param; the bytes are kept when it has none.
pub fn unlink(raw: &mut Vec<u8>, instance_id: i32, param: i32) {
    let mut all = links(raw);
    let count = all.len();
    all.retain(|l| (l.instance_id, l.param) != (instance_id, param));
    if all.len() != count {
        set_links(raw, &all);
    }
}

pub fn linked(raw: &[u8], instance_id: i32, param: i32) -> bool {
    has(&links(raw), instance_id, param)
}

/// Whether one of `links` drives the param.
pub fn has(links: &[CcLink], instance_id: i32, param: i32) -> bool {
    links
        .iter()
        .any(|l| (l.instance_id, l.param) == (instance_id, param))
}

/// Every channel's links, with the channel index: a CC fires them all.
pub fn song_links(song: &Song) -> Vec<(usize, CcLink)> {
    let per_channel = song.channels.iter().map(|c| links(&c.file.cc_links));
    per_channel
        .enumerate()
        .flat_map(|(i, l)| l.into_iter().map(move |l| (i, l)))
        .collect()
}

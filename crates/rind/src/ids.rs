// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rind's permanent parameter table. GENERATED once, then maintained by hand: **never rename, reorder, remove or reuse an
//! id; new parameters go at the end** (`tests/ids.golden` and `tests/ids_frozen.rs` hold the line). Songs know a parameter
//! by its id, and Honeydew's module parameter `1 + i` is entry `i` of [`SPECS`].

/// How a parameter's range maps from normalised to plain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Lin(f32, f32),
    /// `skew` is nih-plug's `FloatRange::skew_factor` argument (negative: more resolution at the low end).
    Skew(f32, f32, f32),
    Int(i32, i32),
}

/// How a plain value reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fmt {
    Plain,
    Int,
    Pct,
    Hz,
    Sec,
    Ms,
    Db,
    Semi,
    Cents,
    Pan,
    Oct,
    Ratio,
    Names(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub default: f32,
    pub fmt: Fmt,
}

pub const WAVE_NAMES: &[&str] = &[
    "Sine",
    "Saw",
    "Triangle",
    "Pulse",
    "Half Pulse",
    "Quarter Pulse",
    "TriSaw",
    "Noise",
];
pub const LFO_WAVE_NAMES: &[&str] = &[
    "Sine",
    "Triangle",
    "Saw Up",
    "Saw Down",
    "Square",
    "Pulse 25%",
    "Pulse 12%",
    "Sample & Hold",
    "Smooth Random",
    "Stairs",
    "Rect. Sine",
];
pub const DIV_NAMES: &[&str] = &[
    "2/1", "1/1", "1/2.", "1/2", "1/4.", "1/4", "1/4T", "1/8.", "1/8", "1/8T", "1/16.", "1/16",
    "1/16T", "1/32", "1/64",
];
pub const FILTER_TYPE_NAMES: &[&str] = &["Low", "Band", "High"];
pub const SLOPE_NAMES: &[&str] = &["12 dB", "24 dB"];
pub const OFFON_NAMES: &[&str] = &["Off", "On"];
pub const POLYMONO_NAMES: &[&str] = &["Poly", "Mono"];
pub const PORTA_MODE_NAMES: &[&str] = &["Always", "Legato"];
pub const ARP_MODE_NAMES: &[&str] = &[
    "Up",
    "Down",
    "Up/Down",
    "Down/Up",
    "Up/Down Incl",
    "Down/Up Incl",
    "Random",
    "As Played",
    "Chord",
    "Thumb",
];
pub const ARP_VEL_NAMES: &[&str] = &["Key", "Key Hold", "Step", "Step x Key", "Step x Hold"];
pub const DIST_TYPE_NAMES: &[&str] = &["Overdrive", "Foldback", "Clip", "Decimate", "Bitcrush"];
pub const SOURCE_NAMES: &[&str] = &[
    "Off",
    "LFO 1",
    "LFO 2",
    "Env 3",
    "Env 4",
    "Velocity",
    "Key",
    "Mod Wheel",
    "Pitch Wheel",
    "Aftertouch",
    "Random",
    "Arp Step",
    "Amp Env A",
    "Amp Env B",
];
pub const TARGET_NAMES: &[&str] = &[
    "Off",
    "Cutoff 1",
    "Cutoff 2",
    "Resonance 1",
    "Resonance 2",
    "Drive 1",
    "Drive 2",
    "Pitch All",
    "Pitch Part A",
    "Pitch Part B",
    "Pitch A1",
    "Pitch A2",
    "Pitch B1",
    "Pitch B2",
    "Detune All",
    "Level A",
    "Level B",
    "Pan A",
    "Pan B",
    "Amp All",
    "LFO 1 Rate",
    "LFO 2 Rate",
    "Phase A1",
    "Phase A2",
    "Phase B1",
    "Phase B2",
    "Phase Part A",
    "Phase Part B",
    "Phase All",
    "Cutoff All",
    "Resonance All",
    "Level All",
    "Pan All",
    "LFO 1 Gain",
    "LFO 2 Gain",
];
/// Length of a note division in quarter notes, by [`DIV_NAMES`] index.
pub const DIV_BEATS: [f32; 15] = [
    8.0,
    4.0,
    3.0,
    2.0,
    1.5,
    1.0,
    0.6666666666666666,
    0.75,
    0.5,
    0.3333333333333333,
    0.375,
    0.25,
    0.16666666666666666,
    0.125,
    0.0625,
];

pub const COUNT: usize = 267;
pub const MASTER_VOL: usize = 0;
pub const MASTER_TUNE: usize = 1;
pub const POLY: usize = 2;
pub const MONO: usize = 3;
pub const PORTA: usize = 4;
pub const PORTA_MODE: usize = 5;
pub const BEND: usize = 6;
pub const VEL_AMP: usize = 7;
pub const PART_A_ON: usize = 8;
pub const PART_A_LEVEL: usize = 9;
pub const PART_A_PAN: usize = 10;
pub const PART_A_ATT: usize = 11;
pub const PART_A_DEC: usize = 12;
pub const PART_A_SUS: usize = 13;
pub const PART_A_REL: usize = 14;
pub const PART_B_ON: usize = 15;
pub const PART_B_LEVEL: usize = 16;
pub const PART_B_PAN: usize = 17;
pub const PART_B_ATT: usize = 18;
pub const PART_B_DEC: usize = 19;
pub const PART_B_SUS: usize = 20;
pub const PART_B_REL: usize = 21;
pub const OSC1_WAVE: usize = 22;
pub const OSC1_VOICES: usize = 23;
pub const OSC1_DETUNE: usize = 24;
pub const OSC1_STEREO: usize = 25;
pub const OSC1_OCT: usize = 26;
pub const OSC1_SEMI: usize = 27;
pub const OSC1_FINE: usize = 28;
pub const OSC1_PAN: usize = 29;
pub const OSC1_VOL: usize = 30;
pub const OSC1_PHASE: usize = 31;
pub const OSC1_RAND: usize = 32;
pub const OSC1_INV: usize = 33;
pub const OSC2_WAVE: usize = 34;
pub const OSC2_VOICES: usize = 35;
pub const OSC2_DETUNE: usize = 36;
pub const OSC2_STEREO: usize = 37;
pub const OSC2_OCT: usize = 38;
pub const OSC2_SEMI: usize = 39;
pub const OSC2_FINE: usize = 40;
pub const OSC2_PAN: usize = 41;
pub const OSC2_VOL: usize = 42;
pub const OSC2_PHASE: usize = 43;
pub const OSC2_RAND: usize = 44;
pub const OSC2_INV: usize = 45;
pub const OSC3_WAVE: usize = 46;
pub const OSC3_VOICES: usize = 47;
pub const OSC3_DETUNE: usize = 48;
pub const OSC3_STEREO: usize = 49;
pub const OSC3_OCT: usize = 50;
pub const OSC3_SEMI: usize = 51;
pub const OSC3_FINE: usize = 52;
pub const OSC3_PAN: usize = 53;
pub const OSC3_VOL: usize = 54;
pub const OSC3_PHASE: usize = 55;
pub const OSC3_RAND: usize = 56;
pub const OSC3_INV: usize = 57;
pub const OSC4_WAVE: usize = 58;
pub const OSC4_VOICES: usize = 59;
pub const OSC4_DETUNE: usize = 60;
pub const OSC4_STEREO: usize = 61;
pub const OSC4_OCT: usize = 62;
pub const OSC4_SEMI: usize = 63;
pub const OSC4_FINE: usize = 64;
pub const OSC4_PAN: usize = 65;
pub const OSC4_VOL: usize = 66;
pub const OSC4_PHASE: usize = 67;
pub const OSC4_RAND: usize = 68;
pub const OSC4_INV: usize = 69;
pub const FILT1_ON: usize = 70;
pub const FILT1_TYPE: usize = 71;
pub const FILT1_SLOPE: usize = 72;
pub const FILT1_CUTOFF: usize = 73;
pub const FILT1_RES: usize = 74;
pub const FILT1_DRIVE: usize = 75;
pub const FILT1_KEY: usize = 76;
pub const FILT1_IN_A: usize = 77;
pub const FILT1_IN_B: usize = 78;
pub const FILT1_VOL: usize = 79;
pub const FILT1_PAN: usize = 80;
pub const FILT2_ON: usize = 81;
pub const FILT2_TYPE: usize = 82;
pub const FILT2_SLOPE: usize = 83;
pub const FILT2_CUTOFF: usize = 84;
pub const FILT2_RES: usize = 85;
pub const FILT2_DRIVE: usize = 86;
pub const FILT2_KEY: usize = 87;
pub const FILT2_IN_A: usize = 88;
pub const FILT2_IN_B: usize = 89;
pub const FILT2_VOL: usize = 90;
pub const FILT2_PAN: usize = 91;
pub const ENV3_ATT: usize = 92;
pub const ENV3_DEC: usize = 93;
pub const ENV3_SUS: usize = 94;
pub const ENV3_REL: usize = 95;
pub const ENV4_ATT: usize = 96;
pub const ENV4_DEC: usize = 97;
pub const ENV4_SUS: usize = 98;
pub const ENV4_REL: usize = 99;
pub const LFO1_WAVE: usize = 100;
pub const LFO1_RATE: usize = 101;
pub const LFO1_SYNC: usize = 102;
pub const LFO1_DIV: usize = 103;
pub const LFO1_GAIN: usize = 104;
pub const LFO1_OFFSET: usize = 105;
pub const LFO1_PHASE: usize = 106;
pub const LFO1_RETRIG: usize = 107;
pub const LFO2_WAVE: usize = 108;
pub const LFO2_RATE: usize = 109;
pub const LFO2_SYNC: usize = 110;
pub const LFO2_DIV: usize = 111;
pub const LFO2_GAIN: usize = 112;
pub const LFO2_OFFSET: usize = 113;
pub const LFO2_PHASE: usize = 114;
pub const LFO2_RETRIG: usize = 115;
pub const MAT1_SRC: usize = 116;
pub const MAT1_DST: usize = 117;
pub const MAT1_AMT: usize = 118;
pub const MAT2_SRC: usize = 119;
pub const MAT2_DST: usize = 120;
pub const MAT2_AMT: usize = 121;
pub const MAT3_SRC: usize = 122;
pub const MAT3_DST: usize = 123;
pub const MAT3_AMT: usize = 124;
pub const MAT4_SRC: usize = 125;
pub const MAT4_DST: usize = 126;
pub const MAT4_AMT: usize = 127;
pub const MAT5_SRC: usize = 128;
pub const MAT5_DST: usize = 129;
pub const MAT5_AMT: usize = 130;
pub const MAT6_SRC: usize = 131;
pub const MAT6_DST: usize = 132;
pub const MAT6_AMT: usize = 133;
pub const MAT7_SRC: usize = 134;
pub const MAT7_DST: usize = 135;
pub const MAT7_AMT: usize = 136;
pub const MAT8_SRC: usize = 137;
pub const MAT8_DST: usize = 138;
pub const MAT8_AMT: usize = 139;
pub const MAT9_SRC: usize = 140;
pub const MAT9_DST: usize = 141;
pub const MAT9_AMT: usize = 142;
pub const MAT10_SRC: usize = 143;
pub const MAT10_DST: usize = 144;
pub const MAT10_AMT: usize = 145;
pub const MAT11_SRC: usize = 146;
pub const MAT11_DST: usize = 147;
pub const MAT11_AMT: usize = 148;
pub const MAT12_SRC: usize = 149;
pub const MAT12_DST: usize = 150;
pub const MAT12_AMT: usize = 151;
pub const MAT13_SRC: usize = 152;
pub const MAT13_DST: usize = 153;
pub const MAT13_AMT: usize = 154;
pub const MAT14_SRC: usize = 155;
pub const MAT14_DST: usize = 156;
pub const MAT14_AMT: usize = 157;
pub const MAT15_SRC: usize = 158;
pub const MAT15_DST: usize = 159;
pub const MAT15_AMT: usize = 160;
pub const MAT16_SRC: usize = 161;
pub const MAT16_DST: usize = 162;
pub const MAT16_AMT: usize = 163;
pub const ARP_ON: usize = 164;
pub const ARP_MODE: usize = 165;
pub const ARP_DIV: usize = 166;
pub const ARP_OCT: usize = 167;
pub const ARP_GATE: usize = 168;
pub const ARP_STEPS: usize = 169;
pub const ARP_HOLD: usize = 170;
pub const ARP_STEP1: usize = 171;
pub const ARP_STEP2: usize = 172;
pub const ARP_STEP3: usize = 173;
pub const ARP_STEP4: usize = 174;
pub const ARP_STEP5: usize = 175;
pub const ARP_STEP6: usize = 176;
pub const ARP_STEP7: usize = 177;
pub const ARP_STEP8: usize = 178;
pub const ARP_STEP9: usize = 179;
pub const ARP_STEP10: usize = 180;
pub const ARP_STEP11: usize = 181;
pub const ARP_STEP12: usize = 182;
pub const ARP_STEP13: usize = 183;
pub const ARP_STEP14: usize = 184;
pub const ARP_STEP15: usize = 185;
pub const ARP_STEP16: usize = 186;
pub const ARP_VELMODE: usize = 187;
pub const ARP_TR1: usize = 188;
pub const ARP_TR2: usize = 189;
pub const ARP_TR3: usize = 190;
pub const ARP_TR4: usize = 191;
pub const ARP_TR5: usize = 192;
pub const ARP_TR6: usize = 193;
pub const ARP_TR7: usize = 194;
pub const ARP_TR8: usize = 195;
pub const ARP_TR9: usize = 196;
pub const ARP_TR10: usize = 197;
pub const ARP_TR11: usize = 198;
pub const ARP_TR12: usize = 199;
pub const ARP_TR13: usize = 200;
pub const ARP_TR14: usize = 201;
pub const ARP_TR15: usize = 202;
pub const ARP_TR16: usize = 203;
pub const ARP_HOLD1: usize = 204;
pub const ARP_HOLD2: usize = 205;
pub const ARP_HOLD3: usize = 206;
pub const ARP_HOLD4: usize = 207;
pub const ARP_HOLD5: usize = 208;
pub const ARP_HOLD6: usize = 209;
pub const ARP_HOLD7: usize = 210;
pub const ARP_HOLD8: usize = 211;
pub const ARP_HOLD9: usize = 212;
pub const ARP_HOLD10: usize = 213;
pub const ARP_HOLD11: usize = 214;
pub const ARP_HOLD12: usize = 215;
pub const ARP_HOLD13: usize = 216;
pub const ARP_HOLD14: usize = 217;
pub const ARP_HOLD15: usize = 218;
pub const ARP_HOLD16: usize = 219;
pub const DIST_ON: usize = 220;
pub const DIST_TYPE: usize = 221;
pub const DIST_DRIVE: usize = 222;
pub const DIST_TONE: usize = 223;
pub const DIST_MIX: usize = 224;
pub const PHS_ON: usize = 225;
pub const PHS_RATE: usize = 226;
pub const PHS_DEPTH: usize = 227;
pub const PHS_FREQ: usize = 228;
pub const PHS_FB: usize = 229;
pub const PHS_SPREAD: usize = 230;
pub const PHS_MIX: usize = 231;
pub const CHO_ON: usize = 232;
pub const CHO_RATE: usize = 233;
pub const CHO_DEPTH: usize = 234;
pub const CHO_DELAY: usize = 235;
pub const CHO_FB: usize = 236;
pub const CHO_MIX: usize = 237;
pub const EQ_ON: usize = 238;
pub const EQ_BASS_F: usize = 239;
pub const EQ_BASS_G: usize = 240;
pub const EQ_TREB_F: usize = 241;
pub const EQ_TREB_G: usize = 242;
pub const DLY_ON: usize = 243;
pub const DLY_SYNC: usize = 244;
pub const DLY_L: usize = 245;
pub const DLY_R: usize = 246;
pub const DLY_DIV_L: usize = 247;
pub const DLY_DIV_R: usize = 248;
pub const DLY_FB: usize = 249;
pub const DLY_PP: usize = 250;
pub const DLY_LP: usize = 251;
pub const DLY_HP: usize = 252;
pub const DLY_SMEAR: usize = 253;
pub const DLY_MIX: usize = 254;
pub const REV_ON: usize = 255;
pub const REV_PRE: usize = 256;
pub const REV_DAMP: usize = 257;
pub const REV_SIZE: usize = 258;
pub const REV_WIDTH: usize = 259;
pub const REV_MIX: usize = 260;
pub const CMP_ON: usize = 261;
pub const CMP_THR: usize = 262;
pub const CMP_RATIO: usize = 263;
pub const CMP_ATT: usize = 264;
pub const CMP_REL: usize = 265;
pub const CMP_GAIN: usize = 266;
pub const PART_BASE: usize = 8;
pub const PART_STRIDE: usize = 7;
pub mod part {
    pub const ON: usize = 0;
    pub const LEVEL: usize = 1;
    pub const PAN: usize = 2;
    pub const ATT: usize = 3;
    pub const DEC: usize = 4;
    pub const SUS: usize = 5;
    pub const REL: usize = 6;
}
pub const OSC_BASE: usize = 22;
pub const OSC_STRIDE: usize = 12;
pub mod osc {
    pub const WAVE: usize = 0;
    pub const VOICES: usize = 1;
    pub const DETUNE: usize = 2;
    pub const STEREO: usize = 3;
    pub const OCT: usize = 4;
    pub const SEMI: usize = 5;
    pub const FINE: usize = 6;
    pub const PAN: usize = 7;
    pub const VOL: usize = 8;
    pub const PHASE: usize = 9;
    pub const RAND: usize = 10;
    pub const INV: usize = 11;
}
pub const FILT_BASE: usize = 70;
pub const FILT_STRIDE: usize = 11;
pub mod filt {
    pub const ON: usize = 0;
    pub const TYPE: usize = 1;
    pub const SLOPE: usize = 2;
    pub const CUTOFF: usize = 3;
    pub const RES: usize = 4;
    pub const DRIVE: usize = 5;
    pub const KEY: usize = 6;
    pub const IN_A: usize = 7;
    pub const IN_B: usize = 8;
    pub const VOL: usize = 9;
    pub const PAN: usize = 10;
}
/// Env 3 and Env 4 (the modulation envelopes), [`ENV_STRIDE`] apart.
pub const ENV_BASE: usize = 92;
pub const ENV_STRIDE: usize = 4;
pub mod env {
    pub const ATT: usize = 0;
    pub const DEC: usize = 1;
    pub const SUS: usize = 2;
    pub const REL: usize = 3;
}
pub const LFO_BASE: usize = 100;
pub const LFO_STRIDE: usize = 8;
pub mod lfo {
    pub const WAVE: usize = 0;
    pub const RATE: usize = 1;
    pub const SYNC: usize = 2;
    pub const DIV: usize = 3;
    pub const GAIN: usize = 4;
    pub const OFFSET: usize = 5;
    pub const PHASE: usize = 6;
    pub const RETRIG: usize = 7;
}
pub const MAT_BASE: usize = 116;
pub const MAT_STRIDE: usize = 3;
pub const MAT_SLOTS: usize = 16;
pub mod mat {
    pub const SRC: usize = 0;
    pub const DST: usize = 1;
    pub const AMT: usize = 2;
}
pub mod msrc {
    pub const OFF: usize = 0;
    pub const LFO_1: usize = 1;
    pub const LFO_2: usize = 2;
    pub const ENV_3: usize = 3;
    pub const ENV_4: usize = 4;
    pub const VELOCITY: usize = 5;
    pub const KEY: usize = 6;
    pub const MOD_WHEEL: usize = 7;
    pub const PITCH_WHEEL: usize = 8;
    pub const AFTERTOUCH: usize = 9;
    pub const RANDOM: usize = 10;
    pub const ARP_STEP: usize = 11;
    pub const AMP_ENV_A: usize = 12;
    pub const AMP_ENV_B: usize = 13;
}
pub mod mtgt {
    pub const OFF: usize = 0;
    pub const CUTOFF_1: usize = 1;
    pub const CUTOFF_2: usize = 2;
    pub const RESONANCE_1: usize = 3;
    pub const RESONANCE_2: usize = 4;
    pub const DRIVE_1: usize = 5;
    pub const DRIVE_2: usize = 6;
    pub const PITCH_ALL: usize = 7;
    pub const PITCH_PART_A: usize = 8;
    pub const PITCH_PART_B: usize = 9;
    pub const PITCH_A1: usize = 10;
    pub const PITCH_A2: usize = 11;
    pub const PITCH_B1: usize = 12;
    pub const PITCH_B2: usize = 13;
    pub const DETUNE_ALL: usize = 14;
    pub const LEVEL_A: usize = 15;
    pub const LEVEL_B: usize = 16;
    pub const PAN_A: usize = 17;
    pub const PAN_B: usize = 18;
    pub const AMP_ALL: usize = 19;
    pub const LFO_1_RATE: usize = 20;
    pub const LFO_2_RATE: usize = 21;
    pub const PHASE_A1: usize = 22;
    pub const PHASE_A2: usize = 23;
    pub const PHASE_B1: usize = 24;
    pub const PHASE_B2: usize = 25;
    pub const PHASE_PART_A: usize = 26;
    pub const PHASE_PART_B: usize = 27;
    pub const PHASE_ALL: usize = 28;
    pub const CUTOFF_ALL: usize = 29;
    pub const RESONANCE_ALL: usize = 30;
    pub const LEVEL_ALL: usize = 31;
    pub const PAN_ALL: usize = 32;
    pub const LFO_1_GAIN: usize = 33;
    pub const LFO_2_GAIN: usize = 34;
}
pub const ARP_STEP_BASE: usize = 171;
pub const ARP_TR_BASE: usize = 188;
pub const ARP_HOLD_BASE: usize = 204;

pub static SPECS: [Spec; COUNT] = [
    Spec {
        id: "master_vol",
        name: "Master Volume",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.7,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "master_tune",
        name: "Master Tune",
        kind: Kind::Lin(-100.0, 100.0),
        default: 0.0,
        fmt: Fmt::Cents,
    },
    Spec {
        id: "poly",
        name: "Polyphony",
        kind: Kind::Int(1, 16),
        default: 8.0,
        fmt: Fmt::Int,
    },
    Spec {
        id: "mono",
        name: "Voice Mode",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(POLYMONO_NAMES),
    },
    Spec {
        id: "porta",
        name: "Portamento",
        kind: Kind::Skew(0.0, 2.0, -2.5),
        default: 0.0,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "porta_mode",
        name: "Portamento Mode",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(PORTA_MODE_NAMES),
    },
    Spec {
        id: "bend",
        name: "Bend Range",
        kind: Kind::Int(1, 24),
        default: 2.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "vel_amp",
        name: "Velocity Sensitivity",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "pa_on",
        name: "Part A On",
        kind: Kind::Int(0, 1),
        default: 1.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "pa_level",
        name: "Part A Level",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "pa_pan",
        name: "Part A Pan",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pan,
    },
    Spec {
        id: "pa_att",
        name: "Part A Attack",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.003,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "pa_dec",
        name: "Part A Decay",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.6,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "pa_sus",
        name: "Part A Sustain",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "pa_rel",
        name: "Part A Release",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.25,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "pb_on",
        name: "Part B On",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "pb_level",
        name: "Part B Level",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "pb_pan",
        name: "Part B Pan",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pan,
    },
    Spec {
        id: "pb_att",
        name: "Part B Attack",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.003,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "pb_dec",
        name: "Part B Decay",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.6,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "pb_sus",
        name: "Part B Sustain",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "pb_rel",
        name: "Part B Release",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.25,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "o1_wave",
        name: "Osc A1 Wave",
        kind: Kind::Int(0, 7),
        default: 1.0,
        fmt: Fmt::Names(WAVE_NAMES),
    },
    Spec {
        id: "o1_voices",
        name: "Osc A1 Voices",
        kind: Kind::Int(0, 8),
        default: 1.0,
        fmt: Fmt::Int,
    },
    Spec {
        id: "o1_detune",
        name: "Osc A1 Detune",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.2,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o1_stereo",
        name: "Osc A1 Stereo",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o1_oct",
        name: "Osc A1 Octave",
        kind: Kind::Int(-3, 3),
        default: 0.0,
        fmt: Fmt::Oct,
    },
    Spec {
        id: "o1_semi",
        name: "Osc A1 Note",
        kind: Kind::Int(-12, 12),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "o1_fine",
        name: "Osc A1 Fine",
        kind: Kind::Lin(-100.0, 100.0),
        default: 0.0,
        fmt: Fmt::Cents,
    },
    Spec {
        id: "o1_pan",
        name: "Osc A1 Pan",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pan,
    },
    Spec {
        id: "o1_vol",
        name: "Osc A1 Volume",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.75,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o1_phase",
        name: "Osc A1 Phase",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o1_rand",
        name: "Osc A1 Phase Random",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o1_inv",
        name: "Osc A1 Invert",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "o2_wave",
        name: "Osc A2 Wave",
        kind: Kind::Int(0, 7),
        default: 1.0,
        fmt: Fmt::Names(WAVE_NAMES),
    },
    Spec {
        id: "o2_voices",
        name: "Osc A2 Voices",
        kind: Kind::Int(0, 8),
        default: 0.0,
        fmt: Fmt::Int,
    },
    Spec {
        id: "o2_detune",
        name: "Osc A2 Detune",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.2,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o2_stereo",
        name: "Osc A2 Stereo",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o2_oct",
        name: "Osc A2 Octave",
        kind: Kind::Int(-3, 3),
        default: 0.0,
        fmt: Fmt::Oct,
    },
    Spec {
        id: "o2_semi",
        name: "Osc A2 Note",
        kind: Kind::Int(-12, 12),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "o2_fine",
        name: "Osc A2 Fine",
        kind: Kind::Lin(-100.0, 100.0),
        default: 0.0,
        fmt: Fmt::Cents,
    },
    Spec {
        id: "o2_pan",
        name: "Osc A2 Pan",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pan,
    },
    Spec {
        id: "o2_vol",
        name: "Osc A2 Volume",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.75,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o2_phase",
        name: "Osc A2 Phase",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o2_rand",
        name: "Osc A2 Phase Random",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o2_inv",
        name: "Osc A2 Invert",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "o3_wave",
        name: "Osc B1 Wave",
        kind: Kind::Int(0, 7),
        default: 1.0,
        fmt: Fmt::Names(WAVE_NAMES),
    },
    Spec {
        id: "o3_voices",
        name: "Osc B1 Voices",
        kind: Kind::Int(0, 8),
        default: 1.0,
        fmt: Fmt::Int,
    },
    Spec {
        id: "o3_detune",
        name: "Osc B1 Detune",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.2,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o3_stereo",
        name: "Osc B1 Stereo",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o3_oct",
        name: "Osc B1 Octave",
        kind: Kind::Int(-3, 3),
        default: 0.0,
        fmt: Fmt::Oct,
    },
    Spec {
        id: "o3_semi",
        name: "Osc B1 Note",
        kind: Kind::Int(-12, 12),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "o3_fine",
        name: "Osc B1 Fine",
        kind: Kind::Lin(-100.0, 100.0),
        default: 0.0,
        fmt: Fmt::Cents,
    },
    Spec {
        id: "o3_pan",
        name: "Osc B1 Pan",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pan,
    },
    Spec {
        id: "o3_vol",
        name: "Osc B1 Volume",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.75,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o3_phase",
        name: "Osc B1 Phase",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o3_rand",
        name: "Osc B1 Phase Random",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o3_inv",
        name: "Osc B1 Invert",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "o4_wave",
        name: "Osc B2 Wave",
        kind: Kind::Int(0, 7),
        default: 1.0,
        fmt: Fmt::Names(WAVE_NAMES),
    },
    Spec {
        id: "o4_voices",
        name: "Osc B2 Voices",
        kind: Kind::Int(0, 8),
        default: 0.0,
        fmt: Fmt::Int,
    },
    Spec {
        id: "o4_detune",
        name: "Osc B2 Detune",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.2,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o4_stereo",
        name: "Osc B2 Stereo",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o4_oct",
        name: "Osc B2 Octave",
        kind: Kind::Int(-3, 3),
        default: 0.0,
        fmt: Fmt::Oct,
    },
    Spec {
        id: "o4_semi",
        name: "Osc B2 Note",
        kind: Kind::Int(-12, 12),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "o4_fine",
        name: "Osc B2 Fine",
        kind: Kind::Lin(-100.0, 100.0),
        default: 0.0,
        fmt: Fmt::Cents,
    },
    Spec {
        id: "o4_pan",
        name: "Osc B2 Pan",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pan,
    },
    Spec {
        id: "o4_vol",
        name: "Osc B2 Volume",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.75,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o4_phase",
        name: "Osc B2 Phase",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o4_rand",
        name: "Osc B2 Phase Random",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "o4_inv",
        name: "Osc B2 Invert",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "f1_on",
        name: "Filter 1 On",
        kind: Kind::Int(0, 1),
        default: 1.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "f1_type",
        name: "Filter 1 Type",
        kind: Kind::Int(0, 2),
        default: 0.0,
        fmt: Fmt::Names(FILTER_TYPE_NAMES),
    },
    Spec {
        id: "f1_slope",
        name: "Filter 1 Slope",
        kind: Kind::Int(0, 1),
        default: 1.0,
        fmt: Fmt::Names(SLOPE_NAMES),
    },
    Spec {
        id: "f1_cutoff",
        name: "Filter 1 Cutoff",
        kind: Kind::Skew(20.0, 20000.0, -2.5),
        default: 12000.0,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "f1_res",
        name: "Filter 1 Resonance",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.1,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f1_drive",
        name: "Filter 1 Drive",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f1_key",
        name: "Filter 1 Key Track",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f1_in_a",
        name: "Filter 1 Input A",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f1_in_b",
        name: "Filter 1 Input B",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f1_vol",
        name: "Filter 1 Volume",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f1_pan",
        name: "Filter 1 Pan",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pan,
    },
    Spec {
        id: "f2_on",
        name: "Filter 2 On",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "f2_type",
        name: "Filter 2 Type",
        kind: Kind::Int(0, 2),
        default: 0.0,
        fmt: Fmt::Names(FILTER_TYPE_NAMES),
    },
    Spec {
        id: "f2_slope",
        name: "Filter 2 Slope",
        kind: Kind::Int(0, 1),
        default: 1.0,
        fmt: Fmt::Names(SLOPE_NAMES),
    },
    Spec {
        id: "f2_cutoff",
        name: "Filter 2 Cutoff",
        kind: Kind::Skew(20.0, 20000.0, -2.5),
        default: 12000.0,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "f2_res",
        name: "Filter 2 Resonance",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.1,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f2_drive",
        name: "Filter 2 Drive",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f2_key",
        name: "Filter 2 Key Track",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f2_in_a",
        name: "Filter 2 Input A",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f2_in_b",
        name: "Filter 2 Input B",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f2_vol",
        name: "Filter 2 Volume",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "f2_pan",
        name: "Filter 2 Pan",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pan,
    },
    Spec {
        id: "e3_att",
        name: "Env 3 Attack",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.003,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "e3_dec",
        name: "Env 3 Decay",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.4,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "e3_sus",
        name: "Env 3 Sustain",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "e3_rel",
        name: "Env 3 Release",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.3,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "e4_att",
        name: "Env 4 Attack",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.003,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "e4_dec",
        name: "Env 4 Decay",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.4,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "e4_sus",
        name: "Env 4 Sustain",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "e4_rel",
        name: "Env 4 Release",
        kind: Kind::Skew(0.001, 10.0, -3.0),
        default: 0.3,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "l1_wave",
        name: "LFO 1 Wave",
        kind: Kind::Int(0, 10),
        default: 0.0,
        fmt: Fmt::Names(LFO_WAVE_NAMES),
    },
    Spec {
        id: "l1_rate",
        name: "LFO 1 Rate",
        kind: Kind::Skew(0.02, 40.0, -2.5),
        default: 2.0,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "l1_sync",
        name: "LFO 1 Sync",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "l1_div",
        name: "LFO 1 Division",
        kind: Kind::Int(0, 14),
        default: 5.0,
        fmt: Fmt::Names(DIV_NAMES),
    },
    Spec {
        id: "l1_gain",
        name: "LFO 1 Gain",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "l1_offset",
        name: "LFO 1 Offset",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "l1_phase",
        name: "LFO 1 Phase",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "l1_retrig",
        name: "LFO 1 Retrigger",
        kind: Kind::Int(0, 1),
        default: 1.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "l2_wave",
        name: "LFO 2 Wave",
        kind: Kind::Int(0, 10),
        default: 0.0,
        fmt: Fmt::Names(LFO_WAVE_NAMES),
    },
    Spec {
        id: "l2_rate",
        name: "LFO 2 Rate",
        kind: Kind::Skew(0.02, 40.0, -2.5),
        default: 2.0,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "l2_sync",
        name: "LFO 2 Sync",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "l2_div",
        name: "LFO 2 Division",
        kind: Kind::Int(0, 14),
        default: 5.0,
        fmt: Fmt::Names(DIV_NAMES),
    },
    Spec {
        id: "l2_gain",
        name: "LFO 2 Gain",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "l2_offset",
        name: "LFO 2 Offset",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "l2_phase",
        name: "LFO 2 Phase",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "l2_retrig",
        name: "LFO 2 Retrigger",
        kind: Kind::Int(0, 1),
        default: 1.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "m1_src",
        name: "Mod 1 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m1_dst",
        name: "Mod 1 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m1_amt",
        name: "Mod 1 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m2_src",
        name: "Mod 2 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m2_dst",
        name: "Mod 2 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m2_amt",
        name: "Mod 2 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m3_src",
        name: "Mod 3 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m3_dst",
        name: "Mod 3 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m3_amt",
        name: "Mod 3 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m4_src",
        name: "Mod 4 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m4_dst",
        name: "Mod 4 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m4_amt",
        name: "Mod 4 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m5_src",
        name: "Mod 5 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m5_dst",
        name: "Mod 5 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m5_amt",
        name: "Mod 5 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m6_src",
        name: "Mod 6 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m6_dst",
        name: "Mod 6 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m6_amt",
        name: "Mod 6 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m7_src",
        name: "Mod 7 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m7_dst",
        name: "Mod 7 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m7_amt",
        name: "Mod 7 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m8_src",
        name: "Mod 8 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m8_dst",
        name: "Mod 8 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m8_amt",
        name: "Mod 8 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m9_src",
        name: "Mod 9 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m9_dst",
        name: "Mod 9 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m9_amt",
        name: "Mod 9 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m10_src",
        name: "Mod 10 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m10_dst",
        name: "Mod 10 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m10_amt",
        name: "Mod 10 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m11_src",
        name: "Mod 11 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m11_dst",
        name: "Mod 11 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m11_amt",
        name: "Mod 11 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m12_src",
        name: "Mod 12 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m12_dst",
        name: "Mod 12 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m12_amt",
        name: "Mod 12 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m13_src",
        name: "Mod 13 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m13_dst",
        name: "Mod 13 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m13_amt",
        name: "Mod 13 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m14_src",
        name: "Mod 14 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m14_dst",
        name: "Mod 14 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m14_amt",
        name: "Mod 14 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m15_src",
        name: "Mod 15 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m15_dst",
        name: "Mod 15 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m15_amt",
        name: "Mod 15 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "m16_src",
        name: "Mod 16 Source",
        kind: Kind::Int(0, 13),
        default: 0.0,
        fmt: Fmt::Names(SOURCE_NAMES),
    },
    Spec {
        id: "m16_dst",
        name: "Mod 16 Target",
        kind: Kind::Int(0, 34),
        default: 0.0,
        fmt: Fmt::Names(TARGET_NAMES),
    },
    Spec {
        id: "m16_amt",
        name: "Mod 16 Amount",
        kind: Kind::Lin(-1.0, 1.0),
        default: 0.0,
        fmt: Fmt::Plain,
    },
    Spec {
        id: "arp_on",
        name: "Arp On",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_mode",
        name: "Arp Mode",
        kind: Kind::Int(0, 9),
        default: 0.0,
        fmt: Fmt::Names(ARP_MODE_NAMES),
    },
    Spec {
        id: "arp_div",
        name: "Arp Rate",
        kind: Kind::Int(0, 14),
        default: 11.0,
        fmt: Fmt::Names(DIV_NAMES),
    },
    Spec {
        id: "arp_oct",
        name: "Arp Octaves",
        kind: Kind::Int(1, 4),
        default: 1.0,
        fmt: Fmt::Int,
    },
    Spec {
        id: "arp_gate",
        name: "Arp Gate",
        kind: Kind::Lin(0.05, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_steps",
        name: "Arp Steps",
        kind: Kind::Int(1, 16),
        default: 16.0,
        fmt: Fmt::Int,
    },
    Spec {
        id: "arp_hold",
        name: "Arp Hold",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_s1",
        name: "Arp Step 1",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s2",
        name: "Arp Step 2",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s3",
        name: "Arp Step 3",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s4",
        name: "Arp Step 4",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s5",
        name: "Arp Step 5",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s6",
        name: "Arp Step 6",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s7",
        name: "Arp Step 7",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s8",
        name: "Arp Step 8",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s9",
        name: "Arp Step 9",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s10",
        name: "Arp Step 10",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s11",
        name: "Arp Step 11",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s12",
        name: "Arp Step 12",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s13",
        name: "Arp Step 13",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s14",
        name: "Arp Step 14",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s15",
        name: "Arp Step 15",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_s16",
        name: "Arp Step 16",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "arp_velmode",
        name: "Arp Velocity Source",
        kind: Kind::Int(0, 4),
        default: 2.0,
        fmt: Fmt::Names(ARP_VEL_NAMES),
    },
    Spec {
        id: "arp_t1",
        name: "Arp Transpose 1",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t2",
        name: "Arp Transpose 2",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t3",
        name: "Arp Transpose 3",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t4",
        name: "Arp Transpose 4",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t5",
        name: "Arp Transpose 5",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t6",
        name: "Arp Transpose 6",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t7",
        name: "Arp Transpose 7",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t8",
        name: "Arp Transpose 8",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t9",
        name: "Arp Transpose 9",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t10",
        name: "Arp Transpose 10",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t11",
        name: "Arp Transpose 11",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t12",
        name: "Arp Transpose 12",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t13",
        name: "Arp Transpose 13",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t14",
        name: "Arp Transpose 14",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t15",
        name: "Arp Transpose 15",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_t16",
        name: "Arp Transpose 16",
        kind: Kind::Int(-24, 24),
        default: 0.0,
        fmt: Fmt::Semi,
    },
    Spec {
        id: "arp_h1",
        name: "Arp Tie 1",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h2",
        name: "Arp Tie 2",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h3",
        name: "Arp Tie 3",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h4",
        name: "Arp Tie 4",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h5",
        name: "Arp Tie 5",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h6",
        name: "Arp Tie 6",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h7",
        name: "Arp Tie 7",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h8",
        name: "Arp Tie 8",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h9",
        name: "Arp Tie 9",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h10",
        name: "Arp Tie 10",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h11",
        name: "Arp Tie 11",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h12",
        name: "Arp Tie 12",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h13",
        name: "Arp Tie 13",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h14",
        name: "Arp Tie 14",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h15",
        name: "Arp Tie 15",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "arp_h16",
        name: "Arp Tie 16",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "dist_on",
        name: "Distortion On",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "dist_type",
        name: "Distortion Type",
        kind: Kind::Int(0, 4),
        default: 0.0,
        fmt: Fmt::Names(DIST_TYPE_NAMES),
    },
    Spec {
        id: "dist_drive",
        name: "Distortion Drive",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.3,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "dist_tone",
        name: "Distortion Tone",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.8,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "dist_mix",
        name: "Distortion Mix",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "phs_on",
        name: "Phaser On",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "phs_rate",
        name: "Phaser Rate",
        kind: Kind::Skew(0.02, 10.0, -2.5),
        default: 0.4,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "phs_depth",
        name: "Phaser Depth",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.6,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "phs_freq",
        name: "Phaser Frequency",
        kind: Kind::Skew(100.0, 8000.0, -2.5),
        default: 800.0,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "phs_fb",
        name: "Phaser Feedback",
        kind: Kind::Lin(0.0, 0.95),
        default: 0.3,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "phs_spread",
        name: "Phaser Spread",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "phs_mix",
        name: "Phaser Mix",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "cho_on",
        name: "Chorus On",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "cho_rate",
        name: "Chorus Rate",
        kind: Kind::Skew(0.05, 8.0, -2.5),
        default: 0.6,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "cho_depth",
        name: "Chorus Depth",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "cho_delay",
        name: "Chorus Delay",
        kind: Kind::Lin(1.0, 30.0),
        default: 8.0,
        fmt: Fmt::Ms,
    },
    Spec {
        id: "cho_fb",
        name: "Chorus Feedback",
        kind: Kind::Lin(0.0, 0.9),
        default: 0.2,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "cho_mix",
        name: "Chorus Mix",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.4,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "eq_on",
        name: "EQ On",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "eq_bass_f",
        name: "EQ Bass Frequency",
        kind: Kind::Skew(40.0, 800.0, -2.5),
        default: 150.0,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "eq_bass_g",
        name: "EQ Bass Gain",
        kind: Kind::Lin(-12.0, 12.0),
        default: 0.0,
        fmt: Fmt::Db,
    },
    Spec {
        id: "eq_treb_f",
        name: "EQ Treble Frequency",
        kind: Kind::Skew(1500.0, 16000.0, -2.5),
        default: 5000.0,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "eq_treb_g",
        name: "EQ Treble Gain",
        kind: Kind::Lin(-12.0, 12.0),
        default: 0.0,
        fmt: Fmt::Db,
    },
    Spec {
        id: "dly_on",
        name: "Delay On",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "dly_sync",
        name: "Delay Sync",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "dly_l",
        name: "Delay Time L",
        kind: Kind::Skew(0.005, 2.0, -2.5),
        default: 0.3,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "dly_r",
        name: "Delay Time R",
        kind: Kind::Skew(0.005, 2.0, -2.5),
        default: 0.45,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "dly_div_l",
        name: "Delay Division L",
        kind: Kind::Int(0, 14),
        default: 8.0,
        fmt: Fmt::Names(DIV_NAMES),
    },
    Spec {
        id: "dly_div_r",
        name: "Delay Division R",
        kind: Kind::Int(0, 14),
        default: 7.0,
        fmt: Fmt::Names(DIV_NAMES),
    },
    Spec {
        id: "dly_fb",
        name: "Delay Feedback",
        kind: Kind::Lin(0.0, 0.95),
        default: 0.35,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "dly_pp",
        name: "Delay Ping-Pong",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "dly_lp",
        name: "Delay Low-pass",
        kind: Kind::Skew(200.0, 20000.0, -2.5),
        default: 12000.0,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "dly_hp",
        name: "Delay High-pass",
        kind: Kind::Skew(20.0, 2000.0, -2.5),
        default: 40.0,
        fmt: Fmt::Hz,
    },
    Spec {
        id: "dly_smear",
        name: "Delay Smear",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "dly_mix",
        name: "Delay Mix",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.25,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "rev_on",
        name: "Reverb On",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "rev_pre",
        name: "Reverb Pre-delay",
        kind: Kind::Lin(0.0, 0.25),
        default: 0.01,
        fmt: Fmt::Sec,
    },
    Spec {
        id: "rev_damp",
        name: "Reverb Damp",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.5,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "rev_size",
        name: "Reverb Size",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.6,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "rev_width",
        name: "Reverb Width",
        kind: Kind::Lin(0.0, 1.0),
        default: 1.0,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "rev_mix",
        name: "Reverb Mix",
        kind: Kind::Lin(0.0, 1.0),
        default: 0.25,
        fmt: Fmt::Pct,
    },
    Spec {
        id: "cmp_on",
        name: "Compressor On",
        kind: Kind::Int(0, 1),
        default: 0.0,
        fmt: Fmt::Names(OFFON_NAMES),
    },
    Spec {
        id: "cmp_thr",
        name: "Compressor Threshold",
        kind: Kind::Lin(-40.0, 0.0),
        default: -12.0,
        fmt: Fmt::Db,
    },
    Spec {
        id: "cmp_ratio",
        name: "Compressor Ratio",
        kind: Kind::Skew(1.0, 20.0, -2.0),
        default: 3.0,
        fmt: Fmt::Ratio,
    },
    Spec {
        id: "cmp_att",
        name: "Compressor Attack",
        kind: Kind::Skew(0.1, 100.0, -2.0),
        default: 10.0,
        fmt: Fmt::Ms,
    },
    Spec {
        id: "cmp_rel",
        name: "Compressor Release",
        kind: Kind::Skew(10.0, 1000.0, -2.0),
        default: 150.0,
        fmt: Fmt::Ms,
    },
    Spec {
        id: "cmp_gain",
        name: "Compressor Makeup",
        kind: Kind::Lin(0.0, 12.0),
        default: 0.0,
        fmt: Fmt::Db,
    },
];

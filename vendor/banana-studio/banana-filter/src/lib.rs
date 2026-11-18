// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Banana Filter: Banana Studio's filter as a VST3 / CLAP plugin module. The plugin, its parameters and its editor are
//! `banana_small_fx::filter` (shared with the other small effects); this crate gives it its own module file.
//!
// spec: plugins/banana-filter.md, plugins/fruity-filter.md
#![allow(semicolon_in_expressions_from_non_local_macros)]

#[cfg(not(feature = "linked"))]
use nih_plug::prelude::*;

pub use banana_small_fx::filter::BananaFilter;

// Built as a plugin module: the C entry points hosts look for. Linked into Banana Studio (`linked` feature): only the
// symbol-free table, so several Banana plugins fit in one program (spec: plugins/plugin-formats.md).
#[cfg(not(feature = "linked"))]
nih_export_clap!(BananaFilter);
#[cfg(not(feature = "linked"))]
nih_export_vst3!(BananaFilter);
os_plugkit::linked_clap_entry!(BananaFilter);

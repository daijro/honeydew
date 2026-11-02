//! The bundled-content build tool. Recipes in `content/` plus the raw source
//! folders become `content-out/`: ustar pack parts named by hash, `index.json` (the app's `content::bundle::Index`),
//! `CREDITS.txt`, and a precompressed `.gz` beside each.

pub mod build;
pub mod dsp;
pub mod gates;
pub mod pack;
pub mod procedural;
pub mod recipe;
pub mod sfz;
pub mod stages;
pub mod verify;

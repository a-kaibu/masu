//! Windows Media decoders ported from FFmpeg 7.1 (LGPL-2.1-or-later): WMA 1 and 2 (`wma`),
//! MS-MPEG4/WMV1/WMV2 (`msmpeg4`), WMV3 Simple/Main/Complex (`vc1`), and the IntraX8 pictures of WMV2/WMV3 (`intrax8`).
//! sake's wmvcore readers and decoder DMOs use them (crates/sake/src/wmdec.rs). The hot pixel loops
//! have wasm SIMD128 versions (`simd`, built with `target_feature = "simd128"`), bit for bit the
//! scalar ones (wasm-test.sh).

// (Kani 0.68's compiler, nightly 2026-08-20, has these Rust 1.100 library features still
// unstable: formal/kani.sh builds this crate with it; masu-lint kani-features)
#![cfg_attr(kani, feature(vec_from_fn, bool_toggle, trim_prefix_suffix))]

pub mod bits;
pub mod hpel;
pub mod idct;
pub mod intrax8;
pub mod intrax8_tables;
pub mod msmpeg4;
pub mod msmpeg4_tables;
pub mod mdct;
pub mod num;
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub mod simd;
pub mod vc1;
pub mod vc1_tables;
pub mod vlc;
pub mod wma;
pub mod wma_tables;

#[cfg(test)]
mod pin_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod video_tests;

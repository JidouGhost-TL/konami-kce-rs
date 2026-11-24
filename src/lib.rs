//! # konami-kce-rs
//!
//! Pure Rust, zero-dependency parser, software synthesizer, and byte-exact golden-tested
//! emulator for the **Konami KCE (KCEH & KCEJ) GBA Sound Engines** on the Game Boy Advance.
//!
//! Covers **216 commercial GBA releases** across **6 distinct ARM7TDMI mixer revisions**:
//! - **Konami KCEH / Cross-Studio 32-Byte Soundbank Engine** (`172` ROMs: *Frogger's Adventures
//!   1 & 2*, *Frogger's Journey*, *Jurassic Park III: The DNA Factor*, *Konami Collector's
//!   Series*, *Motocross Maniacs Advance*, plus Eurocom's *Harry Potter 1 & 2*, Helixe/THQ's
//!   *Star Wars: The New Droid Army* & *Rugrats*, EA, Midway, and TDK).
//! - **Konami KCEJ Sound Driver** (`44` ROMs across 5 ARM7TDMI mixer revisions: *Yu-Gi-Oh!
//!   Eternal Duelist Soul*, *Worldwide Edition*, *WCT 2004*, *Ultimate Masters WCT 2006*,
//!   *Hunter X Hunter*, *Prince of Tennis 2002–2004*, *Hikaru no Go 1 & 2*, *Shaman King:
//!   Master of Spirits 2*, *Captain Tsubasa*, *Groove Adventure Rave 1 & 2*, *Mermaid Melody*).
//!
//! ## Features
//! - **Zero dependencies** in `[dependencies]`, `#![no_std]` compatible, `#![deny(unsafe_code)]`.
//! - **Dual synthesis pipelines**:
//!   - [`AccuracySynth`]: 100% bit-exact emulation of all 6 Konami KCE ARM7TDMI mixer revisions.
//!   - [`StudioSynth`]: 48,000 Hz `f32` band-limited Lanczos-3 windowed-sinc renderer with
//!     equal-power stereo panning.
//! - **Byte-exact `libmgba` ARM golden tested**: 1,920 / 1,920 output bytes verified strictly
//!   byte-for-byte (`assert_eq!`) against real ARM7TDMI execution across all 6 engine revisions.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod error;
pub mod freq;
pub mod sample;
pub mod synth;
pub mod version;
pub mod voice;

pub use error::{Error, Result};
pub use freq::{
    freq_to_step_q12, kceh_note_step_q12, kcej_sample_rate_hz, rate_from_timer_reload,
    step_q12_to_freq, GBA_CLOCK_HZ, KCEH_DEFAULT_MIX_RATE_HZ, KCEH_SEMITONE_TABLE_Q12,
    KCEJ_DEFAULT_MIX_RATE_HZ, KCEJ_TIMER0_RELOAD_19997, KCEJ_TIMER0_RELOAD_TABLE, Q12_ONE,
};
#[cfg(feature = "alloc")]
pub use sample::KonamiSample;
pub use sample::{
    KcehBankHeader, KcehSampleHeader, KcejSampleHeader, KonamiSampleView, GBA_ROM_BUS_BASE,
    KCEH_BANK_HEADER_SIZE, KCEH_SAMPLE_HEADER_SIZE, KCEJ_SAMPLE_HEADER_SIZE,
};
pub use synth::{AccuracySynth, StudioSynth};
pub use version::{KonamiKceFamily, KonamiKceVersion};
pub use voice::{KcehMixMode, KcehVoice, KcejVoice};

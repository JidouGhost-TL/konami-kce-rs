# konami-kce-rs

**Zero-dependency, pure Rust parser, software synthesizer, and byte-exact golden-tested emulator for the Konami KCE (KCEH & KCEJ) GBA Sound Engines (Game Boy Advance).**

[![Crates.io](https://img.shields.io/crates/v/konami-kce.svg)](https://crates.io/crates/konami-kce)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)
[![no_std](https://img.shields.io/badge/no__std-compatible-success.svg)](#features)

The **Konami KCE** custom DirectSound engines powered **216 commercial Game Boy Advance releases** across two major studio architectures:

- **Konami KCEH / Cross-Studio 32-Byte Soundbank Engine (172 ROMs)**: Shared bit-for-bit across Konami Computer Entertainment Hawaii (*Frogger's Adventures 1 & 2*, *Frogger's Journey*, *Jurassic Park III: The DNA Factor*, *Konami Collector's Series*, *Motocross Maniacs Advance*) and cross-studio partners including Eurocom (*Harry Potter 1 & 2*), Helixe/THQ (*Star Wars: The New Droid Army*, *Rugrats: Go Wild*), EA, Midway, and TDK.
- **Konami KCEJ Sound Driver (44 ROMs across 5 ARM7TDMI revisions)**: Powered Konami Computer Entertainment Japan's GBA catalog including *Yu-Gi-Oh! Eternal Duelist Soul*, *Yu-Gi-Oh! Worldwide Edition*, *Yu-Gi-Oh! WCT 2004*, *Yu-Gi-Oh! Ultimate Masters WCT 2006*, *Hunter X Hunter*, *Prince of Tennis 2002–2004*, *Hikaru no Go 1 & 2*, *Shaman King: Master of Spirits 2*, *Captain Tsubasa*, *Groove Adventure Rave 1 & 2*, and *Mermaid Melody*.

`konami-kce` provides a clean-room, reverse-engineered implementation designed for audio preservation, DAWs, PC source ports, and emulators.

> **Looking for a ready-to-use CLI?** You can scan GBA ROMs, extract WAV samples, and compile SoundFont 2.04 (.sf2) banks directly from the command line using our **[gbakit](https://github.com/JidouGhost-TL/gbakit)** toolkit (`gbakit audio scan`, `gbakit audio export`, `gbakit audio soundfont`).

---

## Features

- **Zero External Dependencies**: Pure Rust with no dependencies in `[dependencies]`, `#![deny(unsafe_code)]`.
- **`#![no_std]` Compatible**: Ready for embedded targets and WASM runtimes (`alloc` optional).
- **Native Sample Parsers**: Zero-allocation parsers for KCEH's 32-byte relocatable soundbank container (`KcehBankHeader`) + 16-byte sample headers (`KcehSampleHeader`) and KCEJ's 12-byte inline sample headers (`KcejSampleHeader`).
- **Dual Synthesis Engines**:
  - **`AccuracySynth`**: 100% bit-exact emulation of all **6** Konami KCE ARM7TDMI mixer revisions — including KCEH's 4 hardware kernels (Stereo/Mono × Linear/Nearest, forward & reverse playback, right-channel phase-invert surround, saturating `[-128, +127]` DMA8 clamp) and all 5 KCEJ ARM7TDMI revisions (`v1`–`v5`).
  - **`StudioSynth`**: Renders at 48,000 Hz, 32-bit floating-point precision with band-limited Lanczos-3 polyphase windowed-sinc interpolation and true equal-power stereo panning.
- **Byte-Exact `libmgba` ARM Golden Tested**: 1,920 / 1,920 output bytes verified strictly byte-for-byte (`assert_eq!`) across all 6 Konami KCE engine revisions.

---

## Supported Konami KCE Engine Revisions

Auditing all 3,377 GBA ROMs identifies **6 distinct ARM7TDMI mixer revisions** spanning **216 commercial titles**, all supported and byte-exact golden-tested via `KonamiKceVersion`:

| Revision | Corpus Share | Representative Titles | Scaling & Interpolation | Output Clamp | Channels |
| --- | ---: | --- | --- | --- | ---: |
| `KcehRelocBank32` | 172 ROMs | *Frogger's Adventures 1 & 2*, *Jurassic Park III*, *Harry Potter 1 & 2*, *Star Wars: The New Droid Army* | 12-bit linear or nearest, `(s * (vol + 1)) >> 8`, fwd/rev, phase-invert | Saturating `[-128, +127]` | 16 |
| `KcejV1Mono8` | 14 ROMs | *Yu-Gi-Oh! EDS*, *Yu-Gi-Oh! WWE*, *Hikaru no Go 1*, *Captain Tsubasa* | Nearest, `(s * (vol + 1)) >> 4`, `0x0800` 2x fast path | Direct wrapping `i8` | 3 (mono) |
| `KcejV2StereoLut` | 9 ROMs | *Hunter X Hunter*, *Prince of Tennis 2003*, *Hikaru no Go 2*, *Rave 2* | Nearest, 256-byte LUT `round(s * v / 15)`, fwd/rev | Truncating `i16 -> i8` | 8 |
| `KcejV3StereoAsr4Bias` | 5 ROMs | *Yu-Gi-Oh! WCT 2004*, *Mermaid Melody*, *Minna no Ouji-sama* | Nearest, `asrs #4; addmi #1`, center-pan fast path, fwd/rev | Truncating `i16 -> i8` | 12 |
| `KcejV4StereoAsr7` | 14 ROMs | *Shaman King: Master of Spirits 2*, *Prince of Tennis 2004*, *Mermaid Melody Live Start* | Nearest, `(s * vol) >> 7` (`asr #7`), center-pan fast path, fwd/rev | Truncating `i16 -> i8` | 12 |
| `KcejV5StereoAsr7Bias` | 3 ROMs | *Yu-Gi-Oh! Ultimate Masters WCT 2006* (US/EU/JP) | Nearest, `muls; addmi #0x80; asr #7`, center-pan fast path, fwd/rev | Truncating `i16 -> i8` | 12 |

---

## Quickstart

### Zero-Allocation Mixing (`#![no_std]`)

```rust
use konami_kce::{AccuracySynth, KcehMixMode, KcehVoice};

// Raw signed 8-bit PCM waveform (0 = silence)
let pcm: &[u8] = &[0, 32, 64, 96, 127, 96, 64, 32, 0, 224, 192, 160, 128, 160, 192, 224];

// KCEH stereo linear forward voice, looped, 1.0x pitch (0x1000)
let mut voice = KcehVoice::new_forward(0, pcm.len(), Some(0), pcm.len(), 0x1000, 0, 200, 140, false);
let mut acc_l = [0i16; 128];
let mut acc_r = [0i16; 128];
let mut dma_l = [0i8; 128];
let mut dma_r = [0i8; 128];

AccuracySynth::mix_kceh_voice(KcehMixMode::StereoLinear, pcm, &mut voice, &mut acc_l, &mut acc_r);
AccuracySynth::clamp_kceh_stereo_to_dma8(&mut acc_l, &mut acc_r, &mut dma_l, &mut dma_r);
```

---

## Golden Test Methodology

Every mixing path in `AccuracySynth` is verified strictly byte-for-byte (`assert_eq!`, zero tolerance) against real ARM7TDMI execution traces captured in headless `libmgba.so.0.10` using clean-room synthetic deterministic waveforms:

- **7 KCEH ARM vectors (1,536 bytes)**: Stereo Linear Forward, Stereo Linear Looped + Phase-Invert Surround, Stereo Linear Reverse, Stereo Nearest Forward, Mono Linear Forward, Mono Nearest Forward, and 4-Voice Saturated Clamping (`+127` / `-128`).
- **9 KCEJ ARM vectors (384 bytes)**: KCEJ v1 general resampled, `0x0800` 2x fast path, and 2-voice wrapping sum; KCEJ v2 forward & reverse 256-byte LUT; KCEJ v3 stereo & center-pan `asrs #4; addmi #1`; KCEJ v4 stereo `asr #7`; and KCEJ v5 stereo `muls; addmi #0x80; asr #7`.

---

## Benchmarks

```sh
cargo bench --bench bench_audio
```

| Benchmark | Throughput |
| --- | ---: |
| `AccuracySynth::kceh_stereo_linear + clamp` (128f) | ~804 MSamples/s |
| `AccuracySynth::kceh_stereo_nearest + clamp` (128f) | ~1.32 GSamples/s |
| `AccuracySynth::kcej_v1_mono8` (128f) | ~1.86 GSamples/s |
| `AccuracySynth::kcej_v4_stereo_asr7 + pack` (128f) | ~1.26 GSamples/s |
| `StudioSynth::render_voice_f32` (48 kHz Lanczos-3, 1,024f) | ~373 MSamples/s |

---

## Community & Support

Come discuss the wonders of GBA RE, modding, etc. — also the best way to report issues or bugs, or ask for help on your own project: https://discord.gg/khwxFSVNrS

---

## License

Licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

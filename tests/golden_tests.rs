//! Byte-exact `libmgba.so.0.10` ARM7TDMI Golden Integration Tests for `konami-kce`.
//!
//! Verifies all **16** clean-room ARM7TDMI execution golden vectors (**1,920 / 1,920 bytes**,
//! strict `assert_eq!` with zero tolerance) covering all **6** Konami KCE engine revisions
//! discovered across the 3,377-ROM GBA corpus:
//!
//! 1. `KonamiKceVersion::KcehRelocBank32` (7 vectors, 1,536 bytes):
//!    - Stereo Linear Forward resampled + ARM saturating clamp
//!    - Stereo Linear Forward + Looped + Phase-Invert Surround + ARM clamp
//!    - Stereo Linear Reverse + ARM clamp
//!    - Stereo Nearest Forward + ARM clamp
//!    - Mono Linear Forward + ARM mono clamp
//!    - Mono Nearest Forward + ARM mono clamp
//!    - 4-Voice Saturated Clamping (`+127` / `-128`) + ARM stereo clamp
//! 2. `KonamiKceVersion::KcejV1Mono8` (3 vectors, 192 bytes):
//!    - General resampled mono 8-bit
//!    - `0x0800` (0.5x) 2x fast path
//!    - 2-voice wrapping 8-bit sum
//! 3. `KonamiKceVersion::KcejV2StereoLut` (2 vectors, 64 bytes):
//!    - Forward stereo 256-byte LUT (`round(s * v / 15)`)
//!    - Reverse stereo 256-byte LUT
//! 4. `KonamiKceVersion::KcejV3StereoAsr4Bias` (2 vectors, 64 bytes):
//!    - Stereo `asrs #4; addmi #1` resampled
//!    - Center-pan fast path (`vol_l == vol_r`)
//! 5. `KonamiKceVersion::KcejV4StereoAsr7` (1 vector, 32 bytes):
//!    - Stereo `asr #7` resampled
//! 6. `KonamiKceVersion::KcejV5StereoAsr7Bias` (1 vector, 32 bytes):
//!    - Stereo `muls; addmi #0x80; asr #7` resampled

use konami_kce::{
    AccuracySynth, KcehMixMode, KcehVoice, KcejVoice, KonamiKceVersion, KonamiSampleView,
    StudioSynth,
};

const MANIFEST_JSON: &str = include_str!("golden/manifest.json");

// KCEH / Cross-Studio 32-byte soundbank goldens (7 vectors, 1,536 bytes)
const GOLDEN_KCEH_STEREO_LINEAR_FWD: &[u8] = include_bytes!("golden/kceh_stereo_linear_fwd.bin");
const GOLDEN_KCEH_STEREO_LINEAR_LOOP_INV: &[u8] =
    include_bytes!("golden/kceh_stereo_linear_loop_inv.bin");
const GOLDEN_KCEH_STEREO_LINEAR_REV: &[u8] = include_bytes!("golden/kceh_stereo_linear_rev.bin");
const GOLDEN_KCEH_STEREO_NEAREST_FWD: &[u8] = include_bytes!("golden/kceh_stereo_nearest_fwd.bin");
const GOLDEN_KCEH_MONO_LINEAR_FWD: &[u8] = include_bytes!("golden/kceh_mono_linear_fwd.bin");
const GOLDEN_KCEH_MONO_NEAREST_FWD: &[u8] = include_bytes!("golden/kceh_mono_nearest_fwd.bin");
const GOLDEN_KCEH_STEREO_4VOICE_CLAMPED: &[u8] =
    include_bytes!("golden/kceh_stereo_4voice_clamped.bin");

// KCEJ v1 goldens (3 vectors, 192 bytes)
const GOLDEN_KCEJ_V1_RESAMPLED: &[u8] = include_bytes!("golden/kcej_v1_resampled.bin");
const GOLDEN_KCEJ_V1_FAST_0800: &[u8] = include_bytes!("golden/kcej_v1_fast_0800.bin");
const GOLDEN_KCEJ_V1_2VOICE_WRAP: &[u8] = include_bytes!("golden/kcej_v1_2voice_wrap.bin");

// KCEJ v2..v5 goldens (6 vectors, 192 bytes)
const GOLDEN_KCEJ_V2_LUT_FWD: &[u8] = include_bytes!("golden/kcej_v2_stereo_lut_fwd.bin");
const GOLDEN_KCEJ_V2_LUT_REV: &[u8] = include_bytes!("golden/kcej_v2_stereo_lut_rev.bin");
const GOLDEN_KCEJ_V3_ASR4_STEREO: &[u8] = include_bytes!("golden/kcej_v3_asr4_bias_stereo.bin");
const GOLDEN_KCEJ_V3_ASR4_CENTER: &[u8] = include_bytes!("golden/kcej_v3_asr4_bias_center.bin");
const GOLDEN_KCEJ_V4_ASR7_STEREO: &[u8] = include_bytes!("golden/kcej_v4_asr7_stereo.bin");
const GOLDEN_KCEJ_V5_ASR7_BIAS_STEREO: &[u8] =
    include_bytes!("golden/kcej_v5_asr7_bias_stereo.bin");

/// Clean-room deterministic synthetic waveform A (identical to the `libmgba` ARM harness).
fn synth_wave_a() -> [u8; 256] {
    let mut out = [0u8; 256];
    for (i, slot) in out.iter_mut().enumerate() {
        let mut v = (((i as i32) * 19 + 7) % 256) - 128;
        if i % 5 == 0 {
            v = -v;
        }
        v = v.clamp(-128, 127);
        *slot = v as i8 as u8;
    }
    out
}

/// Clean-room deterministic synthetic waveform B (identical to the `libmgba` ARM harness).
fn synth_wave_b() -> [u8; 256] {
    let mut out = [0u8; 256];
    for (i, slot) in out.iter_mut().enumerate() {
        let v = (((i as i32) * 37 + 53) % 220) - 110;
        *slot = v as i8 as u8;
    }
    out
}

fn assert_kceh_stereo_exact(dma_l: &[i8], dma_r: &[i8], expected: &[u8], label: &str) {
    assert_eq!(expected.len(), dma_l.len() + dma_r.len());
    let n = dma_l.len();
    for i in 0..n {
        assert_eq!(
            dma_l[i], expected[i] as i8,
            "{label} LEFT mismatch at frame {i}"
        );
    }
    for i in 0..n {
        assert_eq!(
            dma_r[i],
            expected[n + i] as i8,
            "{label} RIGHT mismatch at frame {i}"
        );
    }
}

fn assert_kcej_stereo_exact(dma_l: &[i8], dma_r: &[i8], expected: &[u8], label: &str) {
    // KCEJ harness layout: expected[0..16] = Right (FIFO A), expected[16..32] = Left (FIFO B)
    assert_eq!(expected.len(), dma_l.len() + dma_r.len());
    let n = dma_l.len();
    for i in 0..n {
        assert_eq!(
            dma_r[i], expected[i] as i8,
            "{label} RIGHT mismatch at frame {i}"
        );
    }
    for i in 0..n {
        assert_eq!(
            dma_l[i],
            expected[n + i] as i8,
            "{label} LEFT mismatch at frame {i}"
        );
    }
}

#[test]
fn test_manifest_integrity() {
    assert!(!MANIFEST_JSON.is_empty());
    for name in [
        "kceh_stereo_linear_fwd.bin",
        "kceh_stereo_linear_loop_inv.bin",
        "kceh_stereo_linear_rev.bin",
        "kceh_stereo_nearest_fwd.bin",
        "kceh_mono_linear_fwd.bin",
        "kceh_mono_nearest_fwd.bin",
        "kceh_stereo_4voice_clamped.bin",
        "kcej_v1_resampled.bin",
        "kcej_v1_fast_0800.bin",
        "kcej_v1_2voice_wrap.bin",
        "kcej_v2_stereo_lut_fwd.bin",
        "kcej_v2_stereo_lut_rev.bin",
        "kcej_v3_asr4_bias_stereo.bin",
        "kcej_v3_asr4_bias_center.bin",
        "kcej_v4_asr7_stereo.bin",
        "kcej_v5_asr7_bias_stereo.bin",
    ] {
        assert!(MANIFEST_JSON.contains(name), "manifest missing {name}");
    }
}

// ============================================================================
// KCEH / Cross-Studio 32-Byte Soundbank Golden Tests (7 vectors, 1,536 bytes)
// ============================================================================

#[test]
fn test_golden_kceh_stereo_linear_fwd() {
    let wave_a = synth_wave_a();
    let mut voice = KcehVoice::new_forward(0, 256, None, 0, 0x1345, 0x0210, 180, 95, false);
    let mut acc_l = [0i16; 128];
    let mut acc_r = [0i16; 128];
    let mut dma_l = [0i8; 128];
    let mut dma_r = [0i8; 128];

    AccuracySynth::mix_kceh_voice(
        KcehMixMode::StereoLinear,
        &wave_a,
        &mut voice,
        &mut acc_l,
        &mut acc_r,
    );
    AccuracySynth::clamp_kceh_stereo_to_dma8(&mut acc_l, &mut acc_r, &mut dma_l, &mut dma_r);
    assert_kceh_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEH_STEREO_LINEAR_FWD,
        "kceh_stereo_linear_fwd",
    );
}

#[test]
fn test_golden_kceh_stereo_linear_loop_inv() {
    let wave_a = synth_wave_a();
    let mut voice = KcehVoice::new_forward(200, 256, Some(64), 192, 0x1789, 0x0ABC, 200, 150, true);
    let mut acc_l = [0i16; 128];
    let mut acc_r = [0i16; 128];
    let mut dma_l = [0i8; 128];
    let mut dma_r = [0i8; 128];

    AccuracySynth::mix_kceh_stereo_linear(&wave_a, &mut voice, &mut acc_l, &mut acc_r);
    AccuracySynth::clamp_kceh_stereo_to_dma8(&mut acc_l, &mut acc_r, &mut dma_l, &mut dma_r);
    assert_kceh_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEH_STEREO_LINEAR_LOOP_INV,
        "kceh_stereo_linear_loop_inv",
    );
}

#[test]
fn test_golden_kceh_stereo_linear_rev() {
    let wave_a = synth_wave_a();
    let mut voice = KcehVoice::new_reverse(240, 16, None, 0, 0x0C80, 0x0400, 220, 140, false);
    let mut acc_l = [0i16; 128];
    let mut acc_r = [0i16; 128];
    let mut dma_l = [0i8; 128];
    let mut dma_r = [0i8; 128];

    AccuracySynth::mix_kceh_stereo_linear(&wave_a, &mut voice, &mut acc_l, &mut acc_r);
    AccuracySynth::clamp_kceh_stereo_to_dma8(&mut acc_l, &mut acc_r, &mut dma_l, &mut dma_r);
    assert_kceh_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEH_STEREO_LINEAR_REV,
        "kceh_stereo_linear_rev",
    );
}

#[test]
fn test_golden_kceh_stereo_nearest_fwd() {
    let wave_a = synth_wave_a();
    let mut voice = KcehVoice::new_forward(0, 256, None, 0, 0x1567, 0x0100, 255, 128, false);
    let mut acc_l = [0i16; 128];
    let mut acc_r = [0i16; 128];
    let mut dma_l = [0i8; 128];
    let mut dma_r = [0i8; 128];

    AccuracySynth::mix_kceh_stereo_nearest(&wave_a, &mut voice, &mut acc_l, &mut acc_r);
    AccuracySynth::clamp_kceh_stereo_to_dma8(&mut acc_l, &mut acc_r, &mut dma_l, &mut dma_r);
    assert_kceh_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEH_STEREO_NEAREST_FWD,
        "kceh_stereo_nearest_fwd",
    );
}

#[test]
fn test_golden_kceh_mono_linear_fwd() {
    let wave_a = synth_wave_a();
    let mut voice = KcehVoice::new_forward(0, 256, None, 0, 0x1123, 0x0333, 180, 100, false);
    let mut acc = [0i16; 128];
    let mut dma = [0i8; 128];

    AccuracySynth::mix_kceh_mono_linear(&wave_a, &mut voice, &mut acc);
    AccuracySynth::clamp_kceh_mono_to_dma8(&mut acc, &mut dma);
    for i in 0..128 {
        assert_eq!(
            dma[i], GOLDEN_KCEH_MONO_LINEAR_FWD[i] as i8,
            "kceh_mono_linear_fwd mismatch at frame {i}"
        );
    }
}

#[test]
fn test_golden_kceh_mono_nearest_fwd() {
    let wave_a = synth_wave_a();
    let mut voice = KcehVoice::new_forward(0, 256, None, 0, 0x0E40, 0x0700, 200, 60, false);
    let mut acc = [0i16; 128];
    let mut dma = [0i8; 128];

    AccuracySynth::mix_kceh_mono_nearest(&wave_a, &mut voice, &mut acc);
    AccuracySynth::clamp_kceh_mono_to_dma8(&mut acc, &mut dma);
    for i in 0..128 {
        assert_eq!(
            dma[i], GOLDEN_KCEH_MONO_NEAREST_FWD[i] as i8,
            "kceh_mono_nearest_fwd mismatch at frame {i}"
        );
    }
}

#[test]
fn test_golden_kceh_stereo_4voice_clamped() {
    let wave_a = synth_wave_a();
    let wave_b = synth_wave_b();
    let mut acc_l = [0i16; 128];
    let mut acc_r = [0i16; 128];
    let mut dma_l = [0i8; 128];
    let mut dma_r = [0i8; 128];

    let mut v0 = KcehVoice::new_forward(0, 256, Some(0), 256, 0x1000, 0, 255, 255, false);
    let mut v1 = KcehVoice::new_forward(10, 256, Some(0), 256, 0x1200, 0x400, 255, 255, false);
    let mut v2 = KcehVoice::new_forward(50, 256, Some(0), 256, 0x0E00, 0x800, 255, 128, false);
    let mut v3 = KcehVoice::new_forward(90, 256, Some(0), 256, 0x1400, 0x100, 128, 255, true);

    AccuracySynth::mix_kceh_stereo_linear(&wave_a, &mut v0, &mut acc_l, &mut acc_r);
    AccuracySynth::mix_kceh_stereo_linear(&wave_b, &mut v1, &mut acc_l, &mut acc_r);
    AccuracySynth::mix_kceh_stereo_linear(&wave_a, &mut v2, &mut acc_l, &mut acc_r);
    AccuracySynth::mix_kceh_stereo_linear(&wave_b, &mut v3, &mut acc_l, &mut acc_r);
    AccuracySynth::clamp_kceh_stereo_to_dma8(&mut acc_l, &mut acc_r, &mut dma_l, &mut dma_r);

    assert_kceh_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEH_STEREO_4VOICE_CLAMPED,
        "kceh_stereo_4voice_clamped",
    );
}

// ============================================================================
// KCEJ v1 (Mono 8-bit Direct Wrapping) Golden Tests (3 vectors, 192 bytes)
// ============================================================================

#[test]
fn test_golden_kcej_v1_resampled() {
    let wave_a = synth_wave_a();
    let mut voice = KcejVoice::new(0, 256, 0x1345, 0x0210, 11, 11, None, 256, false);
    let mut out = [0i8; 64];
    AccuracySynth::mix_kcej_v1_mono8(&wave_a, &mut voice, &mut out);
    for i in 0..64 {
        assert_eq!(
            out[i], GOLDEN_KCEJ_V1_RESAMPLED[i] as i8,
            "kcej_v1_resampled mismatch at frame {i}"
        );
    }
}

#[test]
fn test_golden_kcej_v1_fast_0800() {
    let wave_a = synth_wave_a();
    let mut voice = KcejVoice::new(0, 256, 0x0800, 0x0000, 15, 15, None, 256, false);
    let mut out = [0i8; 64];
    AccuracySynth::mix_kcej_v1_mono8(&wave_a, &mut voice, &mut out);
    for i in 0..64 {
        assert_eq!(
            out[i], GOLDEN_KCEJ_V1_FAST_0800[i] as i8,
            "kcej_v1_fast_0800 mismatch at frame {i}"
        );
    }
}

#[test]
fn test_golden_kcej_v1_2voice_wrap() {
    let wave_a = synth_wave_a();
    let wave_b = synth_wave_b();
    let mut v0 = KcejVoice::new(0, 256, 0x1000, 0, 15, 15, None, 256, false);
    let mut v1 = KcejVoice::new(0, 256, 0x1400, 0x0300, 15, 15, None, 256, false);
    let mut out = [0i8; 64];
    AccuracySynth::mix_kcej_v1_mono8(&wave_a, &mut v0, &mut out);
    AccuracySynth::mix_kcej_v1_mono8(&wave_b, &mut v1, &mut out);
    for i in 0..64 {
        assert_eq!(
            out[i], GOLDEN_KCEJ_V1_2VOICE_WRAP[i] as i8,
            "kcej_v1_2voice_wrap mismatch at frame {i}"
        );
    }
}

// ============================================================================
// KCEJ v2..v5 Stereo Golden Tests (6 vectors, 192 bytes)
// ============================================================================

#[test]
fn test_golden_kcej_v2_stereo_lut_fwd() {
    let wave_a = synth_wave_a();
    let mut voice = KcejVoice::new(0, 256, 0x1234, 0x0321, 7, 11, None, 256, false);
    let mut acc_l = [0i16; 16];
    let mut acc_r = [0i16; 16];
    let mut dma_l = [0i8; 16];
    let mut dma_r = [0i8; 16];

    AccuracySynth::mix_kcej_stereo_voice(
        KonamiKceVersion::KcejV2StereoLut,
        &wave_a,
        &mut voice,
        &mut acc_l,
        &mut acc_r,
    );
    AccuracySynth::pack_kcej_stereo_to_dma8(&acc_l, &acc_r, &mut dma_l, &mut dma_r);
    assert_kcej_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEJ_V2_LUT_FWD,
        "kcej_v2_stereo_lut_fwd",
    );
}

#[test]
fn test_golden_kcej_v2_stereo_lut_rev() {
    let wave_a = synth_wave_a();
    let mut voice = KcejVoice::new(200, 200, 0x0D80, 0x0100, 9, 15, None, 256, true);
    let mut acc_l = [0i16; 16];
    let mut acc_r = [0i16; 16];
    let mut dma_l = [0i8; 16];
    let mut dma_r = [0i8; 16];

    AccuracySynth::mix_kcej_stereo_voice(
        KonamiKceVersion::KcejV2StereoLut,
        &wave_a,
        &mut voice,
        &mut acc_l,
        &mut acc_r,
    );
    AccuracySynth::pack_kcej_stereo_to_dma8(&acc_l, &acc_r, &mut dma_l, &mut dma_r);
    assert_kcej_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEJ_V2_LUT_REV,
        "kcej_v2_stereo_lut_rev",
    );
}

#[test]
fn test_golden_kcej_v3_asr4_bias_stereo() {
    let wave_a = synth_wave_a();
    let mut voice = KcejVoice::new(0, 256, 0x1456, 0x0111, 9, 13, None, 256, false);
    let mut acc_l = [0i16; 16];
    let mut acc_r = [0i16; 16];
    let mut dma_l = [0i8; 16];
    let mut dma_r = [0i8; 16];

    AccuracySynth::mix_kcej_stereo_voice(
        KonamiKceVersion::KcejV3StereoAsr4Bias,
        &wave_a,
        &mut voice,
        &mut acc_l,
        &mut acc_r,
    );
    AccuracySynth::pack_kcej_stereo_to_dma8(&acc_l, &acc_r, &mut dma_l, &mut dma_r);
    assert_kcej_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEJ_V3_ASR4_STEREO,
        "kcej_v3_asr4_bias_stereo",
    );
}

#[test]
fn test_golden_kcej_v3_asr4_bias_center() {
    let wave_a = synth_wave_a();
    let mut voice = KcejVoice::new(0, 256, 0x0E20, 0x0400, 12, 12, None, 256, false);
    let mut acc_l = [0i16; 16];
    let mut acc_r = [0i16; 16];
    let mut dma_l = [0i8; 16];
    let mut dma_r = [0i8; 16];

    AccuracySynth::mix_kcej_stereo_voice(
        KonamiKceVersion::KcejV3StereoAsr4Bias,
        &wave_a,
        &mut voice,
        &mut acc_l,
        &mut acc_r,
    );
    AccuracySynth::pack_kcej_stereo_to_dma8(&acc_l, &acc_r, &mut dma_l, &mut dma_r);
    assert_kcej_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEJ_V3_ASR4_CENTER,
        "kcej_v3_asr4_bias_center",
    );
}

#[test]
fn test_golden_kcej_v4_asr7_stereo() {
    let wave_a = synth_wave_a();
    let mut voice = KcejVoice::new(0, 256, 0x1567, 0x0222, 65, 110, None, 256, false);
    let mut acc_l = [0i16; 16];
    let mut acc_r = [0i16; 16];
    let mut dma_l = [0i8; 16];
    let mut dma_r = [0i8; 16];

    AccuracySynth::mix_kcej_stereo_voice(
        KonamiKceVersion::KcejV4StereoAsr7,
        &wave_a,
        &mut voice,
        &mut acc_l,
        &mut acc_r,
    );
    AccuracySynth::pack_kcej_stereo_to_dma8(&acc_l, &acc_r, &mut dma_l, &mut dma_r);
    assert_kcej_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEJ_V4_ASR7_STEREO,
        "kcej_v4_asr7_stereo",
    );
}

#[test]
fn test_golden_kcej_v5_asr7_bias_stereo() {
    let wave_a = synth_wave_a();
    let mut voice = KcejVoice::new(0, 256, 0x1678, 0x0333, 73, 115, None, 256, false);
    let mut acc_l = [0i16; 16];
    let mut acc_r = [0i16; 16];
    let mut dma_l = [0i8; 16];
    let mut dma_r = [0i8; 16];

    AccuracySynth::mix_kcej_stereo_voice(
        KonamiKceVersion::KcejV5StereoAsr7Bias,
        &wave_a,
        &mut voice,
        &mut acc_l,
        &mut acc_r,
    );
    AccuracySynth::pack_kcej_stereo_to_dma8(&acc_l, &acc_r, &mut dma_l, &mut dma_r);
    assert_kcej_stereo_exact(
        &dma_l,
        &dma_r,
        GOLDEN_KCEJ_V5_ASR7_BIAS_STEREO,
        "kcej_v5_asr7_bias_stereo",
    );
}

// ============================================================================
// StudioSynth (48,000 Hz Lanczos-3 + Equal-Power Panning) Verification
// ============================================================================

#[test]
fn test_studio_synth_equal_power_panning_and_loop() {
    let wave_a = synth_wave_a();
    let sample = KonamiSampleView::from_pcm_bytes(0, 13_379, 60, Some(64), &wave_a);
    let studio = StudioSynth::default();

    let mut out_l = [0.0f32; 512];
    let mut out_r = [0.0f32; 512];
    let mut pos = 0.0f64;
    studio.render_voice_f32(&sample, &mut pos, 13_379.0, 1.0, 0.0, &mut out_l, &mut out_r);
    assert!(pos > 100.0);
    // Center pan: L == R to f32 precision
    for i in 0..512 {
        assert!((out_l[i] - out_r[i]).abs() < 1e-6);
    }

    // Hard left: R == 0
    let mut hl = [0.0f32; 128];
    let mut hr = [0.0f32; 128];
    let mut pos_l = 0.0f64;
    studio.render_voice_f32(&sample, &mut pos_l, 13_379.0, 1.0, -1.0, &mut hl, &mut hr);
    assert!(hr.iter().all(|&x| x.abs() < 1e-6));
    assert!(hl.iter().any(|&x| x.abs() > 0.05));
}

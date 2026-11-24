//! Dual synthesis pipelines for Konami KCE (KCEH & KCEJ) GBA audio:
//!
//! 1. [`AccuracySynth`]: 100% bit-exact emulation of all 6 Konami KCE ARM7TDMI mixer
//!    revisions discovered across the 3,377-ROM GBA corpus:
//!    - **KCEH / Cross-Studio 32-byte soundbank** (4 ARM kernels: Stereo/Mono × Linear/Nearest,
//!      forward & reverse playback, phase-invert surround, and saturating `[-128, +127]` DMA8 clamp).
//!    - **KCEJ v1** (`KcejV1Mono8`): mono direct wrapping `i8` mixer + `0x0800` 2x fast path.
//!    - **KCEJ v2** (`KcejV2StereoLut`): stereo 16-bit mixer with exact `round(s * v / 15)` LUT.
//!    - **KCEJ v3** (`KcejV3StereoAsr4Bias`): stereo `asrs #4; addmi #1` negative-bias mixer.
//!    - **KCEJ v4** (`KcejV4StereoAsr7`): stereo `asr #7` mixer.
//!    - **KCEJ v5** (`KcejV5StereoAsr7Bias`): stereo `muls; addmi #0x80; asr #7` mixer.
//! 2. [`StudioSynth`]: 48,000 Hz `f32` band-limited Lanczos-3 polyphase windowed-sinc
//!    renderer with equal-power stereo panning.

use crate::sample::KonamiSampleView;
use crate::version::KonamiKceVersion;
use crate::voice::{KcehMixMode, KcehVoice, KcejVoice};

/// Bit-exact software emulation of all 6 Konami KCE ARM7TDMI mixer revisions.
pub struct AccuracySynth;

impl AccuracySynth {
    // ========================================================================
    // Konami KCEH / Cross-Studio 32-Byte Soundbank ARM Mixer (172 ROMs)
    // ========================================================================

    /// Mix a KCEH voice into separate Left/Right `i16` accumulators using the exact
    /// ARM7TDMI **Stereo Linear** kernel (`0x08043C08`).
    ///
    /// Returns `true` if a one-shot voice reached its end during this block.
    pub fn mix_kceh_stereo_linear(
        pcm_bytes: &[u8],
        voice: &mut KcehVoice,
        acc_l: &mut [i16],
        acc_r: &mut [i16],
    ) -> bool {
        if !voice.active || pcm_bytes.is_empty() {
            return !voice.active;
        }
        let frames = acc_l.len().min(acc_r.len());
        if frames == 0 {
            return false;
        }

        let vol_l: i32 = if voice.vol_l != 0 {
            (voice.vol_l as i32) + 1
        } else {
            0
        };
        let mut vol_r: i32 = if voice.vol_r != 0 {
            (voice.vol_r as i32) + 1
        } else {
            0
        };
        if voice.phase_invert {
            vol_r = -vol_r;
        }

        let step = voice.step as u32;
        let mut frac = voice.frac as u32;
        let mut cur = voice.cur;
        let mut end = voice.end;

        if end >= cur {
            // Forward playback
            for i in 0..frames {
                if cur >= pcm_bytes.len() {
                    voice.active = false;
                    return true;
                }
                let next = cur + 1;
                let next_idx = if next >= end {
                    voice.loop_start.unwrap_or(cur)
                } else {
                    next
                };
                let s0 = pcm_bytes[cur] as i8 as i32;
                let s1 = pcm_bytes[next_idx.min(pcm_bytes.len() - 1)] as i8 as i32;
                let f = (frac & 0x0FFF) as i32;
                let s = s0 + (((s1 - s0) * f) >> 12);

                acc_l[i] = acc_l[i].wrapping_add(((s * vol_l) >> 8) as i16);
                acc_r[i] = acc_r[i].wrapping_add(((s * vol_r) >> 8) as i16);

                frac += step;
                if frac >= 0x1000 {
                    cur += (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur >= end {
                        if let Some(ls) = voice.loop_start {
                            cur = ls + (cur - end);
                            end = ls + voice.loop_len;
                            if cur >= end {
                                cur = ls;
                            }
                        } else {
                            voice.cur = cur;
                            voice.end = end;
                            voice.frac = frac as u16;
                            voice.active = false;
                            return true;
                        }
                    }
                }
            }
        } else {
            // Reverse playback (end < cur)
            for i in 0..frames {
                if cur >= pcm_bytes.len() {
                    voice.active = false;
                    return true;
                }
                let next_idx = if cur == 0 || (cur - 1) <= end {
                    voice.loop_start.unwrap_or(cur)
                } else {
                    cur - 1
                };
                let s0 = pcm_bytes[cur] as i8 as i32;
                let s1 = pcm_bytes[next_idx.min(pcm_bytes.len() - 1)] as i8 as i32;
                let f = (frac & 0x0FFF) as i32;
                let s = s0 + (((s1 - s0) * f) >> 12);

                acc_l[i] = acc_l[i].wrapping_add(((s * vol_l) >> 8) as i16);
                acc_r[i] = acc_r[i].wrapping_add(((s * vol_r) >> 8) as i16);

                frac += step;
                if frac >= 0x1000 {
                    let adv = (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur <= end.saturating_add(adv) {
                        if let Some(ls) = voice.loop_start {
                            let under = end.saturating_add(adv) - cur;
                            cur = ls.saturating_sub(under);
                            end = ls.saturating_sub(voice.loop_len);
                            if cur <= end {
                                cur = ls;
                            }
                        } else {
                            voice.cur = cur.saturating_sub(adv);
                            voice.end = end;
                            voice.frac = frac as u16;
                            voice.active = false;
                            return true;
                        }
                    } else {
                        cur -= adv;
                    }
                }
            }
        }

        voice.cur = cur;
        voice.end = end;
        voice.frac = frac as u16;
        false
    }

    /// Mix a KCEH voice into separate Left/Right `i16` accumulators using the exact
    /// ARM7TDMI **Stereo Nearest-Neighbor** kernel (`0x08043DE8`).
    pub fn mix_kceh_stereo_nearest(
        pcm_bytes: &[u8],
        voice: &mut KcehVoice,
        acc_l: &mut [i16],
        acc_r: &mut [i16],
    ) -> bool {
        if !voice.active || pcm_bytes.is_empty() {
            return !voice.active;
        }
        let frames = acc_l.len().min(acc_r.len());
        if frames == 0 {
            return false;
        }

        let vol_l: i32 = if voice.vol_l != 0 {
            (voice.vol_l as i32) + 1
        } else {
            0
        };
        let mut vol_r: i32 = if voice.vol_r != 0 {
            (voice.vol_r as i32) + 1
        } else {
            0
        };
        if voice.phase_invert {
            vol_r = -vol_r;
        }

        let step = voice.step as u32;
        let mut frac = voice.frac as u32;
        let mut cur = voice.cur;
        let mut end = voice.end;

        if end >= cur {
            for i in 0..frames {
                if cur >= pcm_bytes.len() {
                    voice.active = false;
                    return true;
                }
                let s = pcm_bytes[cur] as i8 as i32;
                acc_l[i] = acc_l[i].wrapping_add(((s * vol_l) >> 8) as i16);
                acc_r[i] = acc_r[i].wrapping_add(((s * vol_r) >> 8) as i16);

                frac += step;
                if frac >= 0x1000 {
                    cur += (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur >= end {
                        if let Some(ls) = voice.loop_start {
                            cur = ls + (cur - end);
                            end = ls + voice.loop_len;
                            if cur >= end {
                                cur = ls;
                            }
                        } else {
                            voice.cur = cur;
                            voice.end = end;
                            voice.frac = frac as u16;
                            voice.active = false;
                            return true;
                        }
                    }
                }
            }
        } else {
            for i in 0..frames {
                if cur >= pcm_bytes.len() {
                    voice.active = false;
                    return true;
                }
                let s = pcm_bytes[cur] as i8 as i32;
                acc_l[i] = acc_l[i].wrapping_add(((s * vol_l) >> 8) as i16);
                acc_r[i] = acc_r[i].wrapping_add(((s * vol_r) >> 8) as i16);

                frac += step;
                if frac >= 0x1000 {
                    let adv = (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur <= end.saturating_add(adv) {
                        if let Some(ls) = voice.loop_start {
                            let under = end.saturating_add(adv) - cur;
                            cur = ls.saturating_sub(under);
                            end = ls.saturating_sub(voice.loop_len);
                            if cur <= end {
                                cur = ls;
                            }
                        } else {
                            voice.cur = cur.saturating_sub(adv);
                            voice.end = end;
                            voice.frac = frac as u16;
                            voice.active = false;
                            return true;
                        }
                    } else {
                        cur -= adv;
                    }
                }
            }
        }

        voice.cur = cur;
        voice.end = end;
        voice.frac = frac as u16;
        false
    }

    /// Mix a KCEH voice into a mono `i16` accumulator using the exact ARM7TDMI
    /// **Mono Linear** kernel (`0x08043F74`).
    pub fn mix_kceh_mono_linear(
        pcm_bytes: &[u8],
        voice: &mut KcehVoice,
        acc: &mut [i16],
    ) -> bool {
        if !voice.active || pcm_bytes.is_empty() || acc.is_empty() {
            return !voice.active;
        }
        let mut vol = (voice.vol_l as i32) + (voice.vol_r as i32);
        if vol != 0 {
            vol += 2;
        }
        vol >>= 1;

        let step = voice.step as u32;
        let mut frac = voice.frac as u32;
        let mut cur = voice.cur;
        let mut end = voice.end;

        if end >= cur {
            for slot in acc.iter_mut() {
                if cur >= pcm_bytes.len() {
                    voice.active = false;
                    return true;
                }
                let next = cur + 1;
                let next_idx = if next >= end {
                    voice.loop_start.unwrap_or(cur)
                } else {
                    next
                };
                let s0 = pcm_bytes[cur] as i8 as i32;
                let s1 = pcm_bytes[next_idx.min(pcm_bytes.len() - 1)] as i8 as i32;
                let f = (frac & 0x0FFF) as i32;
                let s = s0 + (((s1 - s0) * f) >> 12);
                *slot = slot.wrapping_add(((s * vol) >> 8) as i16);

                frac += step;
                if frac >= 0x1000 {
                    cur += (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur >= end {
                        if let Some(ls) = voice.loop_start {
                            cur = ls + (cur - end);
                            end = ls + voice.loop_len;
                            if cur >= end {
                                cur = ls;
                            }
                        } else {
                            voice.cur = cur;
                            voice.end = end;
                            voice.frac = frac as u16;
                            voice.active = false;
                            return true;
                        }
                    }
                }
            }
        } else {
            for slot in acc.iter_mut() {
                if cur >= pcm_bytes.len() {
                    voice.active = false;
                    return true;
                }
                let next_idx = if cur == 0 || (cur - 1) <= end {
                    voice.loop_start.unwrap_or(cur)
                } else {
                    cur - 1
                };
                let s0 = pcm_bytes[cur] as i8 as i32;
                let s1 = pcm_bytes[next_idx.min(pcm_bytes.len() - 1)] as i8 as i32;
                let f = (frac & 0x0FFF) as i32;
                let s = s0 + (((s1 - s0) * f) >> 12);
                *slot = slot.wrapping_add(((s * vol) >> 8) as i16);

                frac += step;
                if frac >= 0x1000 {
                    let adv = (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur <= end.saturating_add(adv) {
                        if let Some(ls) = voice.loop_start {
                            let under = end.saturating_add(adv) - cur;
                            cur = ls.saturating_sub(under);
                            end = ls.saturating_sub(voice.loop_len);
                            if cur <= end {
                                cur = ls;
                            }
                        } else {
                            voice.cur = cur.saturating_sub(adv);
                            voice.end = end;
                            voice.frac = frac as u16;
                            voice.active = false;
                            return true;
                        }
                    } else {
                        cur -= adv;
                    }
                }
            }
        }

        voice.cur = cur;
        voice.end = end;
        voice.frac = frac as u16;
        false
    }

    /// Mix a KCEH voice into a mono `i16` accumulator using the exact ARM7TDMI
    /// **Mono Nearest-Neighbor** kernel (`0x08044128`).
    pub fn mix_kceh_mono_nearest(
        pcm_bytes: &[u8],
        voice: &mut KcehVoice,
        acc: &mut [i16],
    ) -> bool {
        if !voice.active || pcm_bytes.is_empty() || acc.is_empty() {
            return !voice.active;
        }
        let mut vol = (voice.vol_l as i32) + (voice.vol_r as i32);
        if vol != 0 {
            vol += 2;
        }
        vol >>= 1;

        let step = voice.step as u32;
        let mut frac = voice.frac as u32;
        let mut cur = voice.cur;
        let mut end = voice.end;

        if end >= cur {
            for slot in acc.iter_mut() {
                if cur >= pcm_bytes.len() {
                    voice.active = false;
                    return true;
                }
                let s = pcm_bytes[cur] as i8 as i32;
                *slot = slot.wrapping_add(((s * vol) >> 8) as i16);

                frac += step;
                if frac >= 0x1000 {
                    cur += (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur >= end {
                        if let Some(ls) = voice.loop_start {
                            cur = ls + (cur - end);
                            end = ls + voice.loop_len;
                            if cur >= end {
                                cur = ls;
                            }
                        } else {
                            voice.cur = cur;
                            voice.end = end;
                            voice.frac = frac as u16;
                            voice.active = false;
                            return true;
                        }
                    }
                }
            }
        } else {
            for slot in acc.iter_mut() {
                if cur >= pcm_bytes.len() {
                    voice.active = false;
                    return true;
                }
                let s = pcm_bytes[cur] as i8 as i32;
                *slot = slot.wrapping_add(((s * vol) >> 8) as i16);

                frac += step;
                if frac >= 0x1000 {
                    let adv = (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur <= end.saturating_add(adv) {
                        if let Some(ls) = voice.loop_start {
                            let under = end.saturating_add(adv) - cur;
                            cur = ls.saturating_sub(under);
                            end = ls.saturating_sub(voice.loop_len);
                            if cur <= end {
                                cur = ls;
                            }
                        } else {
                            voice.cur = cur.saturating_sub(adv);
                            voice.end = end;
                            voice.frac = frac as u16;
                            voice.active = false;
                            return true;
                        }
                    } else {
                        cur -= adv;
                    }
                }
            }
        }

        voice.cur = cur;
        voice.end = end;
        voice.frac = frac as u16;
        false
    }

    /// Dispatch a KCEH voice using the selected [`KcehMixMode`].
    pub fn mix_kceh_voice(
        mode: KcehMixMode,
        pcm_bytes: &[u8],
        voice: &mut KcehVoice,
        acc_l: &mut [i16],
        acc_r: &mut [i16],
    ) -> bool {
        match mode {
            KcehMixMode::StereoLinear => Self::mix_kceh_stereo_linear(pcm_bytes, voice, acc_l, acc_r),
            KcehMixMode::StereoNearest => {
                Self::mix_kceh_stereo_nearest(pcm_bytes, voice, acc_l, acc_r)
            }
            KcehMixMode::MonoLinear => Self::mix_kceh_mono_linear(pcm_bytes, voice, acc_l),
            KcehMixMode::MonoNearest => Self::mix_kceh_mono_nearest(pcm_bytes, voice, acc_l),
        }
    }

    /// Exact KCEH stereo saturating clamp (`0x08043A5C`): clamps `acc_l` and `acc_r` to
    /// `[-128, +127]` DirectSound `i8` output and zeroes the `i16` accumulators.
    pub fn clamp_kceh_stereo_to_dma8(
        acc_l: &mut [i16],
        acc_r: &mut [i16],
        dma_l: &mut [i8],
        dma_r: &mut [i8],
    ) {
        let n = acc_l.len().min(acc_r.len()).min(dma_l.len()).min(dma_r.len());
        for i in 0..n {
            dma_l[i] = acc_l[i].clamp(-128, 127) as i8;
            dma_r[i] = acc_r[i].clamp(-128, 127) as i8;
            acc_l[i] = 0;
            acc_r[i] = 0;
        }
    }

    /// Exact KCEH mono saturating clamp (`0x08043B70`): clamps `acc` to `[-128, +127]`
    /// DirectSound `i8` output and zeroes the `i16` accumulator.
    pub fn clamp_kceh_mono_to_dma8(acc: &mut [i16], dma: &mut [i8]) {
        let n = acc.len().min(dma.len());
        for i in 0..n {
            dma[i] = acc[i].clamp(-128, 127) as i8;
            acc[i] = 0;
        }
    }

    // ========================================================================
    // Konami KCEJ ARM Mixers (v1..v5, 44 ROMs)
    // ========================================================================

    /// Exact KCEJ v2 (`KcejV2StereoLut`) 256-byte ROM volume LUT entry.
    ///
    /// Verified 0 / 16,384 mismatches against the retail *Hunter X Hunter* (`A8NJ`) ROM LUT:
    /// `round_half_away_from_zero(sample * vol / 15)` clamped to `[-128, +127]`.
    #[inline]
    #[must_use]
    pub const fn kcej_v2_volume_lut_entry(sample: i8, vol: u8) -> i8 {
        let num = (sample as i32) * (vol as i32);
        let q = if num >= 0 {
            (num * 2 + 15) / 30
        } else {
            -((-num * 2 + 15) / 30)
        };
        if q > 127 {
            127
        } else if q < -128 {
            -128
        } else {
            q as i8
        }
    }

    /// Mix a **KCEJ v1** (`KcejV1Mono8`, e.g. *Yu-Gi-Oh! EDS / WWE*) voice directly into
    /// a mono `i8` DirectSound ring buffer (wrapping 8-bit add, `(s * (vol + 1)) >> 4`,
    /// dedicated `step == 0x0800` 2x fast path).
    pub fn mix_kcej_v1_mono8(pcm_bytes: &[u8], voice: &mut KcejVoice, out: &mut [i8]) {
        if !voice.active || pcm_bytes.is_empty() || out.is_empty() {
            return;
        }
        let vol_eff = (voice.vol_r as i32) + 1;
        let mut pos = voice.pos;
        let mut rem = voice.rem;
        let mut frac = voice.frac as u32;
        let step = voice.step as u32;
        let mut out_idx = 0usize;
        let out_len = out.len();

        if step == 0x0800 {
            // Exact ARM 0x0800 (0.5x) fast path: subs r4, r4, #2; bmi done
            while out_idx + 2 <= out_len {
                if pos >= pcm_bytes.len() {
                    voice.active = false;
                    break;
                }
                let s = ((pcm_bytes[pos] as i8 as i32) * vol_eff) >> 4;
                pos += 1;
                out[out_idx] = out[out_idx].wrapping_add(s as i8);
                out[out_idx + 1] = out[out_idx + 1].wrapping_add(s as i8);
                out_idx += 2;
                rem -= 1;
                if rem <= 0 {
                    if let Some(ls) = voice.loop_start {
                        pos = ls;
                        rem = (voice.sample_len.saturating_sub(ls)) as i32;
                    } else {
                        voice.active = false;
                        break;
                    }
                }
            }
        } else {
            // General Q4.12 nearest-neighbor loop
            if pos >= pcm_bytes.len() {
                voice.active = false;
                return;
            }
            let mut scaled = ((pcm_bytes[pos] as i8 as i32) * vol_eff) >> 4;
            while out_idx < out_len {
                out[out_idx] = out[out_idx].wrapping_add(scaled as i8);
                out_idx += 1;

                frac += step;
                let adv = frac >> 12;
                if adv != 0 {
                    frac &= 0x0FFF;
                    rem -= adv as i32;
                    if rem > 0 {
                        pos += adv as usize;
                        if pos >= pcm_bytes.len() {
                            voice.active = false;
                            break;
                        }
                        scaled = ((pcm_bytes[pos] as i8 as i32) * vol_eff) >> 4;
                    } else if let Some(ls) = voice.loop_start {
                        pos = ls;
                        rem = (voice.sample_len.saturating_sub(ls)) as i32;
                        if pos >= pcm_bytes.len() {
                            voice.active = false;
                            break;
                        }
                        scaled = ((pcm_bytes[pos] as i8 as i32) * vol_eff) >> 4;
                    } else {
                        voice.active = false;
                        break;
                    }
                }
            }
        }

        voice.pos = pos;
        voice.rem = rem;
        voice.frac = frac as u16;
    }

    /// Mix a **KCEJ v2 / v3 / v4 / v5** stereo voice into separate Left/Right `i16`
    /// accumulators (native block size: 16 frames).
    ///
    /// Exact per-version sample scaling:
    /// - [`KonamiKceVersion::KcejV2StereoLut`]: 256-byte LUT `round(s * v / 15)`
    /// - [`KonamiKceVersion::KcejV3StereoAsr4Bias`]: `v = (s * vol) >> 4; if v < 0 { v += 1; }`
    /// - [`KonamiKceVersion::KcejV4StereoAsr7`]: `(s * vol) >> 7`
    /// - [`KonamiKceVersion::KcejV5StereoAsr7Bias`]: `p = s * vol; if p < 0 { p += 0x80; } p >> 7`
    pub fn mix_kcej_stereo_voice(
        version: KonamiKceVersion,
        pcm_bytes: &[u8],
        voice: &mut KcejVoice,
        acc_l: &mut [i16],
        acc_r: &mut [i16],
    ) {
        if !voice.active || pcm_bytes.is_empty() {
            return;
        }
        let frames = acc_l.len().min(acc_r.len());
        if frames == 0 {
            return;
        }

        let scale = |sample: i8, vol: u8| -> i16 {
            match version {
                KonamiKceVersion::KcejV2StereoLut => {
                    // Authentic KCEJ v2 ARM reverse-path quirk (`08027454: ldrsb r1, [r5, -r0]!`):
                    // sign-extending a negative sample before `[lut_base + vol*256 + r1]`
                    // indexes 256 bytes earlier, i.e. the `vol - 1` LUT row.
                    let eff_vol = if voice.reverse && sample < 0 {
                        vol.saturating_sub(1)
                    } else {
                        vol
                    };
                    Self::kcej_v2_volume_lut_entry(sample, eff_vol) as i16
                }
                KonamiKceVersion::KcejV3StereoAsr4Bias => {
                    let mut v = ((sample as i32) * (vol as i32)) >> 4;
                    if v < 0 {
                        v += 1;
                    }
                    v as i16
                }
                KonamiKceVersion::KcejV4StereoAsr7 => {
                    (((sample as i32) * (vol as i32)) >> 7) as i16
                }
                KonamiKceVersion::KcejV5StereoAsr7Bias => {
                    let mut p = (sample as i32) * (vol as i32);
                    if p < 0 {
                        p += 0x80;
                    }
                    (p >> 7) as i16
                }
                _ => (((sample as i32) * (vol as i32)) >> 7) as i16,
            }
        };

        let mut pos = voice.pos;
        let mut rem = voice.rem;
        let mut frac = voice.frac as u32;
        let step = voice.step as u32;

        if pos >= pcm_bytes.len() {
            voice.active = false;
            return;
        }
        let mut s = pcm_bytes[pos] as i8;
        let mut vl = scale(s, voice.vol_l);
        let mut vr = scale(s, voice.vol_r);

        for i in 0..frames {
            acc_l[i] = acc_l[i].wrapping_add(vl);
            acc_r[i] = acc_r[i].wrapping_add(vr);

            frac += step;
            let adv = frac >> 12;
            if adv != 0 {
                frac &= 0x0FFF;
                rem -= adv as i32;
                if rem > 0 {
                    if voice.reverse {
                        if pos < adv as usize {
                            voice.active = false;
                            break;
                        }
                        pos -= adv as usize;
                    } else {
                        pos += adv as usize;
                        if pos >= pcm_bytes.len() {
                            voice.active = false;
                            break;
                        }
                    }
                    s = pcm_bytes[pos] as i8;
                    vl = scale(s, voice.vol_l);
                    vr = scale(s, voice.vol_r);
                } else if !voice.reverse {
                    if let Some(ls) = voice.loop_start {
                        pos = ls;
                        rem = (voice.sample_len.saturating_sub(ls)) as i32;
                        if pos >= pcm_bytes.len() {
                            voice.active = false;
                            break;
                        }
                        s = pcm_bytes[pos] as i8;
                        vl = scale(s, voice.vol_l);
                        vr = scale(s, voice.vol_r);
                    } else {
                        voice.active = false;
                        break;
                    }
                } else {
                    voice.active = false;
                    break;
                }
            }
        }

        voice.pos = pos;
        voice.rem = rem;
        voice.frac = frac as u16;
    }

    /// Exact KCEJ v2–v5 stereo DirectSound unpack (`strb` low-8-bit truncation, no clamp).
    pub fn pack_kcej_stereo_to_dma8(
        acc_l: &[i16],
        acc_r: &[i16],
        dma_l: &mut [i8],
        dma_r: &mut [i8],
    ) {
        let n = acc_l.len().min(acc_r.len()).min(dma_l.len()).min(dma_r.len());
        for i in 0..n {
            dma_l[i] = acc_l[i] as i8;
            dma_r[i] = acc_r[i] as i8;
        }
    }
}

// ============================================================================
// StudioSynth: 48,000 Hz f32 Band-Limited Lanczos-3 + Equal-Power Panning
// ============================================================================

const LANCZOS_PHASES: usize = 256;

const fn sin_const(mut x: f64) -> f64 {
    const PI: f64 = core::f64::consts::PI;
    const TWO_PI: f64 = 2.0 * PI;
    while x > PI {
        x -= TWO_PI;
    }
    while x < -PI {
        x += TWO_PI;
    }
    let x2 = x * x;
    let mut term = x;
    let mut sum = x;
    let mut k = 1u32;
    while k <= 10 {
        let d = (2 * k) * (2 * k + 1);
        term = -term * x2 / (d as f64);
        sum += term;
        k += 1;
    }
    sum
}

const fn lanczos3_weight(x: f64) -> f32 {
    if x == 0.0 {
        return 1.0;
    }
    let ax = if x < 0.0 { -x } else { x };
    if ax >= 3.0 {
        return 0.0;
    }
    let pi_x = core::f64::consts::PI * x;
    let pi_x_a = pi_x / 3.0;
    let sinc_x = sin_const(pi_x) / pi_x;
    let sinc_a = sin_const(pi_x_a) / pi_x_a;
    (sinc_x * sinc_a) as f32
}

const fn build_lanczos3_lut() -> [[f32; 6]; LANCZOS_PHASES + 1] {
    let mut lut = [[0.0f32; 6]; LANCZOS_PHASES + 1];
    let mut p = 0usize;
    while p <= LANCZOS_PHASES {
        let frac = (p as f64) / (LANCZOS_PHASES as f64);
        let mut t = 0usize;
        while t < 6 {
            let k = (t as i32) - 2;
            lut[p][t] = lanczos3_weight(frac - (k as f64));
            t += 1;
        }
        p += 1;
    }
    lut
}

static LANCZOS3_LUT: [[f32; 6]; LANCZOS_PHASES + 1] = build_lanczos3_lut();

/// Pure `no_std` cosine approximation on `[0, pi/2]` (full `f64` precision).
#[inline]
fn cos_f64(x: f64) -> f64 {
    sin_const(core::f64::consts::FRAC_PI_2 - x)
}

/// Pure `no_std` sine approximation on `[0, pi/2]` (full `f64` precision).
#[inline]
fn sin_f64(x: f64) -> f64 {
    sin_const(x)
}

/// High-fidelity 48,000 Hz `f32` software synthesizer with band-limited windowed-sinc
/// (Lanczos-3) interpolation and true equal-power stereo panning.
#[derive(Debug, Clone)]
pub struct StudioSynth {
    pub sample_rate: u32,
    /// Number of Lanczos kernel lobes (default `3`, using a compile-time 256-phase
    /// 6-tap polyphase LUT for maximum throughput).
    pub sinc_lobes: usize,
}

impl Default for StudioSynth {
    fn default() -> Self {
        Self::new(Self::SAMPLE_RATE)
    }
}

impl StudioSynth {
    /// Standard studio output sample rate (`48_000` Hz).
    pub const SAMPLE_RATE: u32 = 48_000;

    /// Create a new `StudioSynth` at the specified output sample rate (typically `48_000` Hz).
    #[must_use]
    pub const fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            sinc_lobes: 3,
        }
    }

    /// Render a Konami KCE sample into stereo `f32` buffers (accumulating) using band-limited
    /// Lanczos-3 windowed-sinc interpolation and true equal-power stereo panning.
    ///
    /// - `freq_hz`: desired playback sample rate in Hz.
    /// - `volume`: linear gain in `0.0..=1.0`.
    /// - `pan`: stereo pan in `-1.0..=1.0` (`-1.0` = hard left, `0.0` = center, `+1.0` = hard right).
    #[allow(clippy::too_many_arguments)]
    pub fn render_voice_f32(
        &self,
        sample: &KonamiSampleView<'_>,
        pos_f64: &mut f64,
        freq_hz: f64,
        volume: f32,
        pan: f32,
        out_left: &mut [f32],
        out_right: &mut [f32],
    ) {
        let frames = out_left.len().min(out_right.len());
        if frames == 0
            || sample.is_empty()
            || self.sample_rate == 0
            || !freq_hz.is_finite()
            || freq_hz <= 0.0
            || !pos_f64.is_finite()
        {
            return;
        }
        if *pos_f64 < 0.0 {
            *pos_f64 = 0.0;
        }

        let step = freq_hz / (self.sample_rate as f64);
        let gain = (volume.clamp(0.0, 1.0) as f64) * (1.0 / 128.0);
        // Equal-power stereo pan law: pan in [-1.0, +1.0] -> theta in [0, pi/2]
        let pan_norm = ((pan.clamp(-1.0, 1.0) as f64) + 1.0) * 0.5;
        let angle = pan_norm * core::f64::consts::FRAC_PI_2;
        let gain_l = (gain * cos_f64(angle)) as f32;
        let gain_r = (gain * sin_f64(angle)) as f32;

        let pcm = sample.pcm_bytes;
        let pcm_len = pcm.len();
        let loop_len = sample.loop_len();
        let looped = loop_len > 0;
        let loop_start = sample.loop_start.unwrap_or(0) as usize;
        let loop_start_f = loop_start as f64;
        let loop_len_f = loop_len as f64;
        let pcm_len_f = pcm_len as f64;

        let out_l = &mut out_left[..frames];
        let out_r = &mut out_right[..frames];
        let safe_max_base = pcm_len.saturating_sub(4);
        let mut ph = *pos_f64;
        let mut idx = 0usize;

        while idx < frames {
            if ph >= pcm_len_f {
                if looped {
                    ph -= loop_len_f;
                    if ph >= pcm_len_f {
                        ph = loop_start_f + ((ph - loop_start_f) % loop_len_f);
                    }
                    if ph < loop_start_f {
                        ph = loop_start_f;
                    }
                } else {
                    break;
                }
            }

            let base = ph as usize;
            if pcm_len >= 6 && base >= 2 && base <= safe_max_base && step < (1u64 << 31) as f64 {
                let safe_limit = (safe_max_base as f64) + 0.999_999;
                let max_interior = (((safe_limit - ph) / step) as usize) + 1;
                let run = max_interior.min(frames - idx);
                if run >= 4 {
                    const Q32_SCALE: f64 = 4_294_967_296.0;
                    const INV_Q24: f32 = 1.0 / 16_777_216.0;
                    let mut ph_q32 = (ph * Q32_SCALE) as u64;
                    let step_q32 = (step * Q32_SCALE) as u64;
                    let last_base = ((ph_q32
                        .saturating_add(step_q32.saturating_mul((run - 1) as u64)))
                        >> 32) as usize;
                    if last_base <= safe_max_base {
                        for (ol, or) in out_l[idx..idx + run]
                            .iter_mut()
                            .zip(&mut out_r[idx..idx + run])
                        {
                            let b = (ph_q32 >> 32) as usize;
                            let p = ((ph_q32 >> 24) & 0xFF) as usize;
                            let t = ((ph_q32 & 0x00FF_FFFF) as f32) * INV_Q24;
                            let w0 = &LANCZOS3_LUT[p];
                            let w1 = &LANCZOS3_LUT[p + 1];
                            let win = &pcm[b - 2..b + 4];
                            let s0 = (win[0] as i8) as f32;
                            let s1 = (win[1] as i8) as f32;
                            let s2 = (win[2] as i8) as f32;
                            let s3 = (win[3] as i8) as f32;
                            let s4 = (win[4] as i8) as f32;
                            let s5 = (win[5] as i8) as f32;
                            let a0 = s0 * w0[0]
                                + s1 * w0[1]
                                + s2 * w0[2]
                                + s3 * w0[3]
                                + s4 * w0[4]
                                + s5 * w0[5];
                            let a1 = s0 * w1[0]
                                + s1 * w1[1]
                                + s2 * w1[2]
                                + s3 * w1[3]
                                + s4 * w1[4]
                                + s5 * w1[5];
                            let val = a0 + t * (a1 - a0);
                            *ol += val * gain_l;
                            *or += val * gain_r;
                            ph_q32 += step_q32;
                        }
                        ph += (run as f64) * step;
                        idx += run;
                        continue;
                    }
                }
            }

            // Boundary / loop-edge single-sample step
            let frac = ph - (base as f64);
            let idx_f = frac * (LANCZOS_PHASES as f64);
            let p = (idx_f as usize).min(LANCZOS_PHASES - 1);
            let t = (idx_f - (p as f64)) as f32;
            let w0 = &LANCZOS3_LUT[p];
            let w1 = &LANCZOS3_LUT[p + 1];

            let mut s = [0.0f32; 6];
            let base_i = base as isize;
            for (slot, k) in s.iter_mut().zip(-2isize..=3) {
                let sample_idx = base_i + k;
                if sample_idx >= 0 {
                    let u = sample_idx as usize;
                    if u < pcm_len {
                        *slot = (pcm[u] as i8) as f32;
                    } else if looped {
                        let wrapped = loop_start + ((u - loop_start) % loop_len);
                        *slot = (pcm[wrapped] as i8) as f32;
                    }
                }
            }

            let a0 = s[0] * w0[0]
                + s[1] * w0[1]
                + s[2] * w0[2]
                + s[3] * w0[3]
                + s[4] * w0[4]
                + s[5] * w0[5];
            let a1 = s[0] * w1[0]
                + s[1] * w1[1]
                + s[2] * w1[2]
                + s[3] * w1[3]
                + s[4] * w1[4]
                + s[5] * w1[5];
            let val = a0 + t * (a1 - a0);

            out_l[idx] += val * gain_l;
            out_r[idx] += val * gain_r;
            ph += step;
            idx += 1;
        }

        *pos_f64 = ph;
    }
}

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

/// Compile-time 16 × 256 signed volume lookup table for KCEJ v2 (`KcejV2StereoLut`),
/// matching the exact retail *Hunter X Hunter* (`A8NJ`) ROM LUT (`4,096` bytes).
const fn build_kcej_v2_lut() -> [[i8; 256]; 16] {
    let mut table = [[0i8; 256]; 16];
    let mut v = 0usize;
    while v < 16 {
        let mut b = 0usize;
        while b < 256 {
            table[v][b] = AccuracySynth::kcej_v2_volume_lut_entry(b as u8 as i8, v as u8);
            b += 1;
        }
        v += 1;
    }
    table
}

static KCEJ_V2_LUT: [[i8; 256]; 16] = build_kcej_v2_lut();

#[inline(always)]
fn kcej_v2_lut_fast(sample: i8, vol: u8) -> i16 {
    if vol < 16 {
        KCEJ_V2_LUT[vol as usize][sample as u8 as usize] as i16
    } else {
        AccuracySynth::kcej_v2_volume_lut_entry(sample, vol) as i16
    }
}

/// Bit-exact software emulation of all 6 Konami KCE ARM7TDMI mixer revisions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AccuracySynth;

impl AccuracySynth {
    // ========================================================================
    // Konami KCEH / Cross-Studio 32-Byte Soundbank ARM Mixer (172 ROMs)
    // ========================================================================

    /// Shared const-generic KCEH stereo mixer kernel (`LINEAR = true` for linear, `false` for nearest).
    #[inline(always)]
    fn mix_kceh_stereo_impl<const LINEAR: bool>(
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
        let acc_l = &mut acc_l[..frames];
        let acc_r = &mut acc_r[..frames];

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
        let step_u64 = step as u64;
        let mut frac = voice.frac as u32;
        let mut cur = voice.cur;
        let mut end = voice.end;
        let loop_start = voice.loop_start;
        let loop_len = voice.loop_len;
        let pcm_len = pcm_bytes.len();
        let center_pan = vol_l == vol_r;
        let mut offset = 0usize;

        if end >= cur {
            // Forward playback
            while offset < frames {
                let safe_end = end.min(pcm_len);
                let limit_idx = if LINEAR {
                    safe_end.saturating_sub(1)
                } else {
                    safe_end
                };

                if step > 0 && frac < 0x1000 && cur < limit_idx {
                    let ph0 = ((cur as u64) << 12) | (frac as u64);
                    let limit_ph = (limit_idx as u64) << 12;
                    let k_max = (limit_ph - 1 - ph0) / step_u64;
                    let run = ((k_max + 1) as usize).min(frames - offset);

                    let al = &mut acc_l[offset..offset + run];
                    let ar = &mut acc_r[offset..offset + run];

                    if step == 0x1000 && frac == 0 {
                        // 1:1 pitch fast path: contiguous slice enables full SIMD auto-vectorization
                        let src = &pcm_bytes[cur..cur + run];
                        if center_pan {
                            for ((&b, l), r) in src.iter().zip(al.iter_mut()).zip(ar.iter_mut()) {
                                let v = (((b as i8 as i32) * vol_l) >> 8) as i16;
                                *l = l.wrapping_add(v);
                                *r = r.wrapping_add(v);
                            }
                        } else {
                            for ((&b, l), r) in src.iter().zip(al.iter_mut()).zip(ar.iter_mut()) {
                                let s = b as i8 as i32;
                                *l = l.wrapping_add(((s * vol_l) >> 8) as i16);
                                *r = r.wrapping_add(((s * vol_r) >> 8) as i16);
                            }
                        }
                        cur += run;
                    } else if LINEAR {
                        let src = &pcm_bytes[..safe_end];
                        let mut ph = ph0;
                        if center_pan {
                            for (l, r) in al.iter_mut().zip(ar.iter_mut()) {
                                let idx = (ph >> 12) as usize;
                                let pair = &src[idx..idx + 2];
                                let s0 = pair[0] as i8 as i32;
                                let s1 = pair[1] as i8 as i32;
                                let f = (ph & 0x0FFF) as i32;
                                let s = s0 + (((s1 - s0) * f) >> 12);
                                let v = ((s * vol_l) >> 8) as i16;
                                *l = l.wrapping_add(v);
                                *r = r.wrapping_add(v);
                                ph += step_u64;
                            }
                        } else {
                            for (l, r) in al.iter_mut().zip(ar.iter_mut()) {
                                let idx = (ph >> 12) as usize;
                                let pair = &src[idx..idx + 2];
                                let s0 = pair[0] as i8 as i32;
                                let s1 = pair[1] as i8 as i32;
                                let f = (ph & 0x0FFF) as i32;
                                let s = s0 + (((s1 - s0) * f) >> 12);
                                *l = l.wrapping_add(((s * vol_l) >> 8) as i16);
                                *r = r.wrapping_add(((s * vol_r) >> 8) as i16);
                                ph += step_u64;
                            }
                        }
                        cur = (ph >> 12) as usize;
                        frac = (ph & 0x0FFF) as u32;
                    } else {
                        let src = &pcm_bytes[..safe_end];
                        let mut ph = ph0;
                        if center_pan {
                            for (l, r) in al.iter_mut().zip(ar.iter_mut()) {
                                let s = src[(ph >> 12) as usize] as i8 as i32;
                                let v = ((s * vol_l) >> 8) as i16;
                                *l = l.wrapping_add(v);
                                *r = r.wrapping_add(v);
                                ph += step_u64;
                            }
                        } else {
                            for (l, r) in al.iter_mut().zip(ar.iter_mut()) {
                                let s = src[(ph >> 12) as usize] as i8 as i32;
                                *l = l.wrapping_add(((s * vol_l) >> 8) as i16);
                                *r = r.wrapping_add(((s * vol_r) >> 8) as i16);
                                ph += step_u64;
                            }
                        }
                        cur = (ph >> 12) as usize;
                        frac = (ph & 0x0FFF) as u32;
                    }

                    offset += run;
                    if cur >= end {
                        if let Some(ls) = loop_start {
                            cur = ls.saturating_add(cur - end);
                            end = ls.saturating_add(loop_len);
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
                    continue;
                }

                // Boundary / loop-edge single-sample step
                if cur >= pcm_len {
                    voice.active = false;
                    return true;
                }
                let s = if LINEAR {
                    let next = cur + 1;
                    let next_idx = if next >= end {
                        loop_start.unwrap_or(cur)
                    } else {
                        next
                    };
                    let s0 = pcm_bytes[cur] as i8 as i32;
                    let s1 = pcm_bytes[next_idx.min(pcm_len - 1)] as i8 as i32;
                    let f = (frac & 0x0FFF) as i32;
                    s0 + (((s1 - s0) * f) >> 12)
                } else {
                    pcm_bytes[cur] as i8 as i32
                };

                acc_l[offset] = acc_l[offset].wrapping_add(((s * vol_l) >> 8) as i16);
                acc_r[offset] = acc_r[offset].wrapping_add(((s * vol_r) >> 8) as i16);
                offset += 1;

                frac += step;
                if frac >= 0x1000 {
                    cur += (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur >= end {
                        if let Some(ls) = loop_start {
                            cur = ls.saturating_add(cur - end);
                            end = ls.saturating_add(loop_len);
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
            while offset < frames {
                let min_cur = if LINEAR {
                    end.saturating_add(2)
                } else {
                    end.saturating_add(1)
                };

                if step > 0 && frac < 0x1000 && cur < pcm_len && cur >= min_cur {
                    let limit_tf = ((cur - min_cur + 1) as u64) << 12;
                    let tf0 = frac as u64;
                    let k_max = (limit_tf - 1 - tf0) / step_u64;
                    let run = ((k_max + 1) as usize).min(frames - offset);

                    let al = &mut acc_l[offset..offset + run];
                    let ar = &mut acc_r[offset..offset + run];
                    let src = &pcm_bytes[..cur + 1];
                    let cur0 = cur;
                    let mut tf = tf0;

                    if LINEAR {
                        for (l, r) in al.iter_mut().zip(ar.iter_mut()) {
                            let idx = cur0 - (tf >> 12) as usize;
                            let pair = &src[idx - 1..idx + 1];
                            let s0 = pair[1] as i8 as i32;
                            let s1 = pair[0] as i8 as i32;
                            let f = (tf & 0x0FFF) as i32;
                            let s = s0 + (((s1 - s0) * f) >> 12);
                            *l = l.wrapping_add(((s * vol_l) >> 8) as i16);
                            *r = r.wrapping_add(((s * vol_r) >> 8) as i16);
                            tf += step_u64;
                        }
                    } else {
                        for (l, r) in al.iter_mut().zip(ar.iter_mut()) {
                            let idx = cur0 - (tf >> 12) as usize;
                            let s = src[idx] as i8 as i32;
                            *l = l.wrapping_add(((s * vol_l) >> 8) as i16);
                            *r = r.wrapping_add(((s * vol_r) >> 8) as i16);
                            tf += step_u64;
                        }
                    }

                    let adv = (tf >> 12) as usize;
                    frac = (tf & 0x0FFF) as u32;
                    offset += run;

                    if cur <= end.saturating_add(adv) {
                        if let Some(ls) = loop_start {
                            let under = end.saturating_add(adv) - cur;
                            cur = ls.saturating_sub(under);
                            end = ls.saturating_sub(loop_len);
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
                    continue;
                }

                // Boundary / loop-edge single-sample step
                if cur >= pcm_len {
                    voice.active = false;
                    return true;
                }
                let s = if LINEAR {
                    let next_idx = if cur == 0 || (cur - 1) <= end {
                        loop_start.unwrap_or(cur)
                    } else {
                        cur - 1
                    };
                    let s0 = pcm_bytes[cur] as i8 as i32;
                    let s1 = pcm_bytes[next_idx.min(pcm_len - 1)] as i8 as i32;
                    let f = (frac & 0x0FFF) as i32;
                    s0 + (((s1 - s0) * f) >> 12)
                } else {
                    pcm_bytes[cur] as i8 as i32
                };

                acc_l[offset] = acc_l[offset].wrapping_add(((s * vol_l) >> 8) as i16);
                acc_r[offset] = acc_r[offset].wrapping_add(((s * vol_r) >> 8) as i16);
                offset += 1;

                frac += step;
                if frac >= 0x1000 {
                    let adv = (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur <= end.saturating_add(adv) {
                        if let Some(ls) = loop_start {
                            let under = end.saturating_add(adv) - cur;
                            cur = ls.saturating_sub(under);
                            end = ls.saturating_sub(loop_len);
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

    /// Shared const-generic KCEH mono mixer kernel (`LINEAR = true` for linear, `false` for nearest).
    #[inline(always)]
    fn mix_kceh_mono_impl<const LINEAR: bool>(
        pcm_bytes: &[u8],
        voice: &mut KcehVoice,
        acc: &mut [i16],
    ) -> bool {
        if !voice.active || pcm_bytes.is_empty() || acc.is_empty() {
            return !voice.active;
        }
        let frames = acc.len();
        let mut vol = (voice.vol_l as i32) + (voice.vol_r as i32);
        if vol != 0 {
            vol += 2;
        }
        vol >>= 1;

        let step = voice.step as u32;
        let step_u64 = step as u64;
        let mut frac = voice.frac as u32;
        let mut cur = voice.cur;
        let mut end = voice.end;
        let loop_start = voice.loop_start;
        let loop_len = voice.loop_len;
        let pcm_len = pcm_bytes.len();
        let mut offset = 0usize;

        if end >= cur {
            while offset < frames {
                let safe_end = end.min(pcm_len);
                let limit_idx = if LINEAR {
                    safe_end.saturating_sub(1)
                } else {
                    safe_end
                };

                if step > 0 && frac < 0x1000 && cur < limit_idx {
                    let ph0 = ((cur as u64) << 12) | (frac as u64);
                    let limit_ph = (limit_idx as u64) << 12;
                    let k_max = (limit_ph - 1 - ph0) / step_u64;
                    let run = ((k_max + 1) as usize).min(frames - offset);
                    let dst = &mut acc[offset..offset + run];

                    if step == 0x1000 && frac == 0 {
                        let src = &pcm_bytes[cur..cur + run];
                        for (&b, slot) in src.iter().zip(dst.iter_mut()) {
                            *slot = slot.wrapping_add((((b as i8 as i32) * vol) >> 8) as i16);
                        }
                        cur += run;
                    } else if LINEAR {
                        let src = &pcm_bytes[..safe_end];
                        let mut ph = ph0;
                        for slot in dst.iter_mut() {
                            let idx = (ph >> 12) as usize;
                            let pair = &src[idx..idx + 2];
                            let s0 = pair[0] as i8 as i32;
                            let s1 = pair[1] as i8 as i32;
                            let f = (ph & 0x0FFF) as i32;
                            let s = s0 + (((s1 - s0) * f) >> 12);
                            *slot = slot.wrapping_add(((s * vol) >> 8) as i16);
                            ph += step_u64;
                        }
                        cur = (ph >> 12) as usize;
                        frac = (ph & 0x0FFF) as u32;
                    } else {
                        let src = &pcm_bytes[..safe_end];
                        let mut ph = ph0;
                        for slot in dst.iter_mut() {
                            let s = src[(ph >> 12) as usize] as i8 as i32;
                            *slot = slot.wrapping_add(((s * vol) >> 8) as i16);
                            ph += step_u64;
                        }
                        cur = (ph >> 12) as usize;
                        frac = (ph & 0x0FFF) as u32;
                    }

                    offset += run;
                    if cur >= end {
                        if let Some(ls) = loop_start {
                            cur = ls.saturating_add(cur - end);
                            end = ls.saturating_add(loop_len);
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
                    continue;
                }

                if cur >= pcm_len {
                    voice.active = false;
                    return true;
                }
                let s = if LINEAR {
                    let next = cur + 1;
                    let next_idx = if next >= end {
                        loop_start.unwrap_or(cur)
                    } else {
                        next
                    };
                    let s0 = pcm_bytes[cur] as i8 as i32;
                    let s1 = pcm_bytes[next_idx.min(pcm_len - 1)] as i8 as i32;
                    let f = (frac & 0x0FFF) as i32;
                    s0 + (((s1 - s0) * f) >> 12)
                } else {
                    pcm_bytes[cur] as i8 as i32
                };
                acc[offset] = acc[offset].wrapping_add(((s * vol) >> 8) as i16);
                offset += 1;

                frac += step;
                if frac >= 0x1000 {
                    cur += (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur >= end {
                        if let Some(ls) = loop_start {
                            cur = ls.saturating_add(cur - end);
                            end = ls.saturating_add(loop_len);
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
            while offset < frames {
                if cur >= pcm_len {
                    voice.active = false;
                    return true;
                }
                let s = if LINEAR {
                    let next_idx = if cur == 0 || (cur - 1) <= end {
                        loop_start.unwrap_or(cur)
                    } else {
                        cur - 1
                    };
                    let s0 = pcm_bytes[cur] as i8 as i32;
                    let s1 = pcm_bytes[next_idx.min(pcm_len - 1)] as i8 as i32;
                    let f = (frac & 0x0FFF) as i32;
                    s0 + (((s1 - s0) * f) >> 12)
                } else {
                    pcm_bytes[cur] as i8 as i32
                };
                acc[offset] = acc[offset].wrapping_add(((s * vol) >> 8) as i16);
                offset += 1;

                frac += step;
                if frac >= 0x1000 {
                    let adv = (frac >> 12) as usize;
                    frac &= 0x0FFF;
                    if cur <= end.saturating_add(adv) {
                        if let Some(ls) = loop_start {
                            let under = end.saturating_add(adv) - cur;
                            cur = ls.saturating_sub(under);
                            end = ls.saturating_sub(loop_len);
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
    /// ARM7TDMI **Stereo Linear** kernel (`0x08043C08`).
    ///
    /// Returns `true` if a one-shot voice reached its end during this block.
    #[inline]
    pub fn mix_kceh_stereo_linear(
        pcm_bytes: &[u8],
        voice: &mut KcehVoice,
        acc_l: &mut [i16],
        acc_r: &mut [i16],
    ) -> bool {
        Self::mix_kceh_stereo_impl::<true>(pcm_bytes, voice, acc_l, acc_r)
    }

    /// Mix a KCEH voice into separate Left/Right `i16` accumulators using the exact
    /// ARM7TDMI **Stereo Nearest-Neighbor** kernel (`0x08043DE8`).
    #[inline]
    pub fn mix_kceh_stereo_nearest(
        pcm_bytes: &[u8],
        voice: &mut KcehVoice,
        acc_l: &mut [i16],
        acc_r: &mut [i16],
    ) -> bool {
        Self::mix_kceh_stereo_impl::<false>(pcm_bytes, voice, acc_l, acc_r)
    }

    /// Mix a KCEH voice into a mono `i16` accumulator using the exact ARM7TDMI
    /// **Mono Linear** kernel (`0x08043F74`).
    #[inline]
    pub fn mix_kceh_mono_linear(
        pcm_bytes: &[u8],
        voice: &mut KcehVoice,
        acc: &mut [i16],
    ) -> bool {
        Self::mix_kceh_mono_impl::<true>(pcm_bytes, voice, acc)
    }

    /// Mix a KCEH voice into a mono `i16` accumulator using the exact ARM7TDMI
    /// **Mono Nearest-Neighbor** kernel (`0x08044128`).
    #[inline]
    pub fn mix_kceh_mono_nearest(
        pcm_bytes: &[u8],
        voice: &mut KcehVoice,
        acc: &mut [i16],
    ) -> bool {
        Self::mix_kceh_mono_impl::<false>(pcm_bytes, voice, acc)
    }

    /// Dispatch a KCEH voice using the selected [`KcehMixMode`].
    #[inline]
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
    #[inline]
    pub fn clamp_kceh_stereo_to_dma8(
        acc_l: &mut [i16],
        acc_r: &mut [i16],
        dma_l: &mut [i8],
        dma_r: &mut [i8],
    ) {
        let n = acc_l.len().min(acc_r.len()).min(dma_l.len()).min(dma_r.len());
        let al = &mut acc_l[..n];
        let ar = &mut acc_r[..n];
        let dl = &mut dma_l[..n];
        let dr = &mut dma_r[..n];

        for (d, &a) in dl.iter_mut().zip(al.iter()) {
            *d = a.clamp(-128, 127) as i8;
        }
        al.fill(0);

        for (d, &a) in dr.iter_mut().zip(ar.iter()) {
            *d = a.clamp(-128, 127) as i8;
        }
        ar.fill(0);
    }

    /// Exact KCEH mono saturating clamp (`0x08043B70`): clamps `acc` to `[-128, +127]`
    /// DirectSound `i8` output and zeroes the `i16` accumulator.
    #[inline]
    pub fn clamp_kceh_mono_to_dma8(acc: &mut [i16], dma: &mut [i8]) {
        let n = acc.len().min(dma.len());
        let a = &mut acc[..n];
        let d = &mut dma[..n];
        for (dst, &src) in d.iter_mut().zip(a.iter()) {
            *dst = src.clamp(-128, 127) as i8;
        }
        a.fill(0);
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
        let step_u64 = step as u64;
        let pcm_len = pcm_bytes.len();
        let loop_start = voice.loop_start;
        let sample_len = voice.sample_len;
        let mut out_idx = 0usize;
        let out_len = out.len();

        if step == 0x0800 {
            // Exact ARM 0x0800 (0.5x) fast path: subs r4, r4, #2; bmi done
            while out_idx + 2 <= out_len {
                if pos >= pcm_len {
                    voice.active = false;
                    break;
                }
                if rem > 0 {
                    let pairs = ((out_len - out_idx) >> 1)
                        .min(rem as usize)
                        .min(pcm_len - pos);
                    let src = &pcm_bytes[pos..pos + pairs];
                    let (dst_pairs, _) = out[out_idx..out_idx + (pairs << 1)].as_chunks_mut::<2>();
                    for (&b, pair) in src.iter().zip(dst_pairs.iter_mut()) {
                        let s = (((b as i8 as i32) * vol_eff) >> 4) as i8;
                        pair[0] = pair[0].wrapping_add(s);
                        pair[1] = pair[1].wrapping_add(s);
                    }
                    pos += pairs;
                    rem -= pairs as i32;
                    out_idx += pairs << 1;
                    if rem <= 0 {
                        if let Some(ls) = loop_start {
                            pos = ls;
                            rem = sample_len.saturating_sub(ls).min(i32::MAX as usize) as i32;
                        } else {
                            voice.active = false;
                            break;
                        }
                    }
                    continue;
                }

                // Single-pair fallback when `rem <= 0` initially
                let s = (((pcm_bytes[pos] as i8 as i32) * vol_eff) >> 4) as i8;
                pos += 1;
                out[out_idx] = out[out_idx].wrapping_add(s);
                out[out_idx + 1] = out[out_idx + 1].wrapping_add(s);
                out_idx += 2;
                rem = rem.saturating_sub(1);
                if rem <= 0 {
                    if let Some(ls) = loop_start {
                        pos = ls;
                        rem = sample_len.saturating_sub(ls).min(i32::MAX as usize) as i32;
                    } else {
                        voice.active = false;
                        break;
                    }
                }
            }
        } else {
            // General Q4.12 nearest-neighbor loop
            while out_idx < out_len {
                if pos >= pcm_len {
                    voice.active = false;
                    break;
                }

                if step > 0 && frac < 0x1000 && rem > 0 {
                    let safe_span = (rem as usize).min(pcm_len - pos);
                    let ph0 = ((pos as u64) << 12) | (frac as u64);
                    let limit_ph = ((pos + safe_span) as u64) << 12;
                    let k_max = (limit_ph - 1 - ph0) / step_u64;
                    let run = ((k_max + 1) as usize).min(out_len - out_idx);
                    let dst = &mut out[out_idx..out_idx + run];

                    if step == 0x1000 {
                        // 1:1 pitch fast path: contiguous slice SIMD auto-vectorization
                        let src = &pcm_bytes[pos..pos + run];
                        for (&b, slot) in src.iter().zip(dst.iter_mut()) {
                            let s = (((b as i8 as i32) * vol_eff) >> 4) as i8;
                            *slot = slot.wrapping_add(s);
                        }
                        pos += run;
                        rem -= run as i32;
                    } else {
                        let src = &pcm_bytes[..pos + safe_span];
                        let mut ph = ph0;
                        for slot in dst.iter_mut() {
                            let s = (((src[(ph >> 12) as usize] as i8 as i32) * vol_eff) >> 4)
                                as i8;
                            *slot = slot.wrapping_add(s);
                            ph += step_u64;
                        }
                        let new_pos = (ph >> 12) as usize;
                        rem -= (new_pos - pos) as i32;
                        pos = new_pos;
                        frac = (ph & 0x0FFF) as u32;
                    }

                    out_idx += run;
                    if rem <= 0 {
                        if let Some(ls) = loop_start {
                            pos = ls;
                            rem = sample_len.saturating_sub(ls).min(i32::MAX as usize) as i32;
                            if pos >= pcm_len {
                                voice.active = false;
                                break;
                            }
                        } else {
                            voice.active = false;
                            break;
                        }
                    } else if pos >= pcm_len {
                        voice.active = false;
                        break;
                    }
                    continue;
                }

                // Boundary fallback (`rem <= 0` or `step == 0` or `frac >= 0x1000`)
                let scaled = (((pcm_bytes[pos] as i8 as i32) * vol_eff) >> 4) as i8;
                out[out_idx] = out[out_idx].wrapping_add(scaled);
                out_idx += 1;

                frac += step;
                let adv = frac >> 12;
                if adv != 0 {
                    frac &= 0x0FFF;
                    rem = rem.saturating_sub(adv as i32);
                    if rem > 0 {
                        pos += adv as usize;
                    } else if let Some(ls) = loop_start {
                        pos = ls;
                        rem = sample_len.saturating_sub(ls).min(i32::MAX as usize) as i32;
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

    /// Monomorphized KCEJ v2–v5 stereo mixing kernel (eliminates per-sample version dispatch,
    /// hoists reverse & center-pan checks, and chunks safe interior runs).
    #[inline(always)]
    fn mix_kcej_stereo_impl<F>(
        pcm_bytes: &[u8],
        voice: &mut KcejVoice,
        acc_l: &mut [i16],
        acc_r: &mut [i16],
        scale: F,
    ) where
        F: Fn(i8, u8) -> i16,
    {
        if !voice.active || pcm_bytes.is_empty() {
            return;
        }
        let frames = acc_l.len().min(acc_r.len());
        if frames == 0 {
            return;
        }
        let acc_l = &mut acc_l[..frames];
        let acc_r = &mut acc_r[..frames];

        let mut pos = voice.pos;
        let mut rem = voice.rem;
        let mut frac = voice.frac as u32;
        let step = voice.step as u32;
        let step_u64 = step as u64;
        let vol_l = voice.vol_l;
        let vol_r = voice.vol_r;
        let center_pan = vol_l == vol_r;
        let loop_start = voice.loop_start;
        let sample_len = voice.sample_len;
        let pcm_len = pcm_bytes.len();
        let mut offset = 0usize;

        if !voice.reverse {
            // Forward stereo playback
            while offset < frames {
                if pos >= pcm_len {
                    voice.active = false;
                    break;
                }

                if step > 0 && frac < 0x1000 && rem > 0 {
                    let safe_span = (rem as usize).min(pcm_len - pos);
                    let ph0 = ((pos as u64) << 12) | (frac as u64);
                    let limit_ph = ((pos + safe_span) as u64) << 12;
                    let k_max = (limit_ph - 1 - ph0) / step_u64;
                    let run = ((k_max + 1) as usize).min(frames - offset);
                    let al = &mut acc_l[offset..offset + run];
                    let ar = &mut acc_r[offset..offset + run];

                    if step == 0x1000 {
                        let src = &pcm_bytes[pos..pos + run];
                        if center_pan {
                            for ((&b, l), r) in src.iter().zip(al.iter_mut()).zip(ar.iter_mut()) {
                                let v = scale(b as i8, vol_l);
                                *l = l.wrapping_add(v);
                                *r = r.wrapping_add(v);
                            }
                        } else {
                            for ((&b, l), r) in src.iter().zip(al.iter_mut()).zip(ar.iter_mut()) {
                                let s = b as i8;
                                *l = l.wrapping_add(scale(s, vol_l));
                                *r = r.wrapping_add(scale(s, vol_r));
                            }
                        }
                        pos += run;
                        rem -= run as i32;
                    } else {
                        let src = &pcm_bytes[..pos + safe_span];
                        let mut ph = ph0;
                        if center_pan {
                            for (l, r) in al.iter_mut().zip(ar.iter_mut()) {
                                let v = scale(src[(ph >> 12) as usize] as i8, vol_l);
                                *l = l.wrapping_add(v);
                                *r = r.wrapping_add(v);
                                ph += step_u64;
                            }
                        } else {
                            for (l, r) in al.iter_mut().zip(ar.iter_mut()) {
                                let s = src[(ph >> 12) as usize] as i8;
                                *l = l.wrapping_add(scale(s, vol_l));
                                *r = r.wrapping_add(scale(s, vol_r));
                                ph += step_u64;
                            }
                        }
                        let new_pos = (ph >> 12) as usize;
                        rem -= (new_pos - pos) as i32;
                        pos = new_pos;
                        frac = (ph & 0x0FFF) as u32;
                    }

                    offset += run;
                    if rem <= 0 {
                        if let Some(ls) = loop_start {
                            pos = ls;
                            rem = sample_len.saturating_sub(ls).min(i32::MAX as usize) as i32;
                            if pos >= pcm_len {
                                voice.active = false;
                                break;
                            }
                        } else {
                            voice.active = false;
                            break;
                        }
                    } else if pos >= pcm_len {
                        voice.active = false;
                        break;
                    }
                    continue;
                }

                // Boundary fallback
                let s = pcm_bytes[pos] as i8;
                acc_l[offset] = acc_l[offset].wrapping_add(scale(s, vol_l));
                acc_r[offset] = acc_r[offset].wrapping_add(scale(s, vol_r));
                offset += 1;

                frac += step;
                let adv = frac >> 12;
                if adv != 0 {
                    frac &= 0x0FFF;
                    rem = rem.saturating_sub(adv as i32);
                    if rem > 0 {
                        pos += adv as usize;
                    } else if let Some(ls) = loop_start {
                        pos = ls;
                        rem = sample_len.saturating_sub(ls).min(i32::MAX as usize) as i32;
                    } else {
                        voice.active = false;
                        break;
                    }
                }
            }
        } else {
            // Reverse stereo playback
            while offset < frames {
                if pos >= pcm_len {
                    voice.active = false;
                    break;
                }

                if step > 0 && frac < 0x1000 && rem > 0 {
                    // Maximum samples readable in reverse: `0..=pos` has `pos + 1` samples
                    let safe_span = (rem as usize).min(pos + 1);
                    let tf0 = frac as u64;
                    let limit_tf = (safe_span as u64) << 12;
                    let k_max = (limit_tf - 1 - tf0) / step_u64;
                    let run = ((k_max + 1) as usize).min(frames - offset);
                    let al = &mut acc_l[offset..offset + run];
                    let ar = &mut acc_r[offset..offset + run];
                    let src = &pcm_bytes[..pos + 1];
                    let pos0 = pos;
                    let mut tf = tf0;

                    if center_pan {
                        for (l, r) in al.iter_mut().zip(ar.iter_mut()) {
                            let idx = pos0 - (tf >> 12) as usize;
                            let v = scale(src[idx] as i8, vol_l);
                            *l = l.wrapping_add(v);
                            *r = r.wrapping_add(v);
                            tf += step_u64;
                        }
                    } else {
                        for (l, r) in al.iter_mut().zip(ar.iter_mut()) {
                            let idx = pos0 - (tf >> 12) as usize;
                            let s = src[idx] as i8;
                            *l = l.wrapping_add(scale(s, vol_l));
                            *r = r.wrapping_add(scale(s, vol_r));
                            tf += step_u64;
                        }
                    }

                    let adv = (tf >> 12) as usize;
                    frac = (tf & 0x0FFF) as u32;
                    rem -= adv as i32;
                    offset += run;

                    if rem <= 0 || pos < adv {
                        voice.active = false;
                        break;
                    }
                    pos -= adv;
                    continue;
                }

                let s = pcm_bytes[pos] as i8;
                acc_l[offset] = acc_l[offset].wrapping_add(scale(s, vol_l));
                acc_r[offset] = acc_r[offset].wrapping_add(scale(s, vol_r));
                offset += 1;

                frac += step;
                let adv = frac >> 12;
                if adv != 0 {
                    frac &= 0x0FFF;
                    rem = rem.saturating_sub(adv as i32);
                    if rem > 0 {
                        if pos < adv as usize {
                            voice.active = false;
                            break;
                        }
                        pos -= adv as usize;
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
        match version {
            KonamiKceVersion::KcejV2StereoLut => {
                if voice.reverse {
                    // Authentic KCEJ v2 ARM reverse-path quirk (`08027454: ldrsb r1, [r5, -r0]!`):
                    // sign-extending a negative sample before `[lut_base + vol*256 + r1]`
                    // indexes 256 bytes earlier, i.e. the `vol - 1` LUT row.
                    Self::mix_kcej_stereo_impl(pcm_bytes, voice, acc_l, acc_r, |sample, vol| {
                        let eff_vol = if sample < 0 {
                            vol.saturating_sub(1)
                        } else {
                            vol
                        };
                        kcej_v2_lut_fast(sample, eff_vol)
                    });
                } else {
                    Self::mix_kcej_stereo_impl(pcm_bytes, voice, acc_l, acc_r, kcej_v2_lut_fast);
                }
            }
            KonamiKceVersion::KcejV3StereoAsr4Bias => {
                Self::mix_kcej_stereo_impl(pcm_bytes, voice, acc_l, acc_r, |sample, vol| {
                    let mut v = ((sample as i32) * (vol as i32)) >> 4;
                    if v < 0 {
                        v += 1;
                    }
                    v as i16
                });
            }
            KonamiKceVersion::KcejV4StereoAsr7 => {
                Self::mix_kcej_stereo_impl(pcm_bytes, voice, acc_l, acc_r, |sample, vol| {
                    (((sample as i32) * (vol as i32)) >> 7) as i16
                });
            }
            KonamiKceVersion::KcejV5StereoAsr7Bias => {
                Self::mix_kcej_stereo_impl(pcm_bytes, voice, acc_l, acc_r, |sample, vol| {
                    let mut p = (sample as i32) * (vol as i32);
                    if p < 0 {
                        p += 0x80;
                    }
                    (p >> 7) as i16
                });
            }
            _ => {
                Self::mix_kcej_stereo_impl(pcm_bytes, voice, acc_l, acc_r, |sample, vol| {
                    (((sample as i32) * (vol as i32)) >> 7) as i16
                });
            }
        }
    }

    /// Exact KCEJ v2–v5 stereo DirectSound unpack (`strb` low-8-bit truncation, no clamp).
    #[inline]
    pub fn pack_kcej_stereo_to_dma8(
        acc_l: &[i16],
        acc_r: &[i16],
        dma_l: &mut [i8],
        dma_r: &mut [i8],
    ) {
        let n = acc_l.len().min(acc_r.len()).min(dma_l.len()).min(dma_r.len());
        for (d, &a) in dma_l[..n].iter_mut().zip(&acc_l[..n]) {
            *d = a as i8;
        }
        for (d, &a) in dma_r[..n].iter_mut().zip(&acc_r[..n]) {
            *d = a as i8;
        }
    }
}

// ============================================================================
// StudioSynth: 48,000 Hz f32 Band-Limited Lanczos-3 + Equal-Power Panning
// ============================================================================

/// Number of fractional polyphase steps in [`StudioSynth`]'s compile-time Lanczos-3 LUT.
///
/// 1,024 phases (24.6 KB, fits comfortably in L1 data cache) provides >75 dB SNR (far
/// exceeding 8-bit PCM's 48 dB floor) with a single 6-tap dot product per output frame.
const LANCZOS_PHASES: usize = 1024;

const fn sin_const(mut x: f64) -> f64 {
    const PI: f64 = core::f64::consts::PI;
    const TWO_PI: f64 = core::f64::consts::TAU;
    const HALF_PI: f64 = core::f64::consts::FRAC_PI_2;
    let k = (x / TWO_PI) as i64;
    x -= (k as f64) * TWO_PI;
    if x > PI {
        x -= TWO_PI;
    } else if x < -PI {
        x += TWO_PI;
    }
    if x > HALF_PI {
        x = PI - x;
    } else if x < -HALF_PI {
        x = -PI - x;
    }
    let r2 = x * x;
    x * (1.0
        - r2 * (1.0 / 6.0
            - r2 * (1.0 / 120.0
                - r2 * (1.0 / 5_040.0
                    - r2 * (1.0 / 362_880.0
                        - r2 * (1.0 / 39_916_800.0 - r2 / 6_227_020_800.0))))))
}

const fn lanczos_kernel(x: f64, a: f64) -> f64 {
    let ax = if x < 0.0 { -x } else { x };
    if ax < 1e-12 {
        return 1.0;
    }
    if ax >= a {
        return 0.0;
    }
    let pi_x = core::f64::consts::PI * x;
    let pi_x_a = pi_x / a;
    (sin_const(pi_x) / pi_x) * (sin_const(pi_x_a) / pi_x_a)
}

const fn build_lanczos3_lut() -> [[f32; 6]; LANCZOS_PHASES + 1] {
    let mut lut = [[0.0f32; 6]; LANCZOS_PHASES + 1];
    let mut p = 0usize;
    while p <= LANCZOS_PHASES {
        let frac = (p as f64) / (LANCZOS_PHASES as f64);
        let mut weights = [0.0f64; 6];
        let mut sum = 0.0f64;
        let mut t = 0usize;
        while t < 6 {
            let k = (t as i32) - 2;
            let w = lanczos_kernel(frac - (k as f64), 3.0);
            weights[t] = w;
            sum += w;
            t += 1;
        }
        let inv_sum = if sum > 1e-12 { 1.0 / sum } else { 0.0 };
        let mut j = 0usize;
        while j < 6 {
            lut[p][j] = (weights[j] * inv_sum) as f32;
            j += 1;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StudioSynth {
    /// Output sample rate in Hz (default [`Self::SAMPLE_RATE`] = `48_000`).
    pub sample_rate: u32,
    /// Number of Lanczos kernel lobes (default `3`, using a compile-time 1,024-phase
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
    /// Lanczos windowed-sinc interpolation and true equal-power stereo panning.
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
            || !volume.is_finite()
            || !pan.is_finite()
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

        if self.sinc_lobes == 3 {
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
                if pcm_len >= 6 && base >= 2 && base <= safe_max_base && step < (1u64 << 31) as f64
                {
                    let safe_limit = (safe_max_base as f64) + 0.999_999;
                    let max_interior = (((safe_limit - ph) / step) as usize).saturating_add(1);
                    let run = max_interior.min(frames - idx);
                    if run >= 4 {
                        const Q32_SCALE: f64 = 4_294_967_296.0;
                        let ph_q32 = (ph * Q32_SCALE) as u64;
                        let step_q32 = (step * Q32_SCALE) as u64;
                        let last_base = ((ph_q32
                            .saturating_add(step_q32.saturating_mul((run - 1) as u64)))
                            >> 32) as usize;
                        if last_base <= safe_max_base {
                            let src = &pcm[..last_base + 4];
                            let mut ph_shifted = ph_q32 - (2u64 << 32);
                            for (ol, or) in out_l[idx..idx + run]
                                .iter_mut()
                                .zip(&mut out_r[idx..idx + run])
                            {
                                let b0 = (ph_shifted >> 32) as usize;
                                // Nearest 1024-phase index in `0..=1024`
                                let p = (((ph_shifted & 0xFFFF_FFFF) + (1 << 21)) >> 22) as usize;
                                let w = &LANCZOS3_LUT[p];
                                let win = &src[b0..b0 + 6];
                                let s0 = (win[0] as i8) as f32;
                                let s1 = (win[1] as i8) as f32;
                                let s2 = (win[2] as i8) as f32;
                                let s3 = (win[3] as i8) as f32;
                                let s4 = (win[4] as i8) as f32;
                                let s5 = (win[5] as i8) as f32;
                                // Balanced reduction tree halves FMA dependency chain latency
                                let val = (s0 * w[0] + s1 * w[1])
                                    + (s2 * w[2] + s3 * w[3])
                                    + (s4 * w[4] + s5 * w[5]);
                                *ol += val * gain_l;
                                *or += val * gain_r;
                                ph_shifted += step_q32;
                            }
                            ph += (run as f64) * step;
                            idx += run;
                            continue;
                        }
                    }
                }

                // Boundary / loop-edge single-sample step
                let frac = ph - (base as f64);
                let p = ((frac * (LANCZOS_PHASES as f64) + 0.5) as usize).min(LANCZOS_PHASES);
                let w = &LANCZOS3_LUT[p];

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

                let val = s[0] * w[0]
                    + s[1] * w[1]
                    + s[2] * w[2]
                    + s[3] * w[3]
                    + s[4] * w[4]
                    + s[5] * w[5];

                out_l[idx] += val * gain_l;
                out_r[idx] += val * gain_r;
                ph += step;
                idx += 1;
            }

            *pos_f64 = ph;
            return;
        }

        // General fallback for custom `sinc_lobes != 3`
        let a = self.sinc_lobes.clamp(1, 64) as isize;
        let a_f64 = a as f64;
        let mut ph = *pos_f64;

        for (ol, or) in out_l.iter_mut().zip(out_r.iter_mut()) {
            if ph >= pcm_len_f {
                if looped {
                    ph = loop_start_f + ((ph - loop_start_f) % loop_len_f);
                } else {
                    break;
                }
            }
            let center = ph as isize;
            let mut acc = 0.0f64;
            let mut wsum = 0.0f64;

            for k in (center - a + 1)..=(center + a) {
                let s_val = if k < 0 {
                    0.0
                } else if (k as usize) < pcm_len {
                    (pcm[k as usize] as i8) as f64
                } else if looped {
                    let wrapped = loop_start + (((k as usize) - loop_start) % loop_len);
                    (pcm[wrapped] as i8) as f64
                } else {
                    0.0
                };
                let w = lanczos_kernel(ph - (k as f64), a_f64);
                acc += s_val * w;
                wsum += w;
            }

            let val = if wsum.abs() > 1e-9 {
                (acc / wsum) as f32
            } else {
                0.0
            };

            *ol += val * gain_l;
            *or += val * gain_r;
            ph += step;
        }

        *pos_f64 = ph;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_studio_synth_lut_matches_direct_lanczos3() {
        let pcm: [u8; 64] = core::array::from_fn(|i| {
            let angle = (i as f64) * 0.35;
            (sin_const(angle) * 100.0) as i8 as u8
        });
        let view = KonamiSampleView::from_pcm_bytes(0, 13_379, 60, None, &pcm);
        let synth = StudioSynth::new(48_000);
        let mut left = [0.0f32; 128];
        let mut right = [0.0f32; 128];
        let mut pos = 5.25;
        synth.render_voice_f32(&view, &mut pos, 12_000.0, 1.0, 0.0, &mut left, &mut right);

        let step = 12_000.0 / 48_000.0;
        let gain_l = ((1.0 / 128.0) * cos_f64(core::f64::consts::FRAC_PI_4)) as f32;
        let mut ref_pos = 5.25f64;
        for &actual in &left {
            let center = ref_pos as isize;
            let mut acc = 0.0f64;
            let mut wsum = 0.0f64;
            for k in (center - 2)..=(center + 3) {
                let s = (pcm[k as usize] as i8) as f64;
                let w = lanczos_kernel(ref_pos - (k as f64), 3.0);
                acc += s * w;
                wsum += w;
            }
            let expected = ((acc / wsum) as f32) * gain_l;
            assert!(
                (actual - expected).abs() < 1e-3,
                "LUT mismatch: actual={actual}, expected={expected}"
            );
            ref_pos += step;
        }
    }

    #[test]
    fn test_kcej_v2_lut_matches_formula() {
        for vol in 0u8..16 {
            for b in 0u8..=255 {
                let s = b as i8;
                assert_eq!(
                    kcej_v2_lut_fast(s, vol),
                    AccuracySynth::kcej_v2_volume_lut_entry(s, vol) as i16
                );
            }
        }
    }

    #[test]
    fn test_malformed_inputs_do_not_panic_or_poison() {
        let pcm = [0u8, 32, 64, 96, 127, 96, 64, 32];
        let view = KonamiSampleView::from_pcm_bytes(0, 13_379, 60, None, &pcm);
        let synth = StudioSynth::new(48_000);
        let mut l = [0.0f32; 8];
        let mut r = [0.0f32; 8];
        let mut pos = 2.0;
        synth.render_voice_f32(&view, &mut pos, 12_000.0, f32::NAN, 0.0, &mut l, &mut r);
        synth.render_voice_f32(&view, &mut pos, 12_000.0, 1.0, f32::NAN, &mut l, &mut r);
        synth.render_voice_f32(&view, &mut pos, 1e-300, 1.0, 0.0, &mut l, &mut r);
        assert!(l.iter().all(|x| x.is_finite()));
        assert!(r.iter().all(|x| x.is_finite()));

        let mut voice = KcejVoice::new(0, i32::MIN, 0x1000, 0, 15, 15, None, pcm.len(), false);
        let mut out = [0i8; 8];
        AccuracySynth::mix_kcej_v1_mono8(&pcm, &mut voice, &mut out);
    }
}

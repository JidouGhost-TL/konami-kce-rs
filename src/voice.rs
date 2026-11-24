//! Voice state structures matching the exact ARM7TDMI mixer structs for Konami KCEH
//! and Konami KCEJ sound drivers.

/// KCEH / Cross-Studio ARM mixer kernel selection (4 hardware routines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum KcehMixMode {
    /// Stereo 12-bit linear interpolation (`0x08043C08` in *Frogger's Adventures 2*).
    #[default]
    StereoLinear,
    /// Stereo nearest-neighbor (`0x08043DE8`).
    StereoNearest,
    /// Mono 12-bit linear interpolation (`0x08043F74`).
    MonoLinear,
    /// Mono nearest-neighbor (`0x08044128`).
    MonoNearest,
}

/// Active voice state for the Konami KCEH / Cross-Studio 32-byte soundbank ARM mixer.
///
/// Mirrors the exact fields read by the KCEH ARM7TDMI mixer (`r0`):
/// - `+0x04`: `cur` (sample index)
/// - `+0x08`: `end` (exclusive forward end, or reverse stop bound `< cur`)
/// - `+0x0C`: `loop_start` (`None` = one-shot)
/// - `+0x10`: `loop_len`
/// - `+0x14`: `step` (Q4.12 fixed-point step, `0x1000 = 1.0x`)
/// - `+0x16`: `frac` (12-bit fractional phase `0..=0x0FFF`)
/// - `+0x18`: `vol_l` (`0..=255`; non-zero values add `+1` in hardware)
/// - `+0x19`: `vol_r` (`0..=255`; non-zero values add `+1` in hardware)
/// - `+0x1A`: `phase_invert` (when `true`, negates right volume for surround/QSound effect)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KcehVoice {
    /// Current sample index into the PCM buffer.
    pub cur: usize,
    /// Exclusive forward end index, or reverse stop bound (`< cur`).
    pub end: usize,
    /// Loop start index (`None` for one-shot playback).
    pub loop_start: Option<usize>,
    /// Loop length in samples.
    pub loop_len: usize,
    /// Q4.12 fixed-point phase increment (`0x1000 = 1.0x`).
    pub step: u16,
    /// 12-bit fractional phase accumulator (`0..=0x0FFF`).
    pub frac: u16,
    /// Left channel volume (`0..=255`; non-zero values add `+1` in hardware).
    pub vol_l: u8,
    /// Right channel volume (`0..=255`; non-zero values add `+1` in hardware).
    pub vol_r: u8,
    /// When `true`, negates the right-channel volume for surround/QSound phase inversion.
    pub phase_invert: bool,
    /// Whether this voice is currently active.
    pub active: bool,
}

impl KcehVoice {
    /// Create a new forward-playing KCEH voice.
    #[inline]
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new_forward(
        cur: usize,
        end: usize,
        loop_start: Option<usize>,
        loop_len: usize,
        step: u16,
        frac: u16,
        vol_l: u8,
        vol_r: u8,
        phase_invert: bool,
    ) -> Self {
        Self {
            cur,
            end,
            loop_start,
            loop_len,
            step,
            frac,
            vol_l,
            vol_r,
            phase_invert,
            active: true,
        }
    }

    /// Create a new reverse-playing KCEH voice (`cur > end`).
    #[inline]
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new_reverse(
        cur: usize,
        end: usize,
        loop_start: Option<usize>,
        loop_len: usize,
        step: u16,
        frac: u16,
        vol_l: u8,
        vol_r: u8,
        phase_invert: bool,
    ) -> Self {
        Self {
            cur,
            end,
            loop_start,
            loop_len,
            step,
            frac,
            vol_l,
            vol_r,
            phase_invert,
            active: true,
        }
    }
}

/// Active voice state for Konami KCEJ ARM7TDMI mixers (`v1`–`v5`).
///
/// Mirrors the exact fields read by KCEJ ARM7TDMI mixers (`r7`):
/// - `+0x00`: `pos` (current sample index)
/// - `+0x04`: `rem` (remaining sample count before loop or stop)
/// - `+0x08`: `step` (Q4.12 fixed-point phase step, `0x1000 = 1.0x`)
/// - `+0x0A`: `frac` (12-bit fractional phase `0..=0x0FFF`)
/// - `+0x0E`: `flags` (bit 7 = looped, bit 1 = reverse)
/// - `+0x10`: `vol_r` (Right channel volume; for v1 mono, used as mono `vol`)
/// - `+0x11`: `vol_l` (Left channel volume)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KcejVoice {
    /// Current sample index into the PCM buffer.
    pub pos: usize,
    /// Remaining sample count before loop wrap or voice stop.
    pub rem: i32,
    /// Q4.12 fixed-point phase increment (`0x1000 = 1.0x`).
    pub step: u16,
    /// 12-bit fractional phase accumulator (`0..=0x0FFF`).
    pub frac: u16,
    /// Left channel volume.
    pub vol_l: u8,
    /// Right channel volume (also used as mono volume in KCEJ v1).
    pub vol_r: u8,
    /// Loop start index (`None` for one-shot playback).
    pub loop_start: Option<usize>,
    /// Total sample length in bytes.
    pub sample_len: usize,
    /// Whether this voice plays in reverse.
    pub reverse: bool,
    /// Whether this voice is currently active.
    pub active: bool,
}

impl KcejVoice {
    /// Create a new KCEJ voice starting at `pos` with `rem` remaining samples.
    #[inline]
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        pos: usize,
        rem: i32,
        step: u16,
        frac: u16,
        vol_l: u8,
        vol_r: u8,
        loop_start: Option<usize>,
        sample_len: usize,
        reverse: bool,
    ) -> Self {
        Self {
            pos,
            rem,
            step,
            frac,
            vol_l,
            vol_r,
            loop_start,
            sample_len,
            reverse,
            active: true,
        }
    }
}

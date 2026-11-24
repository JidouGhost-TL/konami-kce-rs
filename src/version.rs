//! Konami KCE (KCEH & KCEJ) engine revision detection and mixer variant metadata.
//!
//! A full-corpus audit of all 3,377 GBA ROMs identifies **6 distinct ARM7TDMI mixer
//! revisions** spanning **216 commercial GBA releases**:
//!
//! 1. [`KonamiKceVersion::KcehRelocBank32`] (**172 ROMs** — 21 Konami KCEH titles such as
//!    *Frogger's Adventures 1 & 2*, *Frogger's Journey*, *Jurassic Park III: The DNA Factor*,
//!    *Konami Collector's Series*, *Motocross Maniacs Advance*, plus 151 cross-studio titles
//!    sharing the exact same ARM7TDMI mixer & 32-byte soundbank container including Eurocom's
//!    *Harry Potter 1 & 2*, Helixe/THQ's *Star Wars: The New Droid Army* & *Rugrats*, EA,
//!    Midway, and TDK):
//!    - 128-sample blocks, 12-bit fractional linear or nearest-neighbor interpolation,
//!      forward & reverse playback, optional right-channel phase inversion, interleaved `i16`
//!      accumulation (`(s * (vol + 1)) >> 8`), and saturating `[-128, +127]` DirectSound clamp.
//! 2. [`KonamiKceVersion::KcejV1Mono8`] (**14 ROMs** — *Yu-Gi-Oh! Eternal Duelist Soul*,
//!    *Yu-Gi-Oh! Worldwide Edition*, *Yu-Gi-Oh! DM6*, *Hikaru no Go 1*, *Captain Tsubasa*,
//!    *Groove Adventure Rave 1*, *Prince of Tennis 2002*, *Kami no Kijutsu*):
//!    - 3 mono channels, direct wrapping 8-bit accumulation, `(s * (vol + 1)) >> 4`,
//!      12-bit fractional nearest-neighbor stepping, dedicated `step == 0x0800` (0.5x) fast path.
//! 3. [`KonamiKceVersion::KcejV2StereoLut`] (**9 ROMs** — *Hunter X Hunter*, *Prince of
//!    Tennis 2003 Red/Blue*, *Groove Adventure Rave 2*, *Hikaru no Go 2*):
//!    - 8 stereo channels, 16-sample DMA blocks, 256-byte signed volume lookup table
//!      (`round_half_away_from_zero(s * vol / 15)` clamped to `i8`), forward & reverse playback,
//!      truncating `i16 -> i8` DirectSound output.
//! 4. [`KonamiKceVersion::KcejV3StereoAsr4Bias`] (**5 ROMs** — *Yu-Gi-Oh! World Championship
//!    Tournament 2004 / Expert 3*, *Mermaid Melody Pichi Pichi Pitch*, *Minna no Ouji-sama*):
//!    - 12 stereo channels, 16-sample DMA blocks, `asrs #4` followed by `addmi #1` negative bias,
//!      center-pan fast path, reverse playback, truncating `i16 -> i8` output.
//! 5. [`KonamiKceVersion::KcejV4StereoAsr7`] (**14 ROMs** — *Shaman King: Master of Spirits 2*,
//!    *Prince of Tennis 2004 Gold/Silver*, *Mermaid Melody Live Start*, etc.):
//!    - 12 stereo channels, 16-sample DMA blocks, `(s * vol) >> 7` (`asr #7`, no negative bias),
//!      center-pan fast path, reverse playback, truncating `i16 -> i8` output.
//! 6. [`KonamiKceVersion::KcejV5StereoAsr7Bias`] (**3 ROMs** — *Yu-Gi-Oh! Ultimate Masters:
//!    World Championship Tournament 2006* USA/Europe/Japan):
//!    - 12 stereo channels, 16-sample DMA blocks, `muls` + `addmi #0x80` + `asr #7`,
//!      center-pan fast path, reverse playback, truncating `i16 -> i8` output.

/// High-level Konami KCE driver sub-family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KonamiKceFamily {
    /// Konami Computer Entertainment Hawaii / Cross-Studio 32-byte relocatable soundbank engine.
    Kceh,
    /// Konami Computer Entertainment Japan pointer-table DirectSound engine.
    Kcej,
}

/// Distinct Konami KCE ARM7TDMI mixer revisions discovered across the GBA ROM corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum KonamiKceVersion {
    /// KCEH / Cross-Studio 32-byte soundbank engine (172 ROMs): 128-sample blocks,
    /// stereo/mono linear & nearest-neighbor ARM mixers, `(s * (vol + 1)) >> 8`,
    /// forward/reverse playback, phase-invert surround, and saturating `[-128, +127]` clamp.
    #[default]
    KcehRelocBank32,
    /// KCEJ Rev 1 (14 ROMs, e.g. *Yu-Gi-Oh! EDS/WWE*, *Hikaru no Go 1*):
    /// 3-channel mono direct wrapping `i8` mixer, `(s * (vol + 1)) >> 4`, `0x0800` fast path.
    KcejV1Mono8,
    /// KCEJ Rev 2 (9 ROMs, e.g. *Hunter X Hunter*, *Prince of Tennis 2003*):
    /// 8-channel stereo 16-sample mixer using 256-byte signed volume LUT (`round(s * v / 15)`).
    KcejV2StereoLut,
    /// KCEJ Rev 3 (5 ROMs, e.g. *Yu-Gi-Oh! WCT 2004*, *Mermaid Melody*):
    /// 12-channel stereo 16-sample mixer with `asrs #4; addmi #1` negative-bias rounding.
    KcejV3StereoAsr4Bias,
    /// KCEJ Rev 4 (14 ROMs, e.g. *Shaman King: Master of Spirits 2*, *Prince of Tennis 2004*):
    /// 12-channel stereo 16-sample mixer with `(s * vol) >> 7` (`asr #7`, unbiased).
    KcejV4StereoAsr7,
    /// KCEJ Rev 5 (3 ROMs, *Yu-Gi-Oh! Ultimate Masters WCT 2006*):
    /// 12-channel stereo 16-sample mixer with `muls; addmi #0x80; asr #7` pre-shift negative bias.
    KcejV5StereoAsr7Bias,
}

impl KonamiKceVersion {
    /// Returns the sub-family ([`KonamiKceFamily::Kceh`] or [`KonamiKceFamily::Kcej`]).
    #[inline]
    #[must_use]
    pub const fn family(self) -> KonamiKceFamily {
        match self {
            Self::KcehRelocBank32 => KonamiKceFamily::Kceh,
            Self::KcejV1Mono8
            | Self::KcejV2StereoLut
            | Self::KcejV3StereoAsr4Bias
            | Self::KcejV4StereoAsr7
            | Self::KcejV5StereoAsr7Bias => KonamiKceFamily::Kcej,
        }
    }

    /// Maximum simultaneous software mixer channels for this revision.
    #[inline]
    #[must_use]
    pub const fn max_channels(self) -> u8 {
        match self {
            Self::KcehRelocBank32 => 16,
            Self::KcejV1Mono8 => 3,
            Self::KcejV2StereoLut => 8,
            Self::KcejV3StereoAsr4Bias | Self::KcejV4StereoAsr7 | Self::KcejV5StereoAsr7Bias => 12,
        }
    }

    /// Native hardware DMA mixing block length in samples per mixer invocation.
    #[inline]
    #[must_use]
    pub const fn block_frames(self) -> usize {
        match self {
            Self::KcehRelocBank32 => 128,
            Self::KcejV1Mono8 => 704,
            Self::KcejV2StereoLut
            | Self::KcejV3StereoAsr4Bias
            | Self::KcejV4StereoAsr7
            | Self::KcejV5StereoAsr7Bias => 16,
        }
    }

    /// Whether this revision mixes in stereo (DirectSound A + B).
    #[inline]
    #[must_use]
    pub const fn is_stereo(self) -> bool {
        !matches!(self, Self::KcejV1Mono8)
    }

    /// Whether the hardware DMA conversion saturating-clamps `i16` to `[-128, +127]`
    /// (`true` for KCEH) or truncates low 8 bits (`false` for KCEJ).
    #[inline]
    #[must_use]
    pub const fn clamps_output(self) -> bool {
        matches!(self, Self::KcehRelocBank32)
    }

    /// Arithmetic right-shift applied to `sample * volume` (`None` for LUT-based KCEJ v2).
    #[inline]
    #[must_use]
    pub const fn volume_shift(self) -> Option<u32> {
        match self {
            Self::KcehRelocBank32 => Some(8),
            Self::KcejV1Mono8 | Self::KcejV3StereoAsr4Bias => Some(4),
            Self::KcejV2StereoLut => None,
            Self::KcejV4StereoAsr7 | Self::KcejV5StereoAsr7Bias => Some(7),
        }
    }

    /// Whether this revision scales samples via a 256-byte ROM volume lookup table.
    #[inline]
    #[must_use]
    pub const fn uses_volume_lut(self) -> bool {
        matches!(self, Self::KcejV2StereoLut)
    }

    /// Detect the exact Konami KCE ARM mixer revision directly from a GBA ROM or IWRAM image.
    ///
    /// Dispatches KCEH before KCEJ because some KCEH builds share KCEJ DMA constants while
    /// using the KCEH ARM mixer and 32-byte relocatable soundbank container.
    #[must_use]
    pub fn detect_from_bytes(data: &[u8]) -> Option<Self> {
        // 1. KCEH / Cross-Studio 32-byte soundbank ARM mixer signature:
        //    ldrb r4, [r0, #0x18] (0xE5D04018)
        //    ldrb r5, [r0, #0x19] (0xE5D05019)
        //    cmp  r4, #0          (0xE3540000)
        //    addne r4, r4, #1     (0x12844001)
        const KCEH_SIG: [u8; 16] = [
            0x18, 0x40, 0xD0, 0xE5, 0x19, 0x50, 0xD0, 0xE5, 0x00, 0x00, 0x54, 0xE3, 0x01, 0x40,
            0x84, 0x12,
        ];
        if find_subslice(data, &KCEH_SIG).is_some() {
            return Some(Self::KcehRelocBank32);
        }

        // 2. KCEJ shared 12-bit fractional phase step:
        //    add  r3, r3, r2      (0xE0833002)
        //    lsrs r0, r3, #12     (0xE1B00623)
        const KCEJ_PHASE_SIG: [u8; 8] = [0x02, 0x30, 0x83, 0xE0, 0x23, 0x06, 0xB0, 0xE1];
        let pos = find_subslice(data, &KCEJ_PHASE_SIG)?;
        let win_start = pos.saturating_sub(0x48);
        let win_end = (pos + 0x18).min(data.len());
        let window = &data[win_start..win_end];

        // Distinguish the 5 KCEJ ARM mixer revisions by their exact inner-loop scaling opcodes:
        // - V5: `addmi fp, fp, #0x80` (0x428BB080) + `asr fp, fp, #7` (0xE1A0B3CB)
        if find_subslice(window, &0x428B_B080u32.to_le_bytes()).is_some() {
            return Some(Self::KcejV5StereoAsr7Bias);
        }
        // - V4: `asr fp, fp, #7` (0xE1A0B3CB) without `addmi #0x80`
        if find_subslice(window, &0xE1A0_B3CBu32.to_le_bytes()).is_some() {
            return Some(Self::KcejV4StereoAsr7);
        }
        // - V3: `asrs fp, fp, #4` (0xE1B0B24B) + `addmi fp, fp, #1` (0x428BB001)
        if find_subslice(window, &0x428B_B001u32.to_le_bytes()).is_some() {
            return Some(Self::KcejV3StereoAsr4Bias);
        }
        // - V2: `ldrsb fp, [sb, r1]` (0xE199B0D1) + `ldrsb r1, [r8, r1]` (0xE19810D1)
        if find_subslice(window, &0xE199_B0D1u32.to_le_bytes()).is_some() {
            return Some(Self::KcejV2StereoLut);
        }
        // - V1: `mul r1, r8, r1` (0xE0010198) + `asr r1, r1, #4` (0xE1A01241) + `strb r0, [r6], #1` (0xE4C60001)
        if find_subslice(window, &0xE4C6_0001u32.to_le_bytes()).is_some() {
            return Some(Self::KcejV1Mono8);
        }

        None
    }
}

#[inline]
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_all_six_versions() {
        // KCEH
        let kceh = [
            0x18, 0x40, 0xD0, 0xE5, 0x19, 0x50, 0xD0, 0xE5, 0x00, 0x00, 0x54, 0xE3, 0x01, 0x40,
            0x84, 0x12,
        ];
        assert_eq!(
            KonamiKceVersion::detect_from_bytes(&kceh),
            Some(KonamiKceVersion::KcehRelocBank32)
        );

        let phase = [0x02u8, 0x30, 0x83, 0xE0, 0x23, 0x06, 0xB0, 0xE1];

        let mut v1 = [0u8; 24];
        v1[0..4].copy_from_slice(&0xE4C6_0001u32.to_le_bytes());
        v1[8..16].copy_from_slice(&phase);
        assert_eq!(
            KonamiKceVersion::detect_from_bytes(&v1),
            Some(KonamiKceVersion::KcejV1Mono8)
        );

        let mut v2 = [0u8; 24];
        v2[0..4].copy_from_slice(&0xE199_B0D1u32.to_le_bytes());
        v2[8..16].copy_from_slice(&phase);
        assert_eq!(
            KonamiKceVersion::detect_from_bytes(&v2),
            Some(KonamiKceVersion::KcejV2StereoLut)
        );

        let mut v3 = [0u8; 24];
        v3[0..4].copy_from_slice(&0x428B_B001u32.to_le_bytes());
        v3[8..16].copy_from_slice(&phase);
        assert_eq!(
            KonamiKceVersion::detect_from_bytes(&v3),
            Some(KonamiKceVersion::KcejV3StereoAsr4Bias)
        );

        let mut v4 = [0u8; 24];
        v4[0..4].copy_from_slice(&0xE1A0_B3CBu32.to_le_bytes());
        v4[8..16].copy_from_slice(&phase);
        assert_eq!(
            KonamiKceVersion::detect_from_bytes(&v4),
            Some(KonamiKceVersion::KcejV4StereoAsr7)
        );

        let mut v5 = [0u8; 24];
        v5[0..4].copy_from_slice(&0x428B_B080u32.to_le_bytes());
        v5[4..8].copy_from_slice(&0xE1A0_B3CBu32.to_le_bytes());
        v5[8..16].copy_from_slice(&phase);
        assert_eq!(
            KonamiKceVersion::detect_from_bytes(&v5),
            Some(KonamiKceVersion::KcejV5StereoAsr7Bias)
        );
    }

    #[test]
    fn test_version_metadata() {
        assert_eq!(
            KonamiKceVersion::KcehRelocBank32.family(),
            KonamiKceFamily::Kceh
        );
        assert!(KonamiKceVersion::KcehRelocBank32.clamps_output());
        assert_eq!(KonamiKceVersion::KcehRelocBank32.block_frames(), 128);
        assert_eq!(KonamiKceVersion::KcehRelocBank32.volume_shift(), Some(8));

        assert_eq!(
            KonamiKceVersion::KcejV1Mono8.family(),
            KonamiKceFamily::Kcej
        );
        assert!(!KonamiKceVersion::KcejV1Mono8.is_stereo());
        assert!(!KonamiKceVersion::KcejV1Mono8.clamps_output());
        assert_eq!(KonamiKceVersion::KcejV1Mono8.max_channels(), 3);

        assert!(KonamiKceVersion::KcejV2StereoLut.uses_volume_lut());
        assert_eq!(KonamiKceVersion::KcejV2StereoLut.volume_shift(), None);
        assert_eq!(KonamiKceVersion::KcejV2StereoLut.max_channels(), 8);

        assert_eq!(KonamiKceVersion::KcejV3StereoAsr4Bias.volume_shift(), Some(4));
        assert_eq!(KonamiKceVersion::KcejV4StereoAsr7.volume_shift(), Some(7));
        assert_eq!(KonamiKceVersion::KcejV5StereoAsr7Bias.volume_shift(), Some(7));
        assert_eq!(KonamiKceVersion::KcejV5StereoAsr7Bias.max_channels(), 12);
    }
}

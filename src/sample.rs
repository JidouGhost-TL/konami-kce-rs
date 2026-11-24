//! Zero-allocation parsers for Konami KCEH (32-byte soundbank + 16-byte sample header)
//! and Konami KCEJ (12-byte inline sample header) signed 8-bit PCM assets.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::freq::kcej_sample_rate_hz;

/// Size of the 32-byte KCEH / Cross-Studio relocatable soundbank container header.
pub const KCEH_BANK_HEADER_SIZE: usize = 32;

/// Size of the 16-byte KCEH inline sample header.
pub const KCEH_SAMPLE_HEADER_SIZE: usize = 16;

/// Size of the 12-byte KCEJ inline sample header.
pub const KCEJ_SAMPLE_HEADER_SIZE: usize = 12;

/// GBA ROM bus base address (`0x0800_0000`).
pub const GBA_ROM_BUS_BASE: u32 = 0x0800_0000;

/// Parsed 32-byte KCEH / Cross-Studio relocatable soundbank container header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KcehBankHeader {
    /// Offset to instrument table (always `0x0000_0020`).
    pub inst_table_off: u32,
    /// Relative offset to sequence/song table.
    pub seq_table_off: u32,
    /// Relative offset to sub-table.
    pub sub_table_off: u32,
    /// Relative offset to SFX table.
    pub sfx_table_off: u32,
    /// Reserved word 0 (always `0`).
    pub reserved_0: u32,
    /// Reserved word 1 (always `0`).
    pub reserved_1: u32,
    /// Relative offset to auxiliary table.
    pub extra_table_off: u32,
    /// Relative offset to `[u32]` sample relative-offset table.
    pub sample_table_off: u32,
}

impl KcehBankHeader {
    /// Parse a 32-byte KCEH soundbank header from `data[..32]`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < KCEH_BANK_HEADER_SIZE {
            return Err(Error::UnexpectedEof);
        }
        let inst_table_off = read_u32_le(data, 0);
        let seq_table_off = read_u32_le(data, 4);
        let sub_table_off = read_u32_le(data, 8);
        let sfx_table_off = read_u32_le(data, 12);
        let reserved_0 = read_u32_le(data, 16);
        let reserved_1 = read_u32_le(data, 20);
        let extra_table_off = read_u32_le(data, 24);
        let sample_table_off = read_u32_le(data, 28);

        if inst_table_off != KCEH_BANK_HEADER_SIZE as u32 {
            return Err(Error::InvalidHeader("KCEH inst_table_off must be 0x20"));
        }
        if reserved_0 != 0 || reserved_1 != 0 {
            return Err(Error::InvalidHeader("KCEH reserved header fields must be 0"));
        }
        if sample_table_off < KCEH_BANK_HEADER_SIZE as u32 || (sample_table_off & 3) != 0 {
            return Err(Error::InvalidHeader("KCEH sample_table_off unaligned or invalid"));
        }

        Ok(Self {
            inst_table_off,
            seq_table_off,
            sub_table_off,
            sfx_table_off,
            reserved_0,
            reserved_1,
            extra_table_off,
            sample_table_off,
        })
    }
}

/// Parsed 16-byte KCEH inline sample header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KcehSampleHeader {
    /// Sample payload length in bytes (signed 8-bit PCM).
    pub sample_len: u32,
    /// Loop start offset in bytes (`-1` = one-shot; `>= 0` = looped up to `sample_len`).
    pub loop_start: i32,
    /// Native sample rate in Hz (e.g. `8192`, `11025`, `11397`).
    pub sample_rate_hz: u16,
    /// MIDI root note (`0..=127`, typically `60` = C4).
    pub root_note: u8,
    /// Fine tune / flags byte.
    pub fine_tune: u8,
    /// Reserved field (always `0`).
    pub reserved: u32,
}

impl KcehSampleHeader {
    /// Parse a 16-byte KCEH sample header from `data[..16]`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < KCEH_SAMPLE_HEADER_SIZE {
            return Err(Error::UnexpectedEof);
        }
        let sample_len = read_u32_le(data, 0);
        let loop_start = read_i32_le(data, 4);
        let sample_rate_hz = read_u16_le(data, 8);
        let root_note = data[10];
        let fine_tune = data[11];
        let reserved = read_u32_le(data, 12);

        if reserved != 0 {
            return Err(Error::InvalidHeader("KCEH sample reserved field must be 0"));
        }
        if sample_len == 0 {
            return Err(Error::InvalidHeader("KCEH sample_len must be > 0"));
        }
        if loop_start >= 0 && (loop_start as u32) >= sample_len {
            return Err(Error::InvalidLoopRange {
                loop_start: loop_start as u32,
                pcm_length: sample_len as usize,
            });
        }

        Ok(Self {
            sample_len,
            loop_start,
            sample_rate_hz,
            root_note,
            fine_tune,
            reserved,
        })
    }

    /// Loop start offset in samples, or `None` for one-shot samples.
    #[inline]
    #[must_use]
    pub const fn loop_start_opt(&self) -> Option<u32> {
        if self.loop_start >= 0 {
            Some(self.loop_start as u32)
        } else {
            None
        }
    }
}

/// Parsed 12-byte KCEJ inline sample header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KcejSampleHeader {
    /// Q4.12 fixed-point playback step (`4096 = 1.0x` driver mix rate; e.g. `0x0800 = 0.5x`).
    pub pitch_base: u32,
    /// Sample payload length in bytes (signed 8-bit PCM).
    pub sample_len: u32,
    /// Loop start offset in bytes (`-1` = one-shot; `>= 0` = looped up to `sample_len`).
    pub loop_offset: i32,
}

impl KcejSampleHeader {
    /// Parse a 12-byte KCEJ sample header from `data[..12]`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < KCEJ_SAMPLE_HEADER_SIZE {
            return Err(Error::UnexpectedEof);
        }
        let pitch_base = read_u32_le(data, 0);
        let sample_len = read_u32_le(data, 4);
        let loop_offset = read_i32_le(data, 8);

        if pitch_base == 0 {
            return Err(Error::InvalidHeader("KCEJ pitch_base must be > 0"));
        }
        if sample_len == 0 {
            return Err(Error::InvalidHeader("KCEJ sample_len must be > 0"));
        }
        if loop_offset >= 0 && (loop_offset as u32) >= sample_len {
            return Err(Error::InvalidLoopRange {
                loop_start: loop_offset as u32,
                pcm_length: sample_len as usize,
            });
        }

        Ok(Self {
            pitch_base,
            sample_len,
            loop_offset,
        })
    }

    /// Loop start offset in samples, or `None` for one-shot samples.
    #[inline]
    #[must_use]
    pub const fn loop_start_opt(&self) -> Option<u32> {
        if self.loop_offset >= 0 {
            Some(self.loop_offset as u32)
        } else {
            None
        }
    }

    /// Native playback sample rate in Hz given the driver's hardware Timer 0 mix rate.
    #[inline]
    #[must_use]
    pub fn native_rate_hz(&self, driver_mix_rate_hz: f64) -> u32 {
        kcej_sample_rate_hz(self.pitch_base, driver_mix_rate_hz)
    }
}

/// Zero-allocation borrowed view of a Konami KCE (KCEH or KCEJ) signed 8-bit PCM sample.
///
/// Both KCEH and KCEJ store raw signed 8-bit PCM (`i8`, two's complement, `0` = silence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KonamiSampleView<'a> {
    /// Sample index within its bank or pointer table.
    pub id: u32,
    /// Native sample rate in Hz.
    pub sample_rate_hz: u32,
    /// MIDI root note (`0..=127`, `60` = Middle C).
    pub root_note: u8,
    /// Loop start offset in bytes (`None` for one-shot; `loop_end == pcm_i8.len()`).
    pub loop_start: Option<u32>,
    /// Raw signed 8-bit PCM bytes (reinterpreted as `i8` during synthesis).
    pub pcm_bytes: &'a [u8],
}

impl<'a> KonamiSampleView<'a> {
    /// Construct a view directly from raw signed 8-bit PCM bytes (stored as `&[u8]`).
    #[inline]
    #[must_use]
    pub const fn from_pcm_bytes(
        id: u32,
        sample_rate_hz: u32,
        root_note: u8,
        loop_start: Option<u32>,
        pcm_bytes: &'a [u8],
    ) -> Self {
        Self {
            id,
            sample_rate_hz,
            root_note,
            loop_start,
            pcm_bytes,
        }
    }

    /// Parse a KCEH 16-byte header + inline signed 8-bit PCM payload from `blob`.
    pub fn parse_kceh(id: u32, blob: &'a [u8]) -> Result<Self> {
        let hdr = KcehSampleHeader::parse(blob)?;
        let end = KCEH_SAMPLE_HEADER_SIZE
            .checked_add(hdr.sample_len as usize)
            .ok_or(Error::UnexpectedEof)?;
        if end > blob.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            id,
            sample_rate_hz: hdr.sample_rate_hz as u32,
            root_note: hdr.root_note.min(127),
            loop_start: hdr.loop_start_opt(),
            pcm_bytes: &blob[KCEH_SAMPLE_HEADER_SIZE..end],
        })
    }

    /// Parse a KCEJ 12-byte header + inline signed 8-bit PCM payload from `blob`,
    /// resolving the native sample rate from `driver_mix_rate_hz`.
    pub fn parse_kcej(id: u32, blob: &'a [u8], driver_mix_rate_hz: f64) -> Result<Self> {
        let hdr = KcejSampleHeader::parse(blob)?;
        let end = KCEJ_SAMPLE_HEADER_SIZE
            .checked_add(hdr.sample_len as usize)
            .ok_or(Error::UnexpectedEof)?;
        if end > blob.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            id,
            sample_rate_hz: hdr.native_rate_hz(driver_mix_rate_hz),
            root_note: 60,
            loop_start: hdr.loop_start_opt(),
            pcm_bytes: &blob[KCEJ_SAMPLE_HEADER_SIZE..end],
        })
    }

    /// Length of the PCM payload in samples.
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.pcm_bytes.len()
    }

    /// Returns `true` if the PCM payload is empty.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.pcm_bytes.is_empty()
    }

    /// Read sample at index `idx` as signed `i8`.
    #[inline]
    #[must_use]
    pub fn sample_i8(&self, idx: usize) -> i8 {
        self.pcm_bytes[idx] as i8
    }

    /// Loop length in samples (`0` if one-shot).
    #[inline]
    #[must_use]
    pub const fn loop_len(&self) -> usize {
        match self.loop_start {
            Some(ls) if (ls as usize) < self.pcm_bytes.len() => self.pcm_bytes.len() - (ls as usize),
            _ => 0,
        }
    }
}

/// Owned Konami KCE sample (available with `alloc` or `std`).
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonamiSample {
    /// Sample index within its bank or pointer table.
    pub id: u32,
    /// Native sample rate in Hz.
    pub sample_rate_hz: u32,
    /// MIDI root note (`0..=127`, `60` = Middle C).
    pub root_note: u8,
    /// Loop start offset in bytes (`None` for one-shot).
    pub loop_start: Option<u32>,
    /// Owned raw signed 8-bit PCM bytes (reinterpreted as `i8` during synthesis).
    pub pcm_bytes: Vec<u8>,
}

#[cfg(feature = "alloc")]
impl KonamiSample {
    /// Borrow this owned sample as a zero-allocation [`KonamiSampleView`].
    #[inline]
    #[must_use]
    pub fn as_view(&self) -> KonamiSampleView<'_> {
        KonamiSampleView {
            id: self.id,
            sample_rate_hz: self.sample_rate_hz,
            root_note: self.root_note,
            loop_start: self.loop_start,
            pcm_bytes: &self.pcm_bytes,
        }
    }
}

#[inline]
fn read_u16_le(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

#[inline]
fn read_u32_le(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

#[inline]
fn read_i32_le(data: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_kceh_header_and_sample() {
        let mut blob = [0u8; 16 + 32];
        blob[0..4].copy_from_slice(&32u32.to_le_bytes());
        blob[4..8].copy_from_slice(&8i32.to_le_bytes());
        blob[8..10].copy_from_slice(&11_025u16.to_le_bytes());
        blob[10] = 64;
        blob[16] = 0x80; // -128
        let view = KonamiSampleView::parse_kceh(0, &blob).unwrap();
        assert_eq!(view.len(), 32);
        assert_eq!(view.sample_rate_hz, 11_025);
        assert_eq!(view.root_note, 64);
        assert_eq!(view.loop_start, Some(8));
        assert_eq!(view.loop_len(), 24);
        assert_eq!(view.sample_i8(0), -128);
    }

    #[test]
    fn test_parse_kcej_header_and_sample() {
        let mut blob = [0u8; 12 + 24];
        blob[0..4].copy_from_slice(&0x0800u32.to_le_bytes());
        blob[4..8].copy_from_slice(&24u32.to_le_bytes());
        blob[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        blob[12] = 0x7F; // +127
        let view = KonamiSampleView::parse_kcej(1, &blob, 19_996.68).unwrap();
        assert_eq!(view.len(), 24);
        assert_eq!(view.sample_rate_hz, 9_998);
        assert_eq!(view.loop_start, None);
        assert_eq!(view.sample_i8(0), 127);
    }
}

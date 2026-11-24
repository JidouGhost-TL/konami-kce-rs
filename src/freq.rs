//! GBA Timer 0 frequency math, Q4.12 fixed-point pitch stepping, and 12-TET pitch tables
//! for Konami KCE (KCEH & KCEJ) sound engines.

/// GBA master CPU clock frequency in Hz (`2^24`).
pub const GBA_CLOCK_HZ: u32 = 16_777_216;

/// KCEJ v1 (*Yu-Gi-Oh! EDS / WWE*) Timer 0 reload value (`~19,996.7 Hz`).
pub const KCEJ_TIMER0_RELOAD_19997: u16 = 0xFCB9;

/// KCEJ selectable 5-rate Timer 0 reload table (*Hunter X Hunter*, *Prince of Tennis 2003*):
/// `21,024 Hz`, `18,157 Hz`, `15,768 Hz`, `13,379 Hz`, `10,512 Hz`.
pub const KCEJ_TIMER0_RELOAD_TABLE: [u16; 5] = [0xFCE2, 0xFC64, 0xFBD8, 0xFB1A, 0xF9C4];

/// Default KCEH / Cross-Studio hardware mix rate in Hz (`13,379 Hz` / `0xFB1A`).
pub const KCEH_DEFAULT_MIX_RATE_HZ: u32 = 13_379;

/// Default KCEJ hardware mix rate in Hz (`18,157 Hz` / `0xFC64`).
pub const KCEJ_DEFAULT_MIX_RATE_HZ: u32 = 18_157;

/// Q4.12 fixed-point unit step (`1.0x` playback rate = `4096`).
pub const Q12_ONE: u16 = 0x1000;

/// KCEH / Cross-Studio 12-TET semitone multiplier table in Q4.12 fixed-point (`2^(n/12) * 4096`).
///
/// Matches the exact ROM table (`00 10 F3 10 F5 11 06 13 ...`) shared across all 172 KCEH /
/// Eurocom 32-byte soundbank titles.
pub const KCEH_SEMITONE_TABLE_Q12: [u16; 12] = [
    0x1000, // +0  semitones (1.0000x)
    0x10F3, // +1  semitone  (1.0593x)
    0x11F5, // +2  semitones (1.1223x)
    0x1306, // +3  semitones (1.1890x)
    0x1428, // +4  semitones (1.2598x)
    0x155B, // +5  semitones (1.3347x)
    0x16A0, // +6  semitones (1.4141x)
    0x17F9, // +7  semitones (1.4983x)
    0x1965, // +8  semitones (1.5872x)
    0x1AE8, // +9  semitones (1.6816x)
    0x1C82, // +10 semitones (1.7817x)
    0x1E34, // +11 semitones (1.8877x)
];

/// Compute the exact GBA DirectSound sample rate in Hz from a 16-bit Timer 0 reload value
/// (`prescaler = 1`, `REG_TM0CNT_H = 0x0080`).
#[inline]
#[must_use]
pub fn rate_from_timer_reload(reload: u16) -> f64 {
    let ticks = 65_536u32 - (reload as u32);
    (GBA_CLOCK_HZ as f64) / (ticks as f64)
}

/// Convert a desired playback frequency `freq_hz` at a given `mix_rate_hz` into a Q4.12
/// fixed-point phase step (`4096 = 1.0x`), clamped to `u16::MAX`.
#[inline]
#[must_use]
pub fn freq_to_step_q12(freq_hz: f64, mix_rate_hz: f64) -> u16 {
    if !freq_hz.is_finite() || !mix_rate_hz.is_finite() || freq_hz <= 0.0 || mix_rate_hz <= 0.0 {
        return 0;
    }
    let step = (freq_hz * 4096.0) / mix_rate_hz;
    let rounded = (step + 0.5) as u64;
    if rounded > u16::MAX as u64 {
        u16::MAX
    } else {
        rounded as u16
    }
}

/// Convert a Q4.12 fixed-point phase step (`4096 = 1.0x`) back to a playback frequency in Hz
/// at the given `mix_rate_hz`.
#[inline]
#[must_use]
pub fn step_q12_to_freq(step_q12: u16, mix_rate_hz: f64) -> f64 {
    mix_rate_hz * ((step_q12 as f64) / 4096.0)
}

/// Compute a KCEJ sample's native playback rate in Hz from its 12-byte header `pitch_base`
/// (`4096 = 1.0x` mix rate) and the driver's Timer 0 mix rate.
#[inline]
#[must_use]
pub fn kcej_sample_rate_hz(pitch_base: u32, mix_rate_hz: f64) -> u32 {
    let hz = mix_rate_hz * ((pitch_base as f64) / 4096.0);
    if !hz.is_finite() || hz <= 0.0 {
        return 0;
    }
    (hz + 0.5) as u32
}

/// Compute a KCEH Q4.12 phase step for playing a sample recorded at `sample_rate_hz` and
/// `root_note` at MIDI `target_note` on a driver running at `mix_rate_hz`.
#[inline]
#[must_use]
pub fn kceh_note_step_q12(
    sample_rate_hz: u16,
    root_note: u8,
    target_note: u8,
    mix_rate_hz: u32,
) -> u16 {
    if mix_rate_hz == 0 {
        return 0;
    }
    let delta = (target_note as i32) - (root_note as i32);
    let octaves = delta.div_euclid(12);
    let semitone = delta.rem_euclid(12) as usize;
    let mul = KCEH_SEMITONE_TABLE_Q12[semitone] as u64;
    let base = ((sample_rate_hz as u64) * mul) / (mix_rate_hz as u64);
    let shifted = if octaves >= 0 {
        base.saturating_mul(1u64 << (octaves as u32).min(15))
    } else {
        base >> ((-octaves) as u32).min(63)
    };
    if shifted > u16::MAX as u64 {
        u16::MAX
    } else {
        shifted as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timer_reload_rates() {
        let r_ygo = rate_from_timer_reload(KCEJ_TIMER0_RELOAD_19997);
        assert!((r_ygo - 19_996.68).abs() < 0.1);

        let r_13379 = rate_from_timer_reload(0xFB1A);
        assert!((r_13379 - 13_379.0).abs() < 1.0);
    }

    #[test]
    fn test_q12_roundtrip() {
        let step = freq_to_step_q12(9_998.34, 19_996.68);
        assert_eq!(step, 0x0800);
        assert!((step_q12_to_freq(0x0800, 19_996.68) - 9_998.34).abs() < 0.01);
        assert_eq!(kcej_sample_rate_hz(0x0800, 19_996.68), 9_998);
    }

    #[test]
    fn test_kceh_note_step_q12() {
        // Root note at mix rate -> 0x1000
        assert_eq!(kceh_note_step_q12(13_379, 60, 60, 13_379), 0x1000);
        // One octave up -> 0x2000
        assert_eq!(kceh_note_step_q12(13_379, 60, 72, 13_379), 0x2000);
        // One octave down -> 0x0800
        assert_eq!(kceh_note_step_q12(13_379, 60, 48, 13_379), 0x0800);
    }
}

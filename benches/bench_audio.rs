use std::hint::black_box;
use std::time::Instant;

use konami_kce::{
    AccuracySynth, KcehVoice, KcejVoice, KonamiKceVersion, KonamiSampleView, StudioSynth,
};

fn main() {
    println!("=== konami-kce-rs Audio Performance Benchmarks ===");

    let pcm: Vec<u8> = (0i32..4096)
        .map(|i| (((i * 19 + 7) % 256) - 128) as i8 as u8)
        .collect();
    let sample = KonamiSampleView::from_pcm_bytes(0, 13_379, 60, Some(512), &pcm);

    let mut acc_l_128 = [0i16; 128];
    let mut acc_r_128 = [0i16; 128];
    let mut dma_l_128 = [0i8; 128];
    let mut dma_r_128 = [0i8; 128];

    let iters = 200_000usize;

    // 1. AccuracySynth KCEH Stereo Linear (128 frames)
    let start = Instant::now();
    for _ in 0..iters {
        let mut voice = KcehVoice::new_forward(0, 4096, Some(512), 3584, 0x1345, 0, 200, 140, false);
        AccuracySynth::mix_kceh_stereo_linear(
            black_box(&pcm),
            &mut voice,
            &mut acc_l_128,
            &mut acc_r_128,
        );
        AccuracySynth::clamp_kceh_stereo_to_dma8(
            &mut acc_l_128,
            &mut acc_r_128,
            &mut dma_l_128,
            &mut dma_r_128,
        );
        black_box((&dma_l_128, &dma_r_128));
    }
    let elapsed = start.elapsed();
    println!(
        "AccuracySynth::kceh_stereo_linear + clamp (128f):   {:8.2} ns/iter ({:8.2} MSamples/sec)",
        elapsed.as_nanos() as f64 / iters as f64,
        ((iters * 128) as f64 / elapsed.as_secs_f64()) / 1_000_000.0
    );

    // 2. AccuracySynth KCEH Stereo Nearest (128 frames)
    let start = Instant::now();
    for _ in 0..iters {
        let mut voice = KcehVoice::new_forward(0, 4096, Some(512), 3584, 0x1345, 0, 200, 140, false);
        AccuracySynth::mix_kceh_stereo_nearest(
            black_box(&pcm),
            &mut voice,
            &mut acc_l_128,
            &mut acc_r_128,
        );
        AccuracySynth::clamp_kceh_stereo_to_dma8(
            &mut acc_l_128,
            &mut acc_r_128,
            &mut dma_l_128,
            &mut dma_r_128,
        );
        black_box((&dma_l_128, &dma_r_128));
    }
    let elapsed = start.elapsed();
    println!(
        "AccuracySynth::kceh_stereo_nearest + clamp (128f):  {:8.2} ns/iter ({:8.2} MSamples/sec)",
        elapsed.as_nanos() as f64 / iters as f64,
        ((iters * 128) as f64 / elapsed.as_secs_f64()) / 1_000_000.0
    );

    // 3. AccuracySynth KCEJ v1 Mono8 (128 frames)
    let mut out_v1 = [0i8; 128];
    let start = Instant::now();
    for _ in 0..iters {
        let mut voice = KcejVoice::new(0, 4096, 0x1345, 0, 12, 12, Some(512), 4096, false);
        AccuracySynth::mix_kcej_v1_mono8(black_box(&pcm), &mut voice, &mut out_v1);
        black_box(&out_v1);
    }
    let elapsed = start.elapsed();
    println!(
        "AccuracySynth::kcej_v1_mono8 (128f):                {:8.2} ns/iter ({:8.2} MSamples/sec)",
        elapsed.as_nanos() as f64 / iters as f64,
        ((iters * 128) as f64 / elapsed.as_secs_f64()) / 1_000_000.0
    );

    // 4. AccuracySynth KCEJ v4 Stereo asr #7 (128 frames)
    let start = Instant::now();
    for _ in 0..iters {
        let mut voice = KcejVoice::new(0, 4096, 0x1567, 0, 90, 110, Some(512), 4096, false);
        AccuracySynth::mix_kcej_stereo_voice(
            KonamiKceVersion::KcejV4StereoAsr7,
            black_box(&pcm),
            &mut voice,
            &mut acc_l_128,
            &mut acc_r_128,
        );
        AccuracySynth::pack_kcej_stereo_to_dma8(
            &acc_l_128,
            &acc_r_128,
            &mut dma_l_128,
            &mut dma_r_128,
        );
        black_box((&dma_l_128, &dma_r_128));
    }
    let elapsed = start.elapsed();
    println!(
        "AccuracySynth::kcej_v4_stereo_asr7 + pack (128f):   {:8.2} ns/iter ({:8.2} MSamples/sec)",
        elapsed.as_nanos() as f64 / iters as f64,
        ((iters * 128) as f64 / elapsed.as_secs_f64()) / 1_000_000.0
    );

    // 5. StudioSynth 48 kHz Lanczos-3 (1024 frames)
    let studio = StudioSynth::default();
    let mut out_l = vec![0.0f32; 1024];
    let mut out_r = vec![0.0f32; 1024];
    let studio_iters = 50_000usize;
    let start = Instant::now();
    for _ in 0..studio_iters {
        let mut pos = 0.0f64;
        studio.render_voice_f32(
            black_box(&sample),
            &mut pos,
            13_379.0,
            0.85,
            0.25,
            &mut out_l,
            &mut out_r,
        );
        black_box((&out_l, &out_r));
    }
    let elapsed = start.elapsed();
    println!(
        "StudioSynth::render_voice_f32 (48kHz Lanczos3, 1k): {:8.2} ns/iter ({:8.2} MSamples/sec)",
        elapsed.as_nanos() as f64 / studio_iters as f64,
        ((studio_iters * 1024) as f64 / elapsed.as_secs_f64()) / 1_000_000.0
    );
}

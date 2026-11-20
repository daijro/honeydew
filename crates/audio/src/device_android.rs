//! Real Android output profiles. No export renderer or project settings are changed here.
use super::*;
use oboe::{AudioApi, AudioOutputCallback, AudioOutputStreamSafe, AudioStream,
    AudioStreamAsync, AudioStreamBase, AudioStreamBuilder, AudioStreamSafe,
    DataCallbackResult, Output, PerformanceMode, SharingMode, Stereo};

pub(super) struct Callback<R: AudioRenderer> {
    state: CallbackState<R>,
    scratch: Vec<f32>,
    xruns: u64,
}

impl<R: AudioRenderer> AudioOutputCallback for Callback<R> {
    type FrameType = (f32, Stereo);

    fn on_error_before_close(&mut self, _: &mut dyn AudioOutputStreamSafe, _: oboe::Error) {
        // Reopening happens on the app thread, never Oboe's error or audio thread.
        self.state.shared.failed.store(true, Ordering::Release);
    }

    fn on_audio_ready(&mut self, stream: &mut dyn AudioOutputStreamSafe,
        data: &mut [(f32, f32)]) -> DataCallbackResult {
        let start = Instant::now();
        let frames = data.len();
        let rate = stream.get_sample_rate() as u32;
        self.state.shared.buffer_frames.store(frames as u32, Ordering::Release);
        let backend_xruns = stream.get_xrun_count().ok();
        if let Some(count) = backend_xruns {
            let count = count.max(0) as u64;
            self.state.shared.underruns.fetch_add(count.saturating_sub(self.xruns), Ordering::Relaxed);
            self.xruns = count;
        }
        // Queue-depth estimate, NOT a measured touch-to-sound latency. Routing and DAC add latency.
        let heard = clock().map(|now| now + i64::from(stream.get_buffer_size_in_frames().max(0))
            * 1_000_000_000 / i64::from(rate.max(1)));
        self.state.callback(frames, 2, rate, heard, &mut self.scratch, |at, rendered| {
            for (out, pair) in data[at..].iter_mut().zip(rendered.chunks_exact(2)) {
                let clean = |s: f32| if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
                *out = (clean(pair[0]), clean(pair[1]));
            }
        });
        // OpenSL ES may not expose xruns: report deadline misses instead, without calling them hardware xruns.
        if backend_xruns.is_none() && start.elapsed().as_secs_f64() > frames as f64 / f64::from(rate.max(1)) {
            self.state.shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
        DataCallbackResult::Continue
    }
}

pub(super) fn open<R: AudioRenderer>(mode: BufferMode, rate: u32, state: CallbackState<R>)
    -> Result<AudioStreamAsync<Output, Callback<R>>, String> {
    let native = mode == BufferMode::Native;
    let target = match mode {
        BufferMode::Fast => 256,
        BufferMode::Safe => 1024,
        BufferMode::UltraSafe => 2048,
        BufferMode::Fixed(n) => n.clamp(64, 8192) as i32,
        BufferMode::Native => 0,
    };
    let mut builder = AudioStreamBuilder::default()
        // Unspecified lets Oboe fall back to OpenSL ES if AAudio is unsupported/not recommended.
        .set_audio_api(if native { AudioApi::Unspecified } else { AudioApi::OpenSLES })
        .set_performance_mode(if native || mode == BufferMode::Fast {
            PerformanceMode::LowLatency
        } else { PerformanceMode::None })
        .set_sharing_mode(SharingMode::Shared)
        .set_sample_rate(rate as i32)
        .set_format::<f32>()
        .set_channel_count::<Stereo>();
    if target > 0 {
        builder = builder.set_frames_per_callback(target)
            .set_buffer_capacity_in_frames(target * 2);
    }
    let mut stream = builder.set_callback(Callback {
        state, scratch: vec![0.0; SUB_BLOCK * 2], xruns: 0,
    }).open_stream().map_err(|e| format!("Oboe open {mode:?}: {e:?}"))?;
    // Never silently run the DSP at a different rate from its stream.
    if stream.get_sample_rate() != rate as i32 {
        return Err(format!("Oboe rate mismatch: requested {rate}, got {}", stream.get_sample_rate()));
    }
    let requested_buffer = if native { stream.get_frames_per_burst().max(1) * 2 } else { target };
    let accepted = stream.set_buffer_size_in_frames(requested_buffer);
    eprintln!("BanaForge audio opened: mode={mode:?} backend=Oboe/{:?} performance={:?} rate={}Hz callback={} frames requested_buffer={} actual_buffer={} frames capacity={} frames buffer_request={accepted:?}; default Android route (including compatible USB/headsets)",
        stream.get_audio_api(), stream.get_performance_mode(), stream.get_sample_rate(),
        stream.get_frames_per_callback(), requested_buffer, stream.get_buffer_size_in_frames(),
        stream.get_buffer_capacity_in_frames());
    stream.start().map_err(|e| format!("Oboe start {mode:?}: {e:?}"))?;
    Ok(stream)
}

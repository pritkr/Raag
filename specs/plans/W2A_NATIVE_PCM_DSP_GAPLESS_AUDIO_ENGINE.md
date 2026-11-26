# W2A — Native Audiophile PCM Audio Engine: True Gapless, Equal-Power Crossfade, Real-Time 10-Band EQ, ReplayGain, Silence Skip & Pitch-Preserving Speed

> **Wave:** Wave 2 (Parallel Agent 2A — Runs concurrently with W2B, W2C, W2D after Wave 1.5 Stitch)
> **Exclusive Write Scope:**
> - `crates/core/src/audio_dsp/{mod.rs,eq.rs,loudness.rs,ring_buffer.rs,silence.rs,splice.rs,time_stretch.rs}`
> - `crates/core/src/pcm_engine/{mod.rs,decoder.rs,source.rs,mixer.rs,resampler.rs,engine.rs}` (new)
> - `platform/pcm_backend.rs` (new Desktop `cpal` `AudioBackend`)
> - `platform/android/src/main/kotlin/dev/resonance/ytmusic/ResonanceDspAudioProcessor.kt` (new Android ExoPlayer custom `AudioProcessor` calling Rust `DspPipeline` via zero-copy direct `ByteBuffer` JNI)
> - `tests/w2a_native_pcm_dsp_engine_test.rs` (new)
> **Read-Only Dependencies:** Existing `crates/core/src/audio_dsp/*`, `LoopbackProxy`, `symphonia`, `rubato`, `cpal`, `jni` (all frozen in Wave 0 `Cargo.toml`).
> **Stitch Hooks (For Wave 2.5 Integration Agent):**
> - Select `PcmAudioBackend` (Desktop) and register `ResonanceDspAudioProcessor` into `PlaybackForegroundService.kt` (Android)
> - Flip `AudioDspCapability::NativePcmPipeline` in `src/store/settings_slice.rs` so Settings DSP sliders/toggles become live!
> - Subscribe `PcmAudioBackend` to `DspConfig` & `PlaybackSpeedConfig` updates and wire `DspPipeline::set_track_loudness_db(stream.loudness_db)` on track load
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/**`, `crates/core/src/innertube/**`, `crates/core/src/providers/**`.

---

## 1. Why This Plan Matters (Turning Resonance Into a True Audiophile Client)

Resonance already has well-written DSP primitives in `crates/core/src/audio_dsp/` (`SpscRingBuffer`, `LoudnessNormalizer`, `TenBandEq`, `SilenceSkipper`, `GaplessSplicer`) — **but they are dead code during playback** because `nsh/audio` streams URLs directly to opaque OS decoders.

Wave 2A brings those primitives to life on **both Desktop and Android** with studio-grade real-time safety:
- **Desktop (macOS CoreAudio / Windows WASAPI / Linux PipeWire-ALSA):** Pure-Rust decode (`symphonia`) → dual-slot gapless/crossfade mixer → WSOLA pitch-preserving time-stretcher → `DspPipeline` (ReplayGain + click-free 10-band RBJ biquad EQ + RMS silence skipper + >100% soft-limited boost) → `rubato` RT-safe hardware sample-rate conversion → lock-free `SpscRingBuffer` → `cpal` hardware callback.
- **Android (Media3 ExoPlayer):** Zero-copy `ResonanceDspAudioProcessor` plugged into ExoPlayer's `DefaultAudioSink`, passing direct JVM `ByteBuffer` memory addresses across JNI (`get_direct_buffer_address`) straight into the **exact same Rust `DspPipeline`**—preserving hardware AAC/Opus offload efficiency, battery life, and Media3 gapless playback while delivering identical EQ/ReplayGain/Silence Skip!

---

## 2. Target Real-Time Threading & Dataflow Architecture

```text
================================================================================
DESKTOP NATIVE PCM PIPELINE (3 Strictly Isolated Thread Domains)
================================================================================
[Thread 1: Tokio Async I/O]
  `LoopbackProxy` (256 KB ChunkCache / OfflineVault / Local File)
        │  Range-backed `SeekableHttpOrFileSource` (blocking read-ahead ring)
        ▼
[Thread 2: Dedicated Decode & DSP Thread (`resonance-pcm-decoder`, normal prio)]
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ Dual-Slot Symphonia Decoder                                             │
  │  • Active Slot A (Opus/WebM, AAC/M4A, FLAC, MP3, Vorbis, WAV)           │
  │  • Pre-opened Slot B (warmed at 80% progress for 0-sample gapless!)     │
  │  • Strips codec encoder delay / trailing padding (iTunSMPB / Opus pre)  │
  └───────────────────────────────┬─────────────────────────────────────────┘
                                  ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ Real-Time Modular DSP Chain (Interleaved f32 Stereo @ Engine Rate)      │
  │  1. GaplessSplicer / Equal-Power Crossfade (cos/sin -3 dB center curve) │
  │  2. WsolaTimeStretcher (0.5x–2.0x speed, preserve_pitch on/off)         │
  │  3. SilenceSkipper (RMS dBFS < -45 dB for > 350 ms, 10 ms fade edges)   │
  │  4. LoudnessNormalizer (YouTube loudnessDb / ReplayGain → -14 LUFS)     │
  │  5. TenBandEq (10× RBJ Peaking/Shelf Biquads, 31 Hz–16 kHz, smoothed)   │
  │  6. Volume + >100% Soft-Limiter (`tanh`/cubic knee preventing clipping) │
  │  7. Rubato Resampler (`FftFixedIn` / `SincFixedIn` → OS Device Rate,    │
  │     also accepts ±1.5% micro-slew ratio from W2D Listen Together!)      │
  └───────────────────────────────┬─────────────────────────────────────────┘
                                  │  Lock-Free `SpscProducer::push_slice`
                                  ▼
              ╔═══════════════════════════════════════════╗
              ║   Lock-Free SPSC Ring Buffer (~200 ms)    ║  ◄── REAL-TIME BARRIER
              ╚═══════════════════╦═══════════════════════╝
                                  │  Lock-Free `SpscConsumer::pop_slice`
                                  ▼
[Thread 3: OS Real-Time Audio Callback (`cpal` CoreAudio / WASAPI / PipeWire)]
  • ZERO heap allocations, ZERO mutexes, ZERO syscalls, ZERO logging!
  • Reads atomic `paused` / `target_gain` with 5 ms exponential de-click ramp
  • On underrun: smooth 64-sample fade-to-zero (never raw square-wave click!)
  • Updates `AtomicU64` played_frames counter for exact sub-ms position telemetry

================================================================================
ANDROID ZERO-COPY JNI DSP PIPELINE
================================================================================
  Media3 ExoPlayer (Hardware/FFmpeg Decode + PlaybackParameters speed/pitch)
        │  PCM Float/16-bit Direct `ByteBuffer`
        ▼
  `ResonanceDspAudioProcessor.queueInput(inputBuffer)`
        │  JNI `get_direct_buffer_address` (ZERO byte array copy!)
        ▼
  Rust `DspPipeline` (Shared `TenBandEq` + `LoudnessNormalizer` + `SilenceSkipper`)
        │  In-place / output direct `ByteBuffer`
        ▼
  Android `AudioTrack` / AAudio Output
```

---

## 3. Deep Technical Design Details

### 3.1 Seekable Read-Ahead Source (`crates/core/src/pcm_engine/source.rs`)
- Implements `symphonia::core::io::MediaSource` (`Read + Seek + Send + Sync`) for both:
  1. **Direct Local / Offline Disk Files** (`std::fs::File` wrapped in `BufReader` — zero HTTP overhead for offline downloads and W2C local FLAC/MP3!).
  2. **Loopback HTTP Streams (`http://127.0.0.1:<port>/stream/...`)** using a small synchronous `HTTP Range` read-ahead buffer (256 KB aligned matching `ChunkCache::CHUNK_SIZE`) with atomic cancellation flag (`Arc<AtomicBool>`) so rapid track skips abort blocked HTTP reads within `< 5 ms`.
- Supports instant seeking (`SeekFrom::Start/Current/End`) via Symphonia's `FormatReader::seek(SeekMode::Accurate, SeekTo::Time { .. })`.

### 3.2 Dual-Slot True Gapless & Equal-Power Crossfade (`crates/core/src/pcm_engine/decoder.rs` & `mixer.rs`)
- **Gapless Sample Accuracy:**
  - Respects container/codec `delay` and `padding` fields from `symphonia::core::codecs::CodecParameters` (MP4 `edts`/`elst`, Opus `pre_skip`, MP3 LAME gapless tags) so live albums and DJ mixes transition with **zero audible seam**.
  - When active Track A crosses 80% duration (or receives `PrepareNext(url, loudness_db)` from `PlaybackController`), the decoder thread opens Slot B in advance and decodes its first 100 ms into a warm staging buffer.
  - At Track A EOF:
    - If `crossfade_ms == 0`: immediately splice sample 0 of Slot B onto sample $N$ of Slot A, emit `AudioTelemetryEvent::TrackTransitioned { new_generation }`, and promote Slot B → Slot A.
    - If `crossfade_ms > 0`: begin overlapping `crossfade_ms` before Track A's EOF (clamped to `min(crossfade_ms, track_a_remaining / 2, track_b_duration / 2)`), mixing with **equal-power constant-loudness curves** ($w_A = \cos(\frac{\pi}{2} t)$, $w_B = \sin(\frac{\pi}{2} t)$) so perceived volume never dips in the middle!

### 3.3 Click-Free Real-Time DSP Hardening (`crates/core/src/audio_dsp/*`)
Upgrade the existing DSP modules for real-world audiophile listening:
1. **Parameter Smoothing on `TenBandEq` (`eq.rs`):**
   - Naive immediate biquad coefficient swaps when dragging an EQ slider cause audible zipper noise/clicks.
   - Smooth each band's target gain in dB over a 15 ms per-block ramp (`alpha = 0.08` per 128-frame sub-block) and recalculate RBJ Audio EQ Cookbook coefficients smoothly; enforce **Transposed Direct Form II** state stability (flush denormals `< 1e-18` to `0.0` to prevent x86/ARM CPU spikes on silent passages!).
   - Add **Pre-Amp Headroom Auto-Compensation**: compute `max_positive_gain_db` across the 10 bands and optionally apply `-max_positive_gain_db` pre-attenuation (followed by `soft_limit`) so `+9 dB Bass Boost` never harshly clips!
2. **Smooth Click-Free `SilenceSkipper` (`silence.rs`):**
   - When transitioning into or out of a skipped silent region (`RMS < -45 dBFS` for `>= 350 ms`), apply a 10 ms Hann fade-out / fade-in ramp so skipped intros/outros never pop.
3. **WSOLA Pitch-Preserving Time-Stretcher (`time_stretch.rs`):**
   - Pure-Rust **Waveform Similarity Overlap-Add (WSOLA)** operating on stereo `f32` frames (window size 40 ms @ 48 kHz = 1,920 frames, Hann window, cross-correlation search window ±8 ms):
     - When `speed == 1.0`: **0-copy bypass** (zero CPU overhead, bit-transparent!).
     - When `speed != 1.0 && preserve_pitch == true`: WSOLA stretches/compresses time from `0.5×` to `2.0×` cleanly without chipmunk/Darth-Vader pitch distortion (ideal for podcasts & study).
     - When `speed != 1.0 && preserve_pitch == false`: adjusts `rubato` resampling ratio directly for crisp tape-speed playback.
4. **Live Micro-Slew Hook for W2D Listen Together (`resampler.rs`):**
   - `rubato::SincFixedIn` (when slewing or non-integer ratio) / `FftFixedIn` (when converting e.g. 44.1 kHz → 48 kHz) exposes `set_slew_ratio(ratio: f64)` clamped to `0.985..=1.015` (`±1.5%`), enabling W2D multi-user Listen Together to eliminate clock drift invisibly!

### 3.4 `cpal` Output Stream & Bluetooth Hot-Swap Recovery (`platform/pcm_backend.rs`)
- Queries default OS output device (`cpal::default_host().default_output_device()`) and negotiates preferred `f32` stereo stream (with clean conversion fallback for `i16`/`u16` Windows WASAPI devices).
- **Real-Time Callback Contract:**
  - Reads lock-free `SpscConsumer`, applies atomic smoothed play/pause gain ramp (5 ms fade on pause/resume/seek so pausing NEVER clicks!).
  - Increments `AtomicU64` `frames_played` (adjusted by `speed` and `seek_base_ms`) for accurate sub-millisecond telemetry.
- **Hot-Swap / Device Disconnect Recovery:**
  - When `cpal`'s `err_fn` reports `StreamError::DeviceNotAvailable` (e.g., user unplugs USB DAC or AirPods disconnect), signal a dedicated supervisor task that automatically re-queries `default_output_device()`, rebuilds the output stream at the new device's sample rate, reconfigures `rubato`, and resumes seamlessly from the ring buffer (or pauses cleanly if OS policy requests).

### 3.5 Android Zero-Copy JNI `ResonanceDspAudioProcessor.kt`
- Extends `androidx.media3.common.audio.BaseAudioProcessor`:
  - `onConfigure(inputAudioFormat)`: accepts `ENCODING_PCM_FLOAT` or `ENCODING_PCM_16BIT`, allocates a persistent Rust `DspPipeline` handle via `nativeCreateDspHandle(sampleRate, channelCount): Long` (freed in `onReset()`).
  - `queueInput(inputBuffer: ByteBuffer)`: acquires direct output buffer via `replaceOutputBuffer(inputBuffer.remaining())` and calls `nativeProcessPcm(dspHandle, inputBuffer, inputBuffer.position(), inputBuffer.remaining(), outputBuffer, isFloat)`.
  - Rust side (`platform/android.rs` / `crates/core/src/audio_dsp/`): maps `env.get_direct_buffer_address(...)` directly to `&[f32]` / `&mut [f32]` (converting `i16` in-place via stack scratch if needed), runs `DspPipeline::process_interleaved`, zero heap allocations!
  - Live UI EQ / ReplayGain / Silence Skip updates call `nativeUpdateDspConfig(dspHandle, ...)` via lock-free atomic swap / `parking_lot`/try-lock snapshot.

---

## 4. Deterministic Offline Verification (`tests/w2a_native_pcm_dsp_engine_test.rs`)

1. `symphonia_decodes_generated_wav_and_flac_with_exact_duration_and_gapless_splice` — generates two phase-continuous 440 Hz sine WAV files in `TempDir`, runs them through the dual-slot gapless decoder & mixer, asserts zero sample discontinuity at the track boundary!
2. `equal_power_crossfade_preserves_constant_rms_loudness` — mixes two constant-amplitude signals across a 500 ms crossfade and verifies midpoint RMS stays within ±0.5 dB of steady-state (`cos² + sin² = 1`).
3. `ten_band_eq_boosts_target_band_without_clipping_or_denormals` — feeds 62 Hz and 8 kHz sine sweeps through BassBoost & TrebleBoost presets, asserts FFT/energy delta matches target dB, output peak `< 1.0` (soft-limited), and zero NaN/Inf/denormals on silence.
4. `wsola_time_stretcher_scales_duration_at_0_5x_and_2_0x_preserving_fundamental_pitch` — feeds a 440 Hz sine wave at `1.5×` speed with `preserve_pitch = true`, verifies output length is `~0.667×` input and zero-crossing fundamental frequency remains `440 Hz ± 10 Hz`!
5. `silence_skipper_removes_long_silence_with_click_free_ramps` — feeds tone → 1s silence → tone; verifies silence compressed and max sample-to-sample delta at splice edge stays smooth (no click).
6. `spsc_ring_buffer_underrun_fades_cleanly_to_zero_without_allocations` — exercises RT consumer pull across full, partial, and empty states.

```bash
cargo test --test w2a_native_pcm_dsp_engine_test -- --nocapture
cargo test -p resonance-core audio_dsp
```

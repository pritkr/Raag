# P1: Native PCM Audio Engine (Symphonia + CPAL/AAudio) & Live DSP Pipeline

> **Wave:** A (Parallel)
> **Status of Existing Code:** Scaffolded in `crates/core/src/pcm_engine/*` (`decoder.rs`, `engine.rs`, `mixer.rs`, `resampler.rs`, `source.rs`), `crates/core/src/audio_dsp/*` (`eq.rs`, `loudness.rs`, `ring_buffer.rs`, `silence.rs`, `splice.rs`, `time_stretch.rs`), and `platform/pcm_backend.rs` — **NOT wired into `PlaybackController`**, which currently uses `NshAudioBackend` (`ExternalUrlStreamOnly` via `nsh/audio` AVPlayer/MediaPlayer) causing every DSP knob in `settings_dsp_card.rs` to show a warning or do nothing!
> **Exclusive Owned Paths:** `crates/core/src/pcm_engine/*`, `crates/core/src/audio_dsp/*`, `platform/pcm_backend.rs`, `platform/nsh_audio.rs`, `src/screens/settings_dsp_card.rs`, `src/live_engine/handlers/audio_dsp.rs`, `tests/p1_pcm_dsp_test.rs`.

---

## 1. Deep Technical Research & Architecture

### 1.1 Why Replacing `ExternalUrlStreamOnly` Matters
YouTube Music serves primarily:
- **Opus in WebM/Matroska (`itag 251` ~160kbps, `itag 250` ~70kbps, `itag 249` ~50kbps)** — sample rate 48 kHz, preferred for audio quality per bit.
- **AAC-LC in fragmented MP4/M4A (`itag 140` ~128kbps, `itag 141` ~256kbps Premium, `itag 774` Opus 256kbps Premium)** — sample rate 44.1 kHz or 48 kHz.
- **Local files (FLAC, MP3, Ogg Vorbis, WAV, ALAC, M4A, Opus)** for P4 local playback.

Platform URL players (`nsh/audio`) cannot:
1. Run true zero-gap sample-accurate crossfade/gapless splicing between two streams.
2. Apply real-time 10-band parametric/peaking EQ, EBU R128 loudness normalization, silence skipping, or pitch-preserving speed (`0.5x–2.0x`) on desktop without OS-specific audio graph hacks.
3. Seamlessly splice an already-prefetched next track in RAM before the current track finishes.

### 1.2 Target Threading & Real-Time Lock-Free Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Tokio Async Runtime (Network / Cache / Proxy)                           │
│  ChunkedHttpMediaSource (implements symphonia::core::io::MediaSource)   │
│   └─ reads from 3-tier Cache (RAM 256KB -> SQLite -> LoopbackProxy)     │
└───────────────────────────────┬─────────────────────────────────────────┘
                                │ (blocking read inside dedicated Decode Thread)
┌───────────────────────────────▼─────────────────────────────────────────┐
│ Decode & DSP Worker Thread (std::thread::Builder, high priority)        │
│  1. Symphonia Probe -> FormatReader (mkv/webm, isomp4, flac, mp3, ogg)  │
│  2. Decoder (symphonia-codec-aac/flac/mp3/vorbis + opus via audiopus    │
│     or symphonia-adapter-libopus / pure-Rust libopus bindings)          │
│  3. Channel normalization -> stereo f32 interleaved                     │
│  4. Rubato SincFixedIn resampler -> device output sample rate (48kHz)   │
│  5. Dual-deck Gapless / Crossfade Mixer (Deck A active, Deck B prepped) │
│  6. Live DSP Chain (lock-free atomic params):                           │
│     ├─ WSOLA Pitch-Preserving Time-Stretch (0.5x..=2.0x, bypass at 1.0) │
│     ├─ RMS Silence Skipper (threshold -48 dBFS, hold > 180ms, soft ramp)│
│     ├─ 10-Band Peaking/Shelf Biquad EQ (32..16kHz, Direct Form II T)    │
│     ├─ EBU R128 Loudness Gain + Soft-Knee Peak Limiter (-1.0 dBFS)      │
│     └─ Master Volume + Smooth 10ms Mute/Pause De-Click Ramp             │
└───────────────────────────────┬─────────────────────────────────────────┘
                                │ SPSC Lock-Free RingBuffer (ringbuf / rtrb, 200ms)
┌───────────────────────────────▼─────────────────────────────────────────┐
│ Real-Time Audio Output Callback (ZERO locks, ZERO allocs, ZERO I/O!)    │
│  - Desktop (macOS CoreAudio, Linux PipeWire/ALSA, Windows WASAPI): cpal │
│  - Android API 26+: ExoPlayer/Media3 + ResonanceDspAudioProcessor.kt    │
│    (JNI DSP chain sharing identical biquad/limiter math, or Oboe/AAudio)│
└─────────────────────────────────────────────────────────────────────────┘
```

### 1.3 Critical Research Details & Pitfalls
1. **Opus Decoding in Symphonia:** `symphonia` 0.5 demuxes WebM/MKV (`mkv` feature) and Ogg cleanly, **but does not ship a pure-Rust Opus decoder out of the box** (only AAC, MP3, FLAC, Vorbis, PCM, ALAC).
   - **Solution:** Use `symphonia` (`mkv`, `isomp4`, `ogg`, `flac`, `mp3`, `aac`, `vorbis`, `pcm`) + **`opus-decoder` / `audiopus` (BSD-3-Clause, GPLv3-compatible)** registered as a custom `symphonia::core::codecs::Decoder` adapter for `CODEC_TYPE_OPUS`, OR decode Opus packets directly in `crates/core/src/pcm_engine/decoder.rs` when `codec_params.codec == CODEC_TYPE_OPUS`!
   - Always honor `codec_params.delay` (pre-skip, typically 312 samples at 48 kHz for Opus) and `codec_params.padding` (trailing padding) for **sample-accurate gapless playback**!
2. **Seekable Streaming `MediaSource` (`source.rs`):**
   - YouTube fragmented MP4 (`itag 140`) puts `moov` + `sidx` at the front; WebM (`itag 251`) puts `Cues` at the end or front.
   - `ChunkedMediaSource` MUST implement `Seek` using the 256 KB `ChunkCache` + `LoopbackProxy` HTTP `Range: bytes=start-end` fetches, reporting `is_seekable() == true` and `byte_len() == Some(content_length)` so Symphonia's binary-search seek (`FormatReader::seek(SeekMode::Accurate, ...)`) works instantaneously without downloading the entire file.
3. **Real-Time Safety in `cpal` Output Callback:**
   - **NEVER** acquire a `Mutex`, allocate a `Vec`, log with `tracing`, or perform a syscall inside the `cpal::Stream` callback.
   - Use a single-producer single-consumer lock-free ring buffer (`ringbuf::HeapRb<f32>` or existing `audio_dsp::ring_buffer.rs` hardened with `Acquire`/`Release` atomics).
   - On buffer underrun, fade the last 64 samples to `0.0` (eliminating harsh clicks/pops) and increment an `AtomicU64` underrun counter polled by diagnostics.
4. **Device Hot-Swap (Bluetooth Headphones / USB DAC Disconnect):**
   - `cpal` error callback fires on device disconnect. Signal an `AtomicBool` (`needs_device_rebind`), tear down the old `cpal::Stream`, re-query `host.default_output_device()`, re-initialize the `rubato` resampler if the new device sample rate changed (e.g. 48 kHz → 44.1 kHz), and resume cleanly within `< 150ms` at the exact sample position!
5. **Android Strategy (Battery + Media3 Lockscreen Integration):**
   - On Android, Media3 (`ExoPlayer`) owns hardware audio focus, AudioOffload, Bluetooth AVRCP, and Android Auto.
   - Rather than fighting ExoPlayer with a second AAudio stream, wire the exact same Rust DSP C-ABI (`resonance_dsp_process_f32`) into the existing `platform/android/.../ResonanceDspAudioProcessor.kt` (`BaseAudioProcessor`), so Android gets native Media3 reliability **AND** identical 10-band EQ, loudness normalization, silence skip, and crossfade!

---

## 2. Concrete Implementation Plan (Step-by-Step)

### Step 1: Dependencies & Feature Flags (`crates/core/Cargo.toml`)
- Add/verify `symphonia = { version = "0.5", features = ["aac", "flac", "isomp4", "mkv", "mp3", "ogg", "pcm", "vorbis"] }`.
- Add `cpal = "0.15"` (desktop targets only: `[target.'cfg(not(target_os = "android"))'.dependencies]`).
- Add `rubato = "0.16"` for high-quality sinc resampling.
- Add BSD-licensed Opus decoder (`audiopus` or pure-Rust `opusic-c` / `opus` wrapper; fallback to AAC `itag 140` automatically if `opus` feature is disabled on a build target).

### Step 2: Harden `crates/core/src/pcm_engine/`
1. **`source.rs` (`SeekableStreamSource`):**
   - Bridge async `ChunkCache` + HTTP Range fetcher into synchronous `Read + Seek + symphonia::core::io::MediaSource` using a bounded read-ahead window (2 × 256 KB chunks prefetched asynchronously ahead of cursor).
   - Support `file://` / local filesystem paths directly via `std::fs::File` (zero HTTP overhead for P3 offline vault & P4 local files).
   - Support cancellation token (`Arc<AtomicBool>`) so seeking or skipping aborts stalled reads in `< 10ms`.
2. **`decoder.rs` (`AudioDecoder`):**
   - Support `symphonia` tracks + Opus codec packet decode + pre-skip (`delay`) and end-trim (`padding`).
   - Expose `decode_next_frame(&mut self, out_interleaved: &mut Vec<f32>) -> Result<DecodeResult>` returning `Samples(usize)`, `Eof`, or `RecoverablePacketError`.
   - Expose `seek_ms(&mut self, target_ms: u64) -> Result<u64>` with exact sample timestamp reporting.
3. **`resampler.rs` (`DynamicResampler`):**
   - Wrap `rubato::SincFixedIn<f32>` (bypass zero-copy when `source_rate == output_rate`).
   - Recreate transparently on track change or output device hot-swap.
4. **`mixer.rs` (`DualDeckMixer`):**
   - Maintain `active_deck: Option<Deck>` and `next_deck: Option<Deck>` (pre-opened via `prepare_next(url, metadata)` 12 seconds before EOF).
   - When `crossfade_ms == 0`: sample-accurate gapless splice on exact EOF sample (using trimmed `delay`/`padding`).
   - When `crossfade_ms > 0`: equal-power cosine/sine curve (`cos(t * π/2)` / `sin(t * π/2)`) overlapping the final `crossfade_ms` of Deck A with the start of Deck B.
   - Emit `TrackTransitioned { new_track_id }` event to `PlaybackController` so UI, lyrics, and MediaSession update on the exact sample boundary!
5. **`engine.rs` (`PcmAudioEngine`):**
   - Own the `cpal::Stream`, the decode+DSP worker thread, command channel (`crossbeam_channel::Sender<PcmCommand>`), and atomic telemetry (`position_ms: AtomicU64`, `duration_ms: AtomicU64`, `state: AtomicU8`, `underruns: AtomicU64`, `output_sample_rate: AtomicU32`, `active_codec: RwLock<String>`).
   - Apply `DspConfig` updates (`eq_gains_db`, `eq_enabled`, `loudness_enabled`, `loudness_target_lufs`, `silence_skip_enabled`, `crossfade_ms`, `playback_speed`, `volume`, `muted`) atomically with smooth 15ms parameter ramping (zero zipper noise when dragging EQ sliders!).

### Step 3: Wire `platform/pcm_backend.rs` into `PlaybackController`
- Implement `resonance_contract::AudioBackend` on `PcmAudioBackend` (already partially defined in `platform/pcm_backend.rs`).
- Return `DspCapability::FullNativePcm` (removing the `ExternalUrlStreamOnly` limitation on desktop!).
- Select `PcmAudioBackend` by default on desktop in `src/live_engine/boot.rs`, keeping `NshAudioBackend` only as an explicit fallback if `cpal` finds zero audio output devices (e.g., headless CI).
- Expose `src/live_engine/handlers/audio_dsp.rs` so `settings_dsp_card.rs` controls update `PcmAudioEngine` live AND persist `DspSettings` to SQLite `kv_state`.

### Step 4: Update `src/screens/settings_dsp_card.rs`
- Remove any "DSP unavailable on URL backend" warning when `DspCapability::FullNativePcm` is active.
- Show real active audio telemetry chip (e.g., `"Native PCM • 48 kHz • Opus • Gapless"`), live 10-band EQ sliders (`-12 dB` to `+12 dB`) with presets (`Flat`, `Bass Boost`, `Vocal`, `Acoustic`, `Late Night`), Loudness Normalization (`-14 LUFS`), Silence Skip toggle, Crossfade (`0s–12s`), and Reset button.

---

## 3. Acceptance Criteria & Verification Tests (`tests/p1_pcm_dsp_test.rs`)

1. **Synthetic PCM Sine & Impulse Tests:**
   - Generate 440 Hz / 1 kHz stereo WAV/FLAC buffers in memory; verify `BiquadEq` boosts 1 kHz by `+6 dB ± 0.3 dB` and leaves 60 Hz within `±0.5 dB`.
   - Verify `LoudnessNormalizer` + soft limiter clamps peaks `<= -0.95 dBFS` (zero hard clipping / NaN / Inf).
   - Verify `SilenceSkipper` skips a 500ms `-60 dBFS` gap with smooth crossfade ramp and zero clicks (`max(|s[n] - s[n-1]|) < 0.05`).
   - Verify `TimeStretch` at `1.5x` shortens buffer duration by `33% ± 3%` while preserving fundamental pitch within `±2%`.
2. **Gapless & Crossfade Splice Test:**
   - Feed Deck A (1.0s continuous 440 Hz phase-aligned sine) + Deck B (1.0s continuation) with `crossfade_ms = 0`; assert max sample discontinuity at boundary `< 0.01`.
   - Feed with `crossfade_ms = 200`; assert output length equals `1.8s ± 5ms` and equal-power sum RMS stays within `±1 dB`.
3. **Seek & Cancellation Latency Test:**
   - Assert seeking cancels stalled `MediaSource` reads within `< 50ms`.
4. **Zero-Allocation Ring Buffer Stress Test:**
   - Run concurrent producer/consumer threads pushing 10M samples; assert zero lost samples and exact FIFO order.

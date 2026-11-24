# Sonora 🎵

**Sonora** is a lightweight audio player for Windows, developed entirely in Rust. It uses native Windows APIs for its graphical interface and a dedicated audio engine for decoding and playback.

The main objective is to provide a standalone, lightweight audio player without relying on large multimedia frameworks such as VLC or FFmpeg.

## ✨ Features

- **Audio playback:** Supports MP3, WAV, AAC, MP4, MKV, WebM, OGG, Vorbis and Opus.
- **Native Windows interface:** Built directly with Win32 and GDI, without external GUI frameworks.
- **Integrated audio engine:** Custom playback engine with dedicated decoding and audio output threads.
- **Playlist navigation:** Previous track, next track and automatic playback advancement.
- **Seeking controls:** Navigate through audio tracks with playback position controls.
- **Volume control:** Adjustable volume, mute and smooth volume transitions.
- **Global media keys:** Supports multimedia keyboard shortcuts, even when Sonora is not focused.
- **Folder browser:** Built-in file explorer with multi-folder selection.
- **Automatic resource management:** Releases unused audio streams and reduces memory usage when the application is idle.
- **Responsive interface:** Window resizing with DPI-aware layout adjustments.

## 🛠️ Technologies and Libraries

| Technology | Purpose |
|---|---|
| Rust | Main programming language |
| Win32 API / GDI | Native Windows graphical interface |
| Symphonia 0.5.5 | Audio decoding and media demuxing |
| libopus | Integrated Opus audio decoder |
| CPAL | Audio output through Windows WASAPI |
| ringbuf | Lock-free circular audio buffer |
| windows-sys | Access to native Windows APIs |
| cc | Compiling the integrated C decoder |
| embed-resource | Embedding Windows resources and manifest |
| miniz_oxide | PNG compression during icon generation |

## 🔊 Audio Engine

Sonora uses a custom audio engine instead of an external multimedia player.

Its playback pipeline works as follows:

1. Symphonia reads and decodes the media file.
2. The decoded audio is converted into PCM samples.
3. Channels are mixed and audio is resampled when necessary.
4. Samples are transferred through a circular buffer.
5. CPAL sends the audio to the output device through WASAPI.

The application also includes a custom sinc resampler for audio sample-rate conversion.

### Integrated Opus Decoder

The project integrates libopus directly into the executable.

Only the Opus decoder is compiled. The encoder and its associated components are excluded.

This avoids requiring an additional libopus DLL at runtime.

### Lightweight Symphonia

The project includes optimized local copies of selected Symphonia components.

Unused FFT tables and the `encoding_rs` dependency have been removed to reduce the final executable size without changing the supported playback functionality.

## 📦 Build

Requirements:
- Windows
- Rust (stable)
- Visual Studio C++ Build Tools

Compile the application in release mode:

```bash
cargo build --release
```

The executable will be generated at:

```text
target/release/sonora.exe
```

## 📏 Performance and Optimization

Sonora is designed to minimize resource consumption.

- Approximately 1.5 MB executable size.
- No external VLC or FFmpeg dependency.
- Audio streams are opened only when needed.
- Audio streams are closed after 10 seconds of inactivity.
- Audio threads remain idle when playback is not required.
- Memory is released back to Windows when appropriate.
- Release compilation uses Link-Time Optimization (LTO), size optimization and symbol stripping.

## 📁 Project Structure

| File / Directory | Description |
|---|---|
| `src/audio_engine.rs` | Audio playback engine |
| `src/media_decoder.rs` | Media decoding |
| `src/format_conversion.rs` | Audio format conversion |
| `src/sinc_resampler.rs` | Custom sinc resampler |
| `src/opus_decoder.rs` | Opus decoder integration |
| `src/libopus_bindings.rs` | Native libopus bindings |
| `src/codec_registry.rs` | Audio codec registration |
| `src/main_window.rs` | Main Windows interface |
| `src/browser_window.rs` | Built-in folder browser interface |
| `src/folder_browser.rs` | Folder navigation and selection |
| `src/media_keys.rs` | Global multimedia shortcuts |
| `src/playlist_navigation.rs` | Playlist navigation |
| `src/auto_advance.rs` | Automatic next-track playback |
| `src/volume_control.rs` | Volume management |
| `src/memory_trimming.rs` | Memory optimization |
| `vendor/libopus/` | Integrated Opus decoder source |
| `vendor/symphonia/` | Optimized Symphonia components |

## 📜 License

See the licenses of the integrated third-party components, including libopus and Symphonia.

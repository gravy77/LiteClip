<div align="center">

<img src="assets/banner.svg" alt="LiteClip" width="100%">

<br>

**Press a key and the last 30 seconds of your game becomes a clip.**<br>
LiteClip is a replay-buffer clipper and mini editor for Windows. It uses your GPU's video encoder so recording barely touches the CPU, and it never connects to the internet.

<br>

[![Build](https://github.com/fenziiarg/LiteClip/actions/workflows/ci.yml/badge.svg)](https://github.com/fenziiarg/LiteClip/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/fenziiarg/LiteClip?color=4aa3ff&label=download)](https://github.com/fenziiarg/LiteClip/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-7c5cff)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0f1117)
![Rust](https://img.shields.io/badge/made%20with-Rust-dea584?logo=rust&logoColor=white)

[**Download**](https://github.com/fenziiarg/LiteClip/releases/latest) · [Features](#-features) · [Getting started](#-getting-started) · [Build from source](#-build-from-source) · [FAQ](#-faq)

</div>

---

## ✨ Why LiteClip?

Most clipping apps come with an account, a cloud upload, a social feed, and a web-based UI. On a weak laptop that adds up to hundreds of MB of RAM and a few percent of CPU spent on things that aren't recording.

LiteClip does one job:

- **It runs on what you already have.** Recording happens on your GPU (NVIDIA NVENC, Intel Quick Sync or AMD AMF). On an Intel Iris Plus laptop, the capture process used under 20% of one CPU core and the UI used about 2%.
- **Saving a clip is instant.** The replay buffer is already encoded, so saving joins the last few chunks without re-encoding. Most clips are written in under a second.
- **It never goes online.** There are no accounts, uploads, telemetry or update checks. Clips stay on your disk.
- **It's a native app.** One ~4 MB `.exe` written in Rust with an `egui` UI. No Electron and no browser engine.

## 🚀 Features

| | |
|---|---|
| 🎮 **Replay buffer** | Keeps the last 10 to 300 seconds of your screen. Press `F8` to save them. |
| ⚡ **GPU encoding** | Picks the best encoder on your machine automatically (NVENC, Quick Sync or AMF), with CPU x264 as the fallback. |
| 🔊 **Audio mixing** | Records game/system sound and your microphone, each with its own device and volume setting. |
| ✂️ **Mini editor** | Scrub the timeline, split, cut parts out, and trim the start or end. Change speed (0.25× to 4×), volume, resolution and frame rate, or mute the audio. |
| 🚄 **Instant export** | Plain trims are exported without re-encoding, so they finish in a moment. |
| 🗂️ **Clip library** | Thumbnails, search, play, rename, show in folder, and delete to the Recycle Bin. |
| ⌨️ **Global hotkeys** | Work inside fullscreen games and while the window is minimized. You can change them. |
| 🔔 **Save sound** | A short chime when a clip is saved. You can also use your own `.wav` or turn it off. |
| 🛟 **Self-healing capture** | If capture stops (a UAC prompt, a display change), LiteClip restarts it. If Desktop Duplication isn't available, it switches to GDI capture. |

## 📦 Getting started

1. **Install FFmpeg.** LiteClip uses it for capture and encoding:
   ```powershell
   winget install Gyan.FFmpeg.Essentials
   ```
2. **Download `LiteClip.exe`** from the [latest release](https://github.com/fenziiarg/LiteClip/releases/latest) and run it. It doesn't need an installer.
3. Play something and press **`F8`**. Your clip is saved to `Videos\LiteClip`.

> [!TIP]
> If your laptop struggles, set **Resolution → 720p** and **Frame rate → 30 fps** in Settings. Clips get smaller and the encoder has less work to do.

### Hotkeys

| Key | Action |
|---|---|
| `F8` | Save clip |
| `Alt+F8` | Start or stop the replay buffer |
| `Space` | Editor: play or pause |
| `S` | Editor: split at the playhead |
| `D` | Editor: cut or keep the current part |
| `←` `→` | Editor: seek 1 second |

You can change the global hotkeys in **Settings → Hotkeys**.

## ⚙️ Settings

You can change all of these from the Settings tab:

- **Replay buffer:** clip length, monitor, capture method (Desktop Duplication or GDI), resolution, fps, bitrate, encoder, and whether the cursor is shown
- **Audio:** output device for game sound, input device for the mic, a volume for each, audio bitrate, and live level meters
- **General:** clips folder, start the buffer on launch, start minimized, and a custom FFmpeg path
- **Feedback:** the clip-saved sound (chime, custom `.wav` or off)

Settings are stored in `%APPDATA%\LiteClip\settings.json`.

## 🧠 How it works

```
 ┌────────────┐   frames    ┌──────────────────────┐   2 s .ts chunks   ┌──────────────┐
 │  Screen    │ ──────────▶ │  ffmpeg              │ ─────────────────▶ │ rolling disk │
 │ (DDA/GDI)  │             │  GPU H.264 + AAC     │                    │ buffer       │
 └────────────┘             └──────────────────────┘                    └──────┬───────┘
 ┌────────────┐  f32 PCM          ▲                                           │ F8
 │ WASAPI     │ ──── mixer ───────┘ stdin                                     ▼
 │ loopback + │   (Rust, cpal)                                    stream-copy → Clip.mp4
 │ microphone │
 └────────────┘
```

- **Capture:** ffmpeg's `ddagrab` (Desktop Duplication) feeds the hardware encoder, and writes 2-second MPEG-TS segments. Only the newest segments are kept on disk.
- **Audio:** Rust captures system sound (WASAPI loopback) and the microphone with `cpal`, resamples both to 48 kHz, and mixes them on a clock-driven thread. Silence still produces audio, so sound stays in sync. The mixed audio is piped into ffmpeg.
- **Saving:** the newest segments are joined and trimmed with `-c copy`, which takes no re-encoding and almost no CPU.
- **Editor:** previews are decoded at low resolution. Exports go through a single ffmpeg filter graph (`trim`/`concat`/`setpts`/`atempo`).

## 🛠️ Build from source

You need [Rust](https://rustup.rs) and a MinGW toolchain:

```powershell
winget install Rustlang.Rustup
rustup default stable-x86_64-pc-windows-gnu
winget install BrechtSanders.WinLibs.POSIX.UCRT
.\build.ps1
```

This creates `LiteClip.exe` in the project folder. The MSVC toolchain works too: `cargo build --release`.

<details>
<summary>Project layout</summary>

```
src/
├── main.rs       window setup
├── app.rs        UI: clips library, editor, settings
├── engine.rs     shared core: buffer control, hotkeys, watchdog
├── recorder.rs   ffmpeg replay buffer + clip saving
├── audio.rs      WASAPI capture, resampling, mixing
├── editor.rs     preview frames, playback, export
├── ffmpeg.rs     locating/probing ffmpeg, encoder detection
├── platform.rs   monitors, sounds, recycle bin (Win32)
└── settings.rs   settings model + persistence
```
</details>

## ❓ FAQ

<details>
<summary><b>Does it work with fullscreen games?</b></summary>

Yes with borderless/windowed fullscreen, which is what most modern games use. Most exclusive-fullscreen games on Windows 10/11 work through Desktop Duplication too. If one shows black, switch the game to borderless.
</details>

<details>
<summary><b>Why does it need FFmpeg?</b></summary>

FFmpeg has mature, well-tested GPU encoder support. Running it as a separate process keeps LiteClip small and lets you update FFmpeg on its own.
</details>

<details>
<summary><b>How much disk space does the buffer use?</b></summary>

Only the last few chunks are kept, so it uses roughly `bitrate × clip length`. At the defaults (8 Mbps, 30 s) that's about 30 MB. Settings shows an estimate.
</details>

<details>
<summary><b>Can I mute just my mic in a clip?</b></summary>

Not yet. Game sound and mic are mixed into one track. Separate tracks are on the roadmap.
</details>

## 🗺️ Roadmap

- [ ] Separate audio tracks for game sound and mic
- [ ] System tray icon
- [ ] Record a single window or game instead of the whole monitor
- [ ] GIF / WebM export
- [ ] HEVC / AV1 encoding

## 🤝 Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md).

## 📄 License

[MIT](LICENSE). You can use it, change it and share it.

<div align="center">
<sub>Built with 🦀 Rust · <a href="https://github.com/emilk/egui">egui</a> · <a href="https://github.com/RustAudio/cpal">cpal</a> · <a href="https://ffmpeg.org">FFmpeg</a></sub>
</div>

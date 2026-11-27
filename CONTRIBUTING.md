# Contributing to LiteClip

Thanks for helping. LiteClip has three goals: **light on resources, fully offline, and simple to use.** Please keep changes in line with them.

## Getting set up
See [Build from source](README.md#-build-from-source). Run `.\build.ps1` (GNU toolchain) or `cargo build --release` (MSVC).

## Pull requests
- Keep each PR focused on one fix or feature.
- Run `cargo fmt` and make sure `cargo build --release` passes without warnings.
- Test on real hardware if you can, and say in the PR which GPU/encoder you used.
- Don't add network features, telemetry or heavy dependencies.

## Reporting bugs
Use the bug report template. The ffmpeg log at `%LOCALAPPDATA%\LiteClip\buffer\ffmpeg.log` is usually the most useful thing to include.

# Changelog

All notable changes to Cued are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.1] - 2026-07-24

### Added

- **Windows builds** (beta) — Cued now ships a Windows installer alongside macOS.
  Windows support is new and not yet extensively tested; please report issues.

### Changed

- Releases are now built automatically for both macOS and Windows.

## [1.0.0] - 2026-07-24

First public release.

### Added

- **Per-song presets** — set a custom start point and skip point for any track;
  Cued remote-controls your running Spotify client so the song starts where you
  want and never overstays its welcome.
- **Timeline editor** — drag the start and skip handles on a framed timeline
  strip, nudge them with arrow keys, and preview the exact moment by ear before
  saving.
- **Automatic playback engine** — presets apply by themselves while you listen,
  with second-accurate timing even across natural track changes. A master
  toggle turns all automation off in one click.
- **Library** — every preset in one searchable list, with inline editing and
  deletion.
- **Suggestions** — Cued notices where you usually skip or jump within a song
  and quietly offers a matching preset: one click to apply, one click to undo.
  Songs you almost always skip can be skipped for you automatically.
- **Skip heatmap** — the timeline shades the parts of a song you tend to skip,
  based on your own listening.
- **Listening insights, local-only** — skip/seek history is stored only on your
  computer, is never uploaded, and can be deleted at any time from Settings.
- **Menu-bar mode** — closing the window keeps Cued running in the tray/menu
  bar with playback info and quick controls.
- **Guided setup** — a 3-step wizard walks you through creating your free
  Spotify Client ID and connecting your account (~3 minutes, one time).
- **Support link** — an optional "Support Cued" link in Settings for anyone who
  wants to leave a tip. Cued is free either way; nothing is locked.

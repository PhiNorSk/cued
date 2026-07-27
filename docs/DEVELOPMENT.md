# Development

Cued is a Tauri v2 + React 18 + TypeScript desktop app. Rust backend in
`src-tauri/`, UI in `src/`. This file holds everything the public README
deliberately leaves out.

## Prerequisites

- [Node.js](https://nodejs.org/) ≥ 20
- [Rust](https://rustup.rs/) (stable toolchain — required by Tauri)
- macOS: Xcode Command Line Tools (`xcode-select --install`)
- Windows: [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) + WebView2 (preinstalled on Windows 11)

## Install & run

```sh
npm install
npm run tauri dev
```

The first run compiles the Rust side and takes a few minutes; subsequent runs
are fast.

## Scripts

| Command               | What it does                            |
| --------------------- | --------------------------------------- |
| `npm run tauri dev`   | Run the desktop app in development mode |
| `npm run typecheck`   | TypeScript check (`tsc --noEmit`)       |
| `npm run lint`        | ESLint, warnings treated as errors      |
| `npm run test`        | Run unit tests with Vitest              |
| `npm run tauri build` | Build a distributable bundle            |

For the Rust side, run these from `src-tauri/`:

```sh
cargo fmt      # format
cargo clippy   # lint
```

## Icons & DMG branding

Every brand binary (app icon set, DMG background, tray templates) is generated
from checked-in, stdlib-only Python scripts — no image tools or design files
needed. To rebuild after a logo or token change:

```sh
# App icon: render the 1024x1024 master, then regenerate the whole bundle set
python3 scripts/gen-app-icon.py /tmp/cued-icon-master.png
npx tauri icon /tmp/cued-icon-master.png

# DMG installer-window background (660x400 pt, rendered @2x with 144-dpi
# metadata so Finder shows it crisp on Retina)
python3 scripts/gen-dmg-background.py src-tauri/dmg/dmg-background.png

# Tray (menu-bar) template icons
python3 scripts/gen-tray-icons.py src-tauri/icons/tray
```

`npx tauri icon` also emits `ios/` and `android/` icon sets — delete them
(desktop-only project).

The DMG window layout (window size, icon positions) lives in
`src-tauri/tauri.conf.json` under `bundle > macOS > dmg` and must stay in sync
with the `APP_X` / `FOLDER_X` / `ICON_Y` constants in
`scripts/gen-dmg-background.py`.

## Unsigned builds

`npm run tauri build` produces an **unsigned** `.dmg` (signing/notarization is
a planned follow-up). macOS Gatekeeper blocks a plain double-click on first
launch: right-click `Cued.app` → Open → Open once; after that it opens
normally.

## Releasing

Releases are built by CI (`.github/workflows/release.yml`), not locally:

1. Bump the version in `package.json`, `src-tauri/tauri.conf.json` and
   `src-tauri/Cargo.toml` (let npm/cargo sync the lockfiles).
2. Commit, then tag and push:

   ```sh
   git tag v1.2.3
   git push origin v1.2.3
   ```

3. GitHub Actions builds on macOS (Apple Silicon `.dmg`) and Windows
   (`.msi` + NSIS setup `.exe`), running typecheck/lint/tests/clippy/fmt
   first, and attaches everything to a **draft** GitHub Release for the tag.
4. Review the draft on the Releases page, edit notes, and publish manually.

Both installers are currently unsigned. Signing secrets can be added to the
`tauri-action` step's `env` block later without restructuring the workflow.

## Structure

```
src/            React UI (TypeScript)
  components/   UI components
  lib/          shared helpers
src-tauri/      Rust backend + Tauri config
docs/           project docs + README assets
```

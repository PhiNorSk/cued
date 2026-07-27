# STATE — Cued (handoff between tickets)

Updated after: **M15 — Release CI (macOS + Windows)** (2026-07-24).
M15 adds `.github/workflows/release.yml`: pushing a tag `v*` builds UNSIGNED
installers on GitHub runners — macOS `.dmg` (Apple Silicon) + Windows `.msi`
and NSIS setup `.exe` — and attaches them to a DRAFT GitHub Release the human
reviews and publishes. CI + docs only, zero product-code changes.
Previous milestone: **M14 — Optional launch at login** (2026-07-24).
M14 adds an opt-in "Start Cued at login" toggle in Settings (default OFF,
registered only by the explicit user action) via `tauri-plugin-autostart`
2.5.1; a login launch starts silently into the tray (M5 accessory mode),
and the toggle reads the live OS state on every settings open.
Previous milestone: **M13 — Donations + release preparation, v1.0.0** (2026-07-24).
M13 makes the repo public-ready: MIT LICENSE (Phil Skribbe), CHANGELOG.md
(Keep a Changelog, single 1.0.0 entry), `.github/FUNDING.yml` (ko_fi),
public product README (dev docs moved to `docs/DEVELOPMENT.md`), a quiet
"Support Cued ♥" row in the Settings footer, and the version bumped to
1.0.0 everywhere. Previous milestone: **M12 — Real app icon + branded DMG**
(2026-07-24) — MVP (M0–M6)
feature-complete, M7 kills the ~1 s intro delay, M8 makes the timeline a
two-state editor, M9 records genuine skip/seek behavior locally, M10 turns that
history into calm suggestions, M11 rebuilds the timeline as a framed strip with
a skip-density heatmap, M12 replaces the Tauri placeholder icons with the real
Cued icon and brands the DMG installer window. First testable DMG exists
(unsigned — testers right-click → Open).

## Built so far
- M15 release CI (`.github/workflows/release.yml` + README/DEVELOPMENT docs):
  - Trigger: push of a tag matching `v*`. Flow: a `create-release` job
    (ubuntu, plain `gh` CLI, idempotent on re-runs) creates ONE draft
    release for the tag and outputs its id; a `build` matrix job
    (macos-latest = arm64 → aarch64 `.dmg` like the manual build;
    windows-latest → `.msi` + NSIS `-setup.exe` via `bundle.targets: "all"`)
    uploads into that draft via `tauri-action`'s `releaseId` input. The
    single create job exists BECAUSE two matrix jobs racing to create the
    same draft can produce duplicates. `fail-fast: false` — a Windows
    failure never cancels the macOS artifact.
  - Quality gates run before bundling on BOTH platforms and fail the job:
    `npm run typecheck`, `lint`, `npm test`, `cargo fmt --check`,
    `cargo clippy -- -D warnings`, `cargo test`. GOTCHA: they are one
    command per step ON PURPOSE — on windows-latest (pwsh) a multi-line
    `run` block only propagates the LAST command's exit code.
  - Pinned versions (supply-chain rule): `actions/checkout@v7.0.1`,
    `actions/setup-node@v7.0.0` (Node 22, npm cache),
    `tauri-apps/tauri-action@v1.0.0` (NOTE: v1 renamed/removed several
    inputs vs the widely-googled v0 examples — input names were verified
    against the v1.0.0 action.yml). Rust comes from the runner's rustup
    (`rustup update stable`), no third-party toolchain action.
  - UNSIGNED by design; zero repo secrets needed (only the built-in
    `GITHUB_TOKEN`, `permissions: contents: write`). Future signing
    secrets go into the tauri-action `env` block — structure stays.
    `uploadUpdaterJson: false` (no updater configured).
  - Windows was NEVER built or run anywhere yet (no Windows machine).
    Pre-checks done this ticket: all platform-specific Rust is properly
    `#[cfg]`-gated (activation-policy calls, tray click behavior — audited
    via grep), so no compile blocker is known. A local
    `cargo check --target x86_64-pc-windows-msvc` was attempted and is NOT
    feasible on macOS: C-source deps (`ring`, bundled sqlite) need a
    Windows C toolchain. The first real Windows compile happens in CI.
  - Windows runtime UNKNOWNS for a follow-up ticket (expected issue areas,
    none verified): keyring → Credential Manager storage; tray icon +
    left-click-opens-window behavior; 127.0.0.1:8917 OAuth loopback vs
    Windows Firewall (may prompt or block the callback); SmartScreen on
    the unsigned installer (documented in README). File findings here.
  - README: Requirements now macOS 13+ OR Windows 10/11 (beta); Install
    split into macOS / Windows (beta) sections with first-launch notes
    (right-click → Open; SmartScreen More info → Run anyway) + an issue
    ask. DEVELOPMENT.md gained a "Releasing" section (bump version → tag
    → push → review draft → publish).
- M14 launch at login (opt-in):
  - Dependency: `tauri-plugin-autostart` **2.5.1** (Rust crate ONLY — no npm
    guest bindings, no capability change; the frontend goes through our own
    commands like every other toggle, same discipline as the opener).
    Registered in `lib.rs` with `MacosLauncher::LaunchAgent` + the
    `--autostart` launch arg (`tray::AUTOSTART_ARG`).
  - LaunchAgent (not AppleScript) BY DECISION: only the LaunchAgent path can
    pass launch args (required for the silent tray start — AppleScript login
    items cannot) and it needs no Automation/TCC consent prompt. Enable
    writes `~/Library/LaunchAgents/Cued.plist` (name = productName); on
    macOS 13+ the entry appears in System Settings > General > Login Items
    under **"Allow in the Background"**, not in the "Open at Login" list.
  - `is_enabled` = plist existence (auto-launch crate, verified in source).
    Deleting the entry outside Cued IS reflected on the next settings open;
    flipping the System-Settings background-item switch OFF is NOT (launchd
    honors the switch but the plist stays, so Cued's toggle still reads on).
    Known upstream limitation — accepted and documented, don't chase it.
  - Silent login start: the main window is now created HIDDEN
    (`"visible": false` in tauri.conf.json) and `tray::apply_launch_visibility`
    (called last in setup) decides: normal launch → show + focus (pre-M14
    behavior); `--autostart` present → `hide_to_tray` (Accessory policy, no
    window, engine untouched). Config windows are created BEFORE the setup
    hook runs (verified in tauri 2.11.5 source), so the lookup is safe. The
    single-instance callback also skips `show_main_window` when the second
    launch carries `--autostart` (a login-item relaunch never pops a window).
  - Commands `get_autostart_enabled` / `set_autostart_enabled` in
    `commands.rs` (via `ManagerExt::autolaunch()`; errors map to
    `AuthError::Config` → `{code, message}` like everything else). The state
    deliberately does NOT live in config.json — the OS is the single source
    of truth, queried live. TS: wrapper `src/lib/autostart.ts`, hook
    `src/hooks/useAutostart.ts` (reads the real state on every panel mount =
    every settings open; optimistic set with rollback + inline error),
    "Startup" section in `SettingsPanel.tsx`, strings in `settingsCopy`.
  - Pure `tray::is_autostart_launch` (exact-match flag detection) is
    unit-tested (3 new cargo tests → 204 total).
- M13 donations + release prep (v1.0.0):
  - Support link: new Rust command `open_support_page` in `commands.rs`
    (fixed `SUPPORT_URL` = `https://ko-fi.com/phinorsk` through the
    opener plugin — same fixed-URL discipline as `open_spotify_dashboard`;
    the frontend can never open arbitrary links), registered in `lib.rs`;
    TS wrapper `openSupportPage` in `src/lib/appInfo.ts`. UI: the Settings
    footer is now "Support Cued ♥ · Cued v1.0.0" (`SettingsPanel.tsx`,
    strings in `settingsCopy`); a failed browser open shows a quiet inline
    error. DELIBERATE: no banners, no nag, no feature gating — donations
    are a tip jar only (Spotify ToS + product principle).
  - Repo files: `LICENSE` (MIT, "Phil Skribbe"), `CHANGELOG.md`
    (Keep a Changelog, one 1.0.0 entry of user-facing features),
    `.github/FUNDING.yml` (`ko_fi: phinorsk`).
  - README rewritten for the public (product only, no dev tooling): hero
    image placeholder `docs/assets/hero.png` (real PNGs pending),
    what/why, features, Requirements box (macOS 13+, Spotify Premium,
    free Client ID via wizard), DMG install incl. right-click → Open,
    FAQ (own Client ID / nothing uploaded / never touches volume / ±1 s
    precision), one-line Ko-fi + star ask, MIT line, "not affiliated
    with Spotify" trademark footer. The old dev sections (prereqs,
    scripts, icon/DMG regen commands, structure) moved VERBATIM-ish to
    `docs/DEVELOPMENT.md` — future tickets: regen commands live THERE now.
  - Version 1.0.0 in `package.json`, `src-tauri/tauri.conf.json`,
    `src-tauri/Cargo.toml`; lockfiles synced by npm/cargo (not by hand).
    The Settings about line shows it via the existing `getVersion` path.
  - "Spotify" audit: all UI strings + README use the name descriptively
    only; no Spotify logo assets anywhere (connect button is text-only,
    all icons are generated Cued brand marks).

## Release checklist (v1.0.0)
- [x] LICENSE / CHANGELOG / FUNDING.yml / public README
- [x] Version 1.0.0 everywhere; shown in Settings about line
- [x] typecheck, lint, vitest (128), cargo test (201), clippy, fmt green
- [x] 1.0.0 DMG built
- [ ] phinorsk placeholder → real Ko-fi name (FUNDING.yml, README,
      `SUPPORT_URL` in `commands.rs`) — HUMAN
- [ ] Real screenshots into `docs/assets/` (hero.png referenced) — HUMAN
- [ ] Manual test: support link opens Ko-fi, about shows 1.0.0 — HUMAN
- [ ] Commit, push, publish repo + GitHub Release with the DMG — HUMAN
- [ ] Post-release backlog: signing/notarization (Windows CI build shipped
      in M15 — runtime verification on real Windows still open)

- M12 packaging (no app-code changes — scripts + config + generated assets):
  - Brand-asset toolchain: `scripts/cued_render.py` is a shared stdlib-only
    Python rasterizer (analytic shapes + adaptive supersampling + PNG writer,
    same philosophy as `gen-tray-icons.py`; includes the Logo.tsx mark geometry
    as `logo_mark()`). Two generators sit on top:
    - `scripts/gen-app-icon.py <out.png>` — 1024×1024 master: Apple-grid
      rounded square (824/1024, corner 22.5%), vertical `--surface`→`--ground`
      gradient, mark at 72% of the square (ring `--accent`, dot `--accent-hi`,
      triangle `--text`), transparent margins. Feed it to `npx tauri icon
      <out.png>` to regenerate the ENTIRE bundle set (icns/ico/PNG/Square*) —
      no hand-edited sizes. The master itself is NOT checked in (the script is
      the source); tray templates in `icons/tray/` are untouched.
    - `scripts/gen-dmg-background.py src-tauri/dmg/dmg-background.png` —
      DMG window background, 660×400 pt rendered @2x (1320×800) with a 144-dpi
      `pHYs` chunk (Finder reads the DPI and shows it at point size → crisp on
      Retina from ONE file; Tauri's dmg `background` accepts png/jpg/gif only,
      so no multi-res TIFF). Design: `--ground` + faint radial glow toward
      `--surface-2`, mark + stroke-built "Cued" wordmark (geometric letterforms
      drawn as arcs/segments — no font dependency) at top, no text. The
      "drag right" instruction is a SOUNDWAVE (user direction 2026-07-24,
      replaced the plain arrow): pill bars swelling toward Applications,
      `--text-mut` fading into `--accent-hi`, small chevron tip.
    - Finder draws the DMG icon labels itself — black in light mode, white in
      dark mode, NO override exists (the `.DS_Store` icvp plist has no text
      color key). Fix for black-on-dark labels: soft mid-grey (`--text-mut` @
      0.78) spotlight capsules (flat 22 pt core + 8 pt smoothstep feather)
      painted into the background behind both label zones (~4.5:1 for black
      AND white text). The label line sits 20 pt below the icon bottom —
      MEASURED, not derived: the script has a `--grid` flag that overlays a
      5 pt calibration grid; a grid build was opened in Finder and the label
      glyph rows were read off a screenshot (user-confirmed centered
      2026-07-24). If Tauri's bundle_dmg.sh icon geometry (128 px icons,
      16 pt labels) ever changes, redo that calibration.
  - DMG layout in `tauri.conf.json` (`bundle > macOS > dmg`): background path,
    windowSize 660×400, appPosition (170,195), applicationFolderPosition
    (490,195). These MUST stay in sync with `APP_X`/`FOLDER_X`/`ICON_Y` in
    `gen-dmg-background.py` (the arrow is painted between those positions).
  - `tauri icon` also emits ios/ + android/ icon sets — deleted (desktop-only
    project); delete them again after any future re-run. `__pycache__` is now
    gitignored. README documents the regen commands + the unsigned-build
    right-click → Open note for testers.

## Built so far (feature milestones)
- M11 timeline redesign "Studio Bracket" + skip-density heatmap (pure bucketing
  in `heatmap.rs`, command in `commands.rs`, pure sampling/coloring in `src/lib/heatmap.ts`,
  hook `useHeatmap.ts`, full rebuild of `PresetTimeline.tsx`, tokens/keyframes in
  `index.css`):
  - The timeline is now a REGION control, not a value slider: a ~48 px strip
    (radius 9 px, `--strip` = rgba(255,255,255,.04) token) filled with a FAKE
    waveform (`WaveTexture`, `BAR_COUNT`=48 rounded PILL bars in a flex row —
    real px geometry, no non-uniform SVG stretch). Bar HEIGHTS come from a fixed,
    left-right SYMMETRIC envelope (`fakeWaveHeights`, module const `WAVE`) —
    identical for every track BY DESIGN and visibly regular/symmetric so it
    reads as decorative, encodes no audio, and can never masquerade as a real
    waveform (no randomness). The bars are GREY and carry no data. NO diagonal
    hatching (M8 `HATCH` gone), NO separate amber curve band (superseded).
  - Active region is FRAMED: 1.5 px top/bottom rails + full-height ~13 px glyph
    end-caps (START = accent-green ▶, SKIP = amber »; glyphs make the handles
    colorblind-safe), rounded OUTER corners, `cursor: ew-resize`. VIEW shows the
    strip + dimmed zones + thin frame, NO caps; EDIT adds the caps + drag/keys +
    the save/cancel row (all M8 semantics — preview, restore, edit gate —
    unchanged, `usePreset` untouched).
  - Labels: NO flag chips (removed). Fixed readouts BELOW the strip corners
    (`Start m:ss` green, `Skip m:ss` amber, duration right, tabular-nums); a
    small mono time tooltip sits above a cap ONLY while hovering/dragging.
    Playhead is a 1.5 px near-white line with a glow dot, `z-30` above the caps.
  - Micro-interactions (all ≤200 ms, disabled under `prefers-reduced-motion`):
    cap hover scale 1.15 + soft glow (opacity-only blurred underlay), grab
    scale 1.2 + stronger glow + active-zone brighten, a single 150 ms
    `anim-zone-pulse` on release. The ONLY non-transform/opacity motion is the
    cap's own `left` (90 ms `.tl-cap`) so snapping to whole seconds glides the
    last pixels — documented in `index.css`; also reduced-motion-gated.
    Position transforms are split (`-translate-x-1/2` on the cap stays static;
    scale lives on `.tl-cap-knob` so reduced motion can cancel just the scale).
  - Heatmap = a soft gradient overlay OVER the grey bars (no separate band),
    DECOUPLED from bar thickness so it stays accurate at the full 100-bucket
    resolution. Rust `heatmap::compute` buckets a track's SKIP-AWAY positions
    (`skip_next` ≥ 15 s + any `seek_forward` `from_ms`) into `HEATMAP_BUCKETS`
    (100) segments and peak-normalizes; needs ≥ `HEATMAP_MIN_EVENTS` (8) eligible
    events or it returns None. Excludes the < 15 s song-REJECTIONS (same
    `REJECTION_WINDOW_MS` rule) and `seek_back` rewinds. TS `heatGradient` builds
    a `linear-gradient(90deg,…)` with one `heatStopColor` stop per bucket:
    transparent at density 0 (grey wave shows through) → amber → dark red
    (`--heat`), alpha smoothstep→ `HEAT_MAX_ALPHA` (0.7). "none" when data is
    insufficient/insights off (→ no wash, plain grey wave, no gap). A quiet
    "You usually skip this part" hint shows on strip hover in VIEW when there is
    real density. Fetched at most once per track change by `useHeatmap` (gated on
    `insights_enabled`), never on the poll path — reads the same events the
    suggestion engine does.
    - NOTE: `heatStopColor`'s RGB endpoints mirror the `--amber`/`--heat` tokens
      numerically (the gradient color is computed in JS) — keep hex + token in
      sync.

- M10 suggestions (pure analysis in `suggestions.rs`, storage in `presets.rs`,
  commands in `commands.rs`, engine hook in `player.rs`/`automation.rs`, UI in
  `SuggestionCard.tsx` / `Library.tsx` / `SettingsPanel.tsx`):
  - PURE analysis engine `suggestions.rs` (events in → suggestions out, no I/O,
    no UI). All thresholds are named consts: `REJECTION_WINDOW_MS` 15 s,
    `EARLY_SEEK_WINDOW_MS` 15 s, `CLUSTER_RADIUS_MS` ±5 s, `DECAY_HALF_LIFE_MS`
    90 d, `PLAY_SESSION_GAP_MS` 8 min, `MIN_PLAYS` 5 / `MIN_MATCH_RATIO` 0.70,
    `AUTO_SKIP_MIN_PLAYS` 10 / `AUTO_SKIP_MIN_RATIO` 0.90, `IGNORE_RETIRE_COUNT`
    3. Classification: `skip_next` with `from_ms < 15 s` = SONG REJECTION
    (auto-skip evidence, excluded from skip clustering); early `seek_forward`
    (`from_ms < 15 s`) = START-POINT signal (target = `to_ms`); `skip_next` /
    `seek_forward` mid-track = SKIP-POINT signal (`from_ms`). `seek_back` is
    ignored. Clustering picks the densest ±5 s neighborhood weighted by
    recency; recent plays dominate the chosen value and the ratio.
  - **"A play" = a temporal cluster of a track's events** (`PLAY_SESSION_GAP_MS`
    gap-based sessionization). M9 records no completions, so a play with zero
    events is invisible; denominators are "plays in which you did something".
    Skip-point denominator excludes rejection plays (the user bailed before the
    mid-track point); start-point and auto-skip use all observed plays. This is
    documented in the module header as an honest floor, not a lie.
  - Suggested values: skip point = weighted cluster start rounded to whole
    seconds (region min→max shown as "1:12–1:48"); start point = weighted
    median of early-seek targets; auto-skip = a flag, no value. Tracks whose
    stored preset already covers the region (skip at/before it, or start
    at/after it) do NOT generate that suggestion type.
  - Lifecycle state machine (pure fns): `status_after_analysis` (never
    resurrects `dismissed`, preserves `applied`/`retired`, else `active`),
    `analysis_may_update` (dismissed = frozen), `status_after_ignore`
    (active + 3 ignores → retired). Re-analysis refreshes VALUES but never a
    status a user set.
  - Engine auto-skip: `Automation::set_auto_skip(bool)` is called by the loop
    each poll from the applied-flag lookup (defaults false → zero churn to the
    68 existing `on_poll` tests). It fires `SkipNext` through the SAME
    `Gates::all()` + staleness + cooldown + action-cap checks as every action,
    BEFORE the `cue?` bail (an auto-skip track may have no preset). A manual
    seek into the song sets a per-instance `auto_skip_suppressed` — we never
    fight a deliberate listen; a new instance re-arms it.
  - Analysis runs opportunistically (Now Playing track change, window focus,
    Library open), debounced by the frontend and coalesced by an
    `analysis_running` guard; NEVER in the poll hot path. Bounded to tracks
    with events newer than the `analysis_state` cursor (`last_event_id`).
  - Surfaces: ONE proactive card in Now Playing (strongest ACTIVE suggestion of
    an enabled type; `pickCardSuggestion`), hairline border + ✦ glyph
    (`text-accent/65`) + single 200 ms `anim-fade-in`, no pulse, never over
    artwork/controls. Accept applies instantly (saved as a normal preset for
    skip/start; `applied` flag for auto-skip) and morphs to Adjust/Undo
    (Adjust = `usePreset.adoptSaved` then `enterEdit`). Library gets a
    collapsible "Suggestions (n)" section (muted header + count chip, ✦ rows,
    inline accept/dismiss, applied-auto-skip badge + "Turn off"). Settings gains
    3 per-type toggles + the provenance line (shown once).
- M9 listening-insights collection (pure classification in `automation.rs`,
  storage in `presets.rs`, background writer in `insights.rs`, UI in
  `SettingsPanel.tsx`):
  - Every GENUINE manual skip/seek is recorded to the local DB, keyed to the
    track. Three event kinds: `seek_forward` / `seek_back` (with `from_ms`
    = interpolated position left, `to_ms` = landing spot) and `skip_next`
    (`from_ms` = last observed position, `to_ms` NULL — user pressed next
    before the natural end). Collection ONLY — no suggestions, badges, or
    heatmap (those are v1.1 / M10–M11).
  - Classification is a PURE function of observations, living inside
    `Automation::on_poll` and REUSING the existing M4 manual-seek detection
    (the same interval/jump math) — not a parallel reimplementation. Events
    are queued in `Automation.events` and drained by the loop via
    `take_events()` after every poll. The pure module holds NO display
    metadata and NO I/O; it emits `automation::InsightEvent`
    (uri/kind/from/to/duration).
  - What is NEVER recorded (tested): Cued's own actions (start-jump, skip,
    M8 preview & exit-restore seeks) — detected via the existing `rebase`
    flag and `note_external_seek()`; natural track ends (last position within
    `NATURAL_END_WINDOW_MS` = 5 s of duration); episodes/local files
    (non-controllable obs); anything while the playing track's preset is being
    edited (edit mode is a sandbox — `Gates::records_insights()` =
    `insights_on && !edit_hold`); and repeat-one restarts (a new instance,
    not a backward seek).
  - The insights toggle is a NEW field on `Gates` (`insights_on`) that is
    deliberately NOT part of `Gates::all()` — it gates ONLY recording, never
    actions. So manual skips are recorded even with automation OFF (still
    genuine behavior). The engine-level gate (not just the UI) is verified by
    the `nothing_is_recorded_while_insights_are_off` test.
  - Writes never touch the poll hot path: the loop enqueues onto a bounded
    (256) `tokio::mpsc` via `InsightsSink::record` (`try_send`, full → log +
    drop); a single background task drains it into the shared `PresetStore`.
    A failed write is logged and dropped — playback control is never
    disturbed.
  - Storage is APPEND-ONLY by explicit product decision: NO caps, NO pruning,
    NO expiry. The ONLY deletion path is the user's "Delete all insights
    data" action (`delete_all_insights` — both tables in one transaction,
    presets untouched). Cued never prunes on its own.
  - Settings surface behind a NEW header gear (`SettingsPanel.tsx`, a modal;
    `useInsights` hook): "Listening insights" toggle (default ON, persisted in
    config.json via `insights_enabled`), a live "N events collected" count
    (loaded each time the panel opens), delete-all with an inline confirm, and
    the app version line (MOVED here from the old bottom `AboutFooter`, which
    was removed — version now shows only in Settings, i.e. only when
    connected). TS pure helper `insightsCountLabel` (pluralization) is
    unit-tested.
- M8 edit mode + timeline redesign + preview + neutral-preset fix:
- M8 edit mode + timeline redesign + preview + neutral-preset fix:
  - Two-state Now Playing timeline (`PresetTimeline.tsx`, one component,
    `mode` prop): VIEW is a calm progress bar with slim start/skip markers
    (zero-effect points draw none: start 0, skip == duration); EDIT is the
    full editor — 10 px pill in a 56 px touch area, hatched trim zones,
    gradient active zone with a soft glow (blurred token-color underlay, no
    hardcoded colors), 20 px circular handles (drag + arrows ±1 s / shift
    ±5 s, plain `focus:` styling ON PURPOSE — the focused handle is the
    preview target, so selection must stay visible after clicks), flag
    chips that anchor to the midpoint and repel instead of overlapping
    (`flagsRepel`, threshold `FLAG_REPEL_BELOW_PCT`). "Edit preset" /
    "Set preset" switches states.
  - Engine-level edit gate: `Gates.edit_hold` joined the pure gate list in
    `automation.rs` (blocks actions, wakeups, bursts, queue fetches).
    `AppState.edit_hold: Mutex<Option<String>>` holds the edited track URI;
    the poll loop compares it per observation, so ONLY the edited track is
    suspended — anything else playing is automated as usual. The tray/master
    toggle is untouched. `set_edit_mode(track_uri|null)` wakes the loop so
    the gate lands before any already-scheduled boundary one-shot fires
    (same discipline as the master toggle). UI enters edit mode only AFTER
    the IPC succeeds.
  - Preview-by-ear: "Listen from here" seeks to the focused handle's point
    minus `PREVIEW_PREROLL_MS` (3 s, clamped at 0) via the new `ui_seek`
    command → existing `spotify::seek` path (no new endpoints). Debounced
    by `PREVIEW_COOLDOWN_MS` (2 s, mirrors ACTION_COOLDOWN_MS; computed
    against `nowMs` from the playback tick — no timers). UI seeks NEVER
    touch automation bookkeeping: `ui_seek` sets `AppState.ui_seek_pending`,
    the loop calls `Automation::note_external_seek()` (absorbs the next
    observation); additionally the manual-seek suppression is skipped
    entirely while `gates.edit_hold` (edit mode is a sandbox). Non-Premium
    hides the control behind the quiet notice pattern (`isPremium` prop now
    flows App → NowPlaying).
  - Enter/exit: entering remembers the interpolated position (NO seek on
    entry); save AND cancel seek back via `restoreTargetMs` — only if the
    same track still plays. Track change (or unmount/tab switch) while
    editing releases the gate silently without seeking. Cancel discards the
    draft entirely.
  - Neutral-preset fix: start 0 + skip == duration is "no preset".
    `validate_times` (Rust, authoritative) now REJECTS neutral writes —
    note `validate_times(0, dur, dur)` flipped from ok to error, the old
    "skip == duration allowed" test now uses start > 0. TS mirrors via
    `isNeutralPreset`/`saveActionFor`: neutral draft with nothing stored →
    Save disabled + "Nothing to save — move a handle first"; editing an
    EXISTING preset back to neutral → Save DELETES it (state line "Preset
    removed — song plays normally", new phase "removed"). Old
    `usePreset.save/reset` API became `enterEdit/cancelEdit/save/preview`.
- M7 predictive start-jump (pure decisions in `automation.rs`, execution in
  `player.rs`, new endpoint in `spotify.rs`):
  - `GET /v1/me/player/queue` (`fetch_queue`/`parse_queue_body`, tolerant
    `QueueResponse`/`QueueItem`) tells the engine the LIKELY next track
    before the transition. Fetched once per playback instance on the track
    change and once more when < `PREDICT_HORIZON_MS` (15 s) remain to the
    next transition boundary (skip point if our skip will fire, else track
    end) — hard-capped at `MAX_QUEUE_FETCHES_PER_INSTANCE` (2, failed
    fetches count). ZERO fetches while automation is off / not Premium /
    device-restricted / paused (`wants_queue_fetch`).
  - If the predicted track has a preset with `start_ms > 0` it is
    pre-armed (`Prearmed`, built by `player.rs::prearm_from` from
    `predicted_next` = FIRST queue entry only; episode/local/uri-less →
    no prediction). A prediction is a HINT: it only makes the engine
    confirm the transition faster — the seek always comes out of the
    normal M4 start-jump path on a confirmed observation, so cap,
    cooldown, manual-seek suppression, staleness and premium/403/429
    gates apply unchanged by construction. A mismatch (shuffle/autoplay
    surprise) is discarded on `begin_instance`; queue errors degrade to
    "no prediction" (`queue_fetch_failed` still counts the attempt).
  - Transition burst: a strictly bounded run of fast polls to confirm the
    new track — `BURST_POLL_COUNT` (3) polls at `BURST_POLL_SPACING_MS`
    (300 ms), never longer than `BURST_MAX_TOTAL_MS` (1.5 s) total, never
    while a 429 Retry-After is pending (`note_rate_limited` cancels it;
    rate-limited iterations never reach the burst scheduling). Started
    (a) by our own successful SkipNext when a pre-arm exists (inside
    `action_executed` — the immediate post-action re-poll is burst poll
    #1) and (b) by a natural-end one-shot: `plan_transition_wakeup_ms`
    wakes `TRANSITION_WAKE_LEAD_MS` (200 ms) before the interpolated
    track end, `on_transition_wakeup` starts the burst (NEVER an action).
    `plan_burst_delay_ms` then replaces the 1 s cadence until the burst is
    spent; the baseline cadence is untouched everywhere else.
  - Result: on a NATURAL transition into a preset track the start jump
    fires ~300 ms after the change (log line
    `cued: predictive start-jump: transition→seek N ms (uri)` measures
    it; also `queue prediction: up next …` / `transition burst begins …`
    / `queue prediction missed …`). After OUR OWN skip the M4 cooldown
    (2 s, deliberately unchanged) still floors the next seek — the burst
    confirms fast, the seek fires on the first poll past the cooldown.
    Loosening that would mean changing `ACTION_COOLDOWN_MS` semantics
    (an M4 rule — out of M7's scope by ticket).
  - `player.rs` also got `query_cue` (uncached preset lookup, shared by
    `lookup_cue` and the pre-arm), the `OneShot` enum (skip boundary vs
    transition wakeup — soonest wins, only when sooner than the next
    poll), and a log-only latency probe (`burst_probe`, expires after
    `BURST_PROBE_EXPIRY` 10 s).
- M6 onboarding wizard + polish pass:
  - Guided 3-step setup replaces the old ConnectPanel (deleted): step 1
    "create your free Spotify app" (why-text, dashboard button, numbered
    instruction card, copyable redirect URI), step 2 validated Client-ID
    paste (live valid ✓ / invalid-after-blur, reuses `clientId.ts`), step 3
    connect (waiting → "Connected as … ✓" 1.5 s → app, or friendly error +
    retry). Navigation is a pure tested machine (`src/lib/wizard.ts`):
    advance/back/goTo(reached-only)/restartForNewClientId. A stored Client
    ID skips straight to step 3 ("Use a different Client ID" escape hatch
    forgets it and restarts at 1) — this is also the authLost/logout path.
    Wizard state survives hide/show (plain component state; the window is
    hidden, never unmounted).
  - New Rust command `open_spotify_dashboard` (fixed URL through the
    opener plugin — the frontend cannot open arbitrary links).
  - `src/lib/clipboard.ts`: async clipboard API with a hidden-textarea
    execCommand fallback (the dev origin is not a secure context, so
    navigator.clipboard may be absent); "Copied ✓" confirmation for 2 s.
  - Error copy now lives in ONE place: `src/lib/errorCopy.ts`
    (`friendlyAuthMessage`) maps every auth/IPC error code to plain words;
    unmapped codes (e.g. preset validation) fall back to the backend's
    display-ready message. All wizard/app strings sit in `src/lib/copy.ts`
    for a later i18n pass; its `REDIRECT_URI` must match `server.rs`.
  - Polish: animated EQ bars in the Now Playing eyebrow (`eq-bounce`,
    static bars under prefers-reduced-motion), `anim-rise-in` 180 ms
    opacity/translate transition on tab switch / wizard step / save
    confirmation, Library empty-state card, focus-visible rings on the
    remaining link-style buttons, about footer "Cued v<version>"
    (`src/lib/appInfo.ts` → getVersion), window min size 720×560.
  - CSP closed (was the last M0 leftover): strict production CSP in
    tauri.conf.json — script-src 'self', img-src 'self' + https://i.scdn.co
    (cover art), connect-src ipc: + http://ipc.localhost only (the frontend
    makes no network calls itself). style-src keeps 'unsafe-inline':
    React inline style attributes (timeline/progress positioning, EQ bar
    timing) require it. `devCsp` additionally allows the Vite dev server
    (ws://localhost:1420 for HMR, inline script for the react-refresh
    preamble).
- M5 tray / menu-bar mode (`tray.rs` + lifecycle wiring in `lib.rs`):
  - Closing the window hides it (close intercepted via `WindowEvent::
    CloseRequested` + `prevent_close`); on macOS the activation policy flips
    to Accessory (Dock icon gone) and back to Regular on reopen. The engine
    is untouched by hide/show. Quit (tray item) stops the engine via the
    existing generation bump, then `app.exit(0)`; `RunEvent::Exit` stops it
    on every other exit path (e.g. Cmd+Q). `RunEvent::ExitRequested` with
    `code: None` is prevented as a safety net (window destroyed ≠ quit).
  - Native tray menu: disabled now-playing line ("Title — Artist", ≤40
    chars, char-safe truncation, "Nothing playing" fallback), checkable
    "Automation", "Open Cued", "Quit Cued". Pure mapping in
    `tray::menu_model`/`now_playing_line` (unit-tested). Item handles are
    managed state (`TrayHandles`); the now-playing line updates from
    `player.rs::maybe_emit` (same changed-only discipline, plus a
    last-line guard so heartbeats never rewrite the menu).
  - Toggle sync both ways: `commands::apply_automation_enabled` is the
    single path (config + AtomicBool + wake + tray checkbox + new
    `automation://enabled` event). The IPC command and the tray item both
    call it; `useAutomation` subscribes to the event so the in-app pill
    follows tray toggles. On a failed save the tray checkbox reverts.
  - Rust-side wake sources (STATE.md M4 warning resolved): tray clicks and
    menu interactions call `player::start` + `wake()` (idempotent). NEW
    const `POLL_HIDDEN_SUSPENDED` (30 s) in `player.rs`: while the window
    is hidden AND polling is suspended, the loop slow-polls instead of
    parking (pure decision: `plan_suspended_wait(window_visible)`;
    `AppState.window_visible` AtomicBool tracks hide/show). Hiding the
    window also nudges the loop so a parked suspend re-decides.
  - Single instance: `tauri-plugin-single-instance` 2.4.3 (registered
    first); a second launch shows/focuses the running instance.
  - Icons: `icons/tray/tray-template(.png|@2x.png)` — monochrome template
    PNGs derived from the Logo.tsx geometry (macOS, `icon_as_template`);
    Windows uses the existing `icons/icon.ico` (tauri features
    `tray-icon`, `image-png`, `image-ico`). macOS shows the menu on left
    click; on Windows left click opens the window instead.
- M4 auto-skip engine (automation core + master toggle):
  - Pure decision logic in `automation.rs`: a deterministic state machine
    (`Automation`) fed by the poll loop — observation in, `None |
    SeekToStart | SkipNext` out. ALL thresholds are named consts at the top
    of that file (lead 300 ms, cooldown 2 s, manual-seek jump 2 s, action
    cap 4/instance, near-start 5 s, restart window 2 s, staleness 2 s,
    start attempts 2, wakeup horizon 1.1 s). 29 unit tests, no network.
  - Playback-instance model: new track OR same track restarting near 0
    (repeat-one / user restart) = new instance → start jump fires again,
    at most once per instance. Manual seeks (position outside the
    extrapolation interval of the last observation) are NEVER corrected;
    a manual seek into the intro suppresses the start jump for good;
    natural crossing of skip_ms still fires afterwards.
  - Boundary accuracy despite 1 s polling: after each poll the loop plans a
    one-shot (`plan_wakeup_ms`, fires `ACTION_LEAD_MS` early from the
    interpolated position); after EVERY action it re-polls immediately.
    Stale data (> 2 poll cycles) never fires.
  - Execution in `player.rs::run_loop` (same task/generation as polling, so
    control calls are trivially serialized with polls): `run_action` maps
    results — 403 → suspend automation for the active device until the
    device id changes; 429 → `Retry-After` honored via the existing exact
    sleep; timeout/5xx → single retry only after a re-poll confirms the
    state still warrants it. Failed/succeeded attempts all count toward
    the per-instance cap and the cooldown.
  - Gates (all must hold before ANY control call): master toggle on,
    Premium account (`AppState.premium`, set on every profile fetch),
    device not 403-restricted, item is a real non-local track, playing.
  - New endpoints in `spotify.rs`: `seek()` (PUT /v1/me/player/seek),
    `next_track()` (POST /v1/me/player/next), plus `device.id` parsing on
    `PlayerResponse`.
  - Preset lookup is cached per (track_uri, `AppState.presets_version`);
    `save_preset`/`delete_preset` bump the version and wake the loop, so a
    preset saved while its track plays applies on the next tick.
  - Master toggle: `automation_enabled` in config.json (absent = ON),
    `get_automation_enabled`/`set_automation_enabled` commands, pill switch
    in the header (`AutomationToggle.tsx`, `useAutomation` hook). The
    engine reports WHY it cannot act via a new `automationSuspended` field
    on the `playback://state` event (`noPremium` / `restrictedDevice` /
    `rateLimited`); the toggle area shows it as muted text, the Now
    Playing state line reads "Automation active — starts at m:ss, skips
    at m:ss" while a preset track plays.
- M3 presets (set/store/manage) + Library — NO auto-seek yet (that's M4):
  - SQLite store in Rust (`presets.rs`, rusqlite 0.40 "bundled"): file
    `cued.db` in the app data dir, table `presets` (track_uri PK, title,
    artists JSON, cover_url NULL, duration_ms, start_ms, skip_ms,
    created_at, updated_at — all times unix ms), `PRAGMA user_version = 1`
    as schema version. One connection behind a `std::sync::Mutex`; every
    write runs in a transaction. Metadata is snapshotted at save time so
    the Library never calls the Spotify API.
  - Validation is authoritative in Rust (`validate_times`/`validate_input`):
    `0 <= start < skip <= duration` and `skip - start >= MIN_GAP_MS`
    (10 000 ms), plus sanity caps on field sizes. Same rules mirrored in TS
    (`src/lib/presetLogic.ts`) for instant UI feedback.
  - Corrupt-DB rescue at startup: `PRAGMA quick_check` + open errors map to
    `Corrupt`; the bad file is renamed `cued.db.corrupt-<unix-ms>` and a
    fresh DB is created. A FUTURE schema version is refused, never renamed.
    `PresetDb` (managed state) wraps the store so an unopenable DB degrades
    to per-command errors instead of aborting the app; the UI shows a
    notice (banner in `App.tsx`, fed by `get_preset_db_health`).
  - Commands (all thin wrappers in `commands.rs`): `save_preset(preset)`,
    `get_preset(trackUri)`, `list_presets()` (newest first by created_at),
    `delete_preset(trackUri)` (idempotent), `get_preset_db_health()`.
    Errors arrive as `{code, message}` (`PresetError`), same contract as
    auth. TS wrappers in `src/lib/presets.ts` (Zod-typed, via `call`).
  - UI: `PresetTimeline` (two draggable handles: START green / SKIP amber,
    flags above, hatched trimmed zones, gradient active zone, playhead;
    pointer capture + arrow keys ±1 s / shift ±5 s when focused; clamping
    makes invalid states unreachable). `usePreset` hook loads the stored
    preset on track change (URI-keyed), tracks the dirty draft, saves,
    resets. `NowPlaying` embeds the editor for regular tracks; episodes /
    local files / tracks shorter than 10 s get the plain M2 progress bar
    plus a quiet note. Save/Reset sit under the timeline; the state line
    shows "Preset saved — starts at m:ss, skips at m:ss" / errors.
  - `Library` view: newest first, cover thumb, title, artists, green start
    chip + amber skip chip, case-insensitive search over title/artists
    (client-side, `presetMatchesQuery`), inline edit (m:ss text fields,
    same validation, saves via `save_preset`) and inline delete confirm
    (no native dialog). Refetches on mount (i.e. on each tab switch).
  - Two-tab navigation in `App.tsx` (Now Playing / Library) — plain state,
    no router. Connected layout is now a compact header (logo + wordmark +
    tabs) with a scrollable content area; the disconnected hero screen is
    unchanged.
- M2 read-only playback: Rust polling engine (`player.rs`) on
  `GET /v1/me/player`, events `playback://state` on meaningful change +
  5 s heartbeat, UI interpolates on a 250 ms tick (`usePlayback`,
  `src/lib/playback.ts`). Cadence/backoff/Retry-After rules all live as
  consts at the top of `player.rs`.
- M0/M1: Tauri v2 + React 18 + TS strict + Tailwind v4 shell; full PKCE
  auth in the system browser, loopback 127.0.0.1:8917, tokens in the OS
  keychain, session restore, premium flag in `Profile`.

## Where things are
- M11: pure bucketing `heatmap.rs` (`compute`/`bucket_index`/`is_skip_away` +
  `HEATMAP_BUCKETS`/`HEATMAP_MIN_EVENTS`, reuses `suggestions::{Event, EventKind,
  REJECTION_WINDOW_MS}`); `HeatmapDto` + `get_track_heatmap` command in
  `commands.rs` (registered in `lib.rs`, `mod heatmap`); NO schema bump (reads
  existing `listening_events`). TS: `src/lib/heatmap.ts` (Zod `getTrackHeatmap`
  wrapper + pure `heatStopColor`/`heatGradient`/`fakeWaveHeights`),
  `src/hooks/useHeatmap.ts`, full rewrite of `src/components/PresetTimeline.tsx`
  (`Strip`/`WaveTexture`/`Readouts` + `ViewBar`/`EditBar`), heatmap wiring +
  removed duplicate `TimeLabels` in `NowPlaying.tsx`, `--strip`/`--heat` tokens +
  `.tl-cap`/`.tl-cap-knob`/`.tl-glow`/`zone-pulse` in `index.css`. ConnectedCard
  now shows the plan name ("Spotify Premium"/"Spotify Free") not "Premium: yes/no".
- M10: pure engine `suggestions.rs` (analysis + lifecycle fns + all consts);
  schema v3 + `StoredSuggestion` + `refresh_track_suggestions`/
  `suggestions_for_track`/`list_suggestions`/`set_suggestion_status`/
  `ignore_suggestion`/`is_auto_skip_applied`/`events_for_track`/
  `track_duration_ms`/`tracks_with_new_events`/`analysis_cursor` in
  `presets.rs`; `auto_skip_flag`/`auto_skip_suppressed`/`set_auto_skip` in
  `automation.rs`; `lookup_track_config`/`query_auto_skip` +
  `set_auto_skip` call in `player.rs`; `SuggestionToggles` +
  `load/save_suggestion_toggles` in `config.rs`; `analyze_suggestions`/
  `get_track_suggestions`/`list_suggestions`/`accept_suggestion`/
  `undo_suggestion`/`dismiss_suggestion`/`ignore_suggestion`/
  `set_auto_skip_applied`/`get/set_suggestion_toggles` +
  `suggestions_version`/`analysis_running` on `AppState` in `commands.rs`;
  commands registered in `lib.rs`. TS: `src/lib/suggestions.ts` (wrappers +
  pure `pickCardSuggestion`/`librarySuggestions`/`isTypeEnabled`),
  `src/hooks/useSuggestions.ts`, `src/components/SuggestionCard.tsx`,
  Suggestions section in `Library.tsx`, toggles in `SettingsPanel.tsx`,
  `suggestionsCopy` in `copy.ts`, `anim-fade-in` in `index.css`,
  `usePreset.adoptSaved`. Dev seeding: `scripts/seed-insights.sh`.
- M9: pure classification + `InsightEvent`/`InsightKind`/`Gates.insights_on`/
  `take_events`/`NATURAL_END_WINDOW_MS` in `automation.rs`; schema v2 +
  `InsightWrite` + `record_event`/`insights_count`/`delete_all_insights` in
  `presets.rs`; background writer + bounded sink in `insights.rs` (managed
  state); `AppState.insights_on` + commands `get/set_insights_enabled`,
  `get_insights_count`, `delete_all_insights` in `commands.rs`;
  `insights_enabled` in `config.rs`; loop wiring + `MetaCache` +
  `record_insights` in `player.rs`; module + spawn + toggle-apply in `lib.rs`.
  TS: `src/lib/insights.ts` (wrappers + `insightsCountLabel`),
  `src/hooks/useInsights.ts`, `src/components/SettingsPanel.tsx`, header gear +
  `GearIcon` in `App.tsx`, `settingsCopy` in `src/lib/copy.ts`.
- M8: edit-gate + absorb in `automation.rs` (`Gates.edit_hold`,
  `note_external_seek`); commands `set_edit_mode`/`ui_seek` in
  `commands.rs`; per-track gate comparison (`editing_track`) + flag
  consumption in `player.rs`; neutral rule in `presets.rs::validate_times`;
  TS pure logic (`previewTargetMs`, `saveActionFor`, `restoreTargetMs`,
  `flagsRepel`, consts) in `src/lib/presetLogic.ts`; edit session in
  `src/hooks/usePreset.ts`; IPC wrappers in `src/lib/player.ts`.
- Rust: `automation.rs` (pure auto-skip decisions + all its consts),
  `presets.rs` (store + validation + errors), `commands.rs` (IPC layer +
  `AppState`), `player.rs` (poll engine + automation execution),
  `tray.rs` (tray menu + window show/hide/quit lifecycle),
  `spotify.rs` (HTTP + serde), `token_store.rs`, `config.rs`, `pkce.rs`,
  `server.rs`, `error.rs`.
- TS: `src/lib/presets.ts` / `src/lib/automation.ts` (IPC wrappers),
  `src/lib/presetLogic.ts` (pure rules), `src/hooks/usePreset.ts` /
  `useAutomation.ts`, `src/components/PresetTimeline.tsx` /
  `NowPlaying.tsx` / `Library.tsx` / `AutomationToggle.tsx`, tabs in
  `App.tsx`.
- M6: `src/lib/wizard.ts` (pure step machine), `errorCopy.ts` (code →
  friendly copy), `clipboard.ts` (copy + confirmation), `copy.ts` (all
  wizard/app strings + REDIRECT_URI), `appInfo.ts` (version),
  `src/components/SetupWizard.tsx`; animations in `src/index.css`
  (`anim-rise-in`, `eq-bounce`).

## Key decisions
- Presets keyed by full track URI (`spotify:track:…`); snapshot fields are
  overwritten on every save (metadata stays fresh whenever the user edits).
- "Newest first" = `created_at DESC` — editing a preset does not reshuffle
  the Library.
- Drag granularity snaps to whole seconds; stored values are exact ms.
- `delete_preset` is idempotent (no error on a missing row).
- Timestamps are assigned in Rust — the frontend never sends them.
- `skip_ms == duration_ms` means "play to the end": the engine never fires
  a skip for it (no artificial next-track 300 ms before the natural end).
- `start_ms == 0` never issues a seek (pointless call).
- M8: the NEUTRAL pair (start 0 AND skip == duration) is rejected by Rust
  validation — "no preset" is expressed by deleting the row, never by
  storing 0→end. Library inline edits to neutral surface the Rust message.
- M8: preview = seek only (constraint: no new endpoints) — if Spotify is
  paused, the preview positions playback but does not press play.
- The action cap counts ATTEMPTS (success or failure) — it is the hard
  never-loop bound; the cooldown after any action survives instance
  changes (a skip's cooldown briefly delays the next track's start jump —
  accepted for the rate bound).
- M9: schema is now v2 (`PRAGMA user_version`). Migration path: fresh DB
  creates presets + insights at v2; an existing v1 DB gets ONLY the insights
  tables added (presets untouched); a future version is still refused, never
  renamed. Insights live in the SAME `cued.db` with the same transactional
  discipline as presets.
- M9: insights retention is PERMANENT and user-controlled by design — no
  caps, no pruning, no expiry anywhere in code. Do not add any "safety" cap
  in future work; the only deletion is the user's explicit delete-all.
- M9: `skip_next` is only classified on a controllable→controllable track
  change. Skipping a regular track straight into an episode/local file (or to
  "nothing playing") clears playback before classification, so that one skip
  is not recorded — an accepted, documented gap (the common case, next→track,
  is covered).
- M10: schema is now v3. Migration path: fresh DB creates presets + insights +
  suggestions at v3; v1 → adds insights AND suggestions; v2 → adds ONLY
  suggestions; a future version is still refused, never renamed. Two new tables
  in the SAME `cued.db`: `suggestions` (PK `(track_uri, type)`) and
  `analysis_state` (kv cursor).
- M10: **auto-skip is an explicit `suggestions` row** (`type='auto_skip'`,
  `status='applied'`) — NOT a fake 0/0 preset (which validation rejects
  anyway). The engine reads it via `is_auto_skip_applied`.
- M10: accepting a skip/start suggestion saves a NORMAL preset (skip-point sets
  `skip_ms`, preserving any existing `start_ms`; start-point sets `start_ms`,
  preserving `skip_ms`/duration). `accept_suggestion` returns the PREVIOUS
  preset so Undo reverts fully (restore it, or delete the one just created).
- M10: `delete_all_insights` now also wipes `suggestions` + `analysis_state`
  (derived data), so an applied auto-skip is cleared by the privacy delete.
  Presets — including one created by accepting a suggestion — are untouched.
- M10: an APPLIED auto-skip keeps working (and stays visible + reversible in
  the Library) even when the master insights toggle or the auto-skip type
  toggle is OFF — it is a committed choice, like a preset. Toggles gate only
  what is SURFACED (the card + active/retired rows), never a committed
  behavior. "Music must never disappear with no way back."
- M11: NO schema change — the heatmap is derived purely from the existing
  `listening_events`, so `SCHEMA_VERSION` stays at 3.
- M11: the heatmap curve and the M10 skip-point suggestion share the 15 s
  rejection rule but are otherwise independent — the curve is a RAW per-event
  density (no play grouping, no recency decay, no clustering), so it can differ
  from the single suggested skip value. That is intended: the curve shows the
  shape of your skipping, the suggestion picks one point from it.
- M11: `get_track_heatmap` returns `null` (not an empty curve) below
  `HEATMAP_MIN_EVENTS`; the waveform then renders all-grey (no red). Bucketing/
  normalization (Rust) + density-sampling/color-mapping/fake-wave (TS) are pure
  + unit-tested (cargo + vitest).
- M11: heatmap is a soft color WASH over a grey fake waveform, not a separate
  curve/band (user direction 2026-07-24): the wave reddens where skips
  concentrate. Iterated: first a per-bar tint, then decoupled into a smooth
  `heatGradient` overlay so accuracy no longer depends on bar thickness (bars
  are fine + grey; the gradient uses all 100 buckets). Amber spline band dropped.
  `heatStopColor` RGB endpoints mirror the `--amber`/`--heat` tokens (JS-computed).
- M11: motion rule — decorative feedback is transform/opacity only; the sole
  layout-animating property is the cap's `left` (90 ms snap-glide), and it is
  disabled WHILE dragging (instant pointer tracking) so the cap never lags the
  cursor — the glide is only for keyboard nudges + the release settle. All
  motion is disabled under `prefers-reduced-motion` (`index.css`).

## M11 status + what a future ticket needs to know
- DONE this ticket: the "Studio Bracket" timeline redesign and the skip-density
  heatmap overlay (see the M11 entries above). Suggestion MARKERS on the strip
  (drawing `suggestions_for_track` regions onto the timeline) were NOT part of
  this ticket and remain a clean follow-up — the redesigned `PresetTimeline`
  already frames a region, so a marker layer would slot in next to the caps.
- Heatmap honesty decisions to preserve: the comb texture is UNIFORM per track
  on purpose (never seed/randomize it — that would imply real audio), and the
  curve excludes < 15 s rejections + `seek_back` rewinds (it answers "where do
  you skip PART of this song", not "which songs do you bail on").
- The reference material below (event storage, suggestion rows) is still
  accurate and is what a marker ticket would read.

- Raw data is in `cued.db`: `listening_events` (id, track_uri, type
  {seek_forward|seek_back|skip_next}, from_ms, to_ms (NULL for skip_next),
  duration_ms, created_at unix ms) indexed on `track_uri`, plus `tracks`
  (uri PK, title, artists JSON, cover_url, duration_ms, last_seen). Read a
  track's events with `PresetStore::events_for_track` (already returns the pure
  `suggestions::Event` shape). For the "honest waveform" density overlay,
  bucket `from_ms` (and optionally seek `to_ms`) over the duration — this is
  exactly the input `suggestions::group_plays` / `analyze` already consume, so
  reuse or parallel it rather than re-reading the DB differently.
- Suggestion markers on the timeline should read the SAME stored rows the card
  uses: `suggestions_for_track(uri)` → `StoredSuggestion` with
  `valueStartMs`/`valueEndMs` (skip-point region), `valueStartMs` (start-point
  target), `status`. Do NOT recompute — the analysis engine is the single
  source of the suggested times.
- The timeline is `PresetTimeline.tsx`; the 24 px band above the strip was
  reserved for the heatmap in the M8 redesign (see the design memory). Keep the
  overlay decorative/uniform texture — it must not imply real audio.
- Schema is v3. Any new column → bump `SCHEMA_VERSION` to 4 with a `3 => …`
  migration arm (the runner is cumulative: 0→all, 1→insights+suggestions,
  2→suggestions, then each arm falls through by re-running open).
- Bump `AppState.suggestions_version` (via `notify_suggestions_changed`)
  whenever an applied auto-skip flag changes, so the poll loop's per-track
  config cache (`lookup_track_config`) stays correct.

## Gotchas
- M14: keep `tray::AUTOSTART_ARG` (`--autostart`) stable — it is baked into
  every existing user's LaunchAgent plist at enable time; renaming it makes
  those login items launch with the window visible again.
- M14: enabling autostart from a DEV build writes the dev binary's path
  (`target/debug/cued`) into the plist — manual login-item testing only makes
  sense with an installed .app build. Remember to toggle it off after testing
  a throwaway build, or delete `~/Library/LaunchAgents/Cued.plist`.
- M14: the window is created hidden and shown by `apply_launch_visibility`
  in setup. Any future second window / splash must account for that, and
  removing `"visible": false` from tauri.conf.json would re-introduce a
  window flash on login launches.
- M11: a `filter: blur()` layer on a fast-MOVING element (the drag caps) leaves
  ghost trails in WebKit. The cap glow therefore uses a `radial-gradient`
  background (no filter), not `blur-md`. Avoid filter blur on anything dragged.
- macOS WebKit does not focus `<button>` on click AND its mousedown default
  steals programmatic focus again. Keyboard-draggable controls need BOTH
  `focus()` in pointerdown and `preventDefault()` in mousedown (see
  `PresetTimeline`), or arrow keys silently do nothing.
- `cargo` lives in `~/.cargo/bin` (not on PATH in plain shells).
- npm install-scripts policy: keep the `allowScripts` entry in package.json.
- Dev-rebuilt binaries may trigger a one-time macOS keychain consent prompt.
  "Always allow" does NOT survive rebuilds: dev binaries are ad-hoc signed,
  so every build has a new code identity. A stable dev signing identity
  (`signingIdentity` in tauri.conf.json with a self-created codesigning
  cert or an Apple Development identity) would make the grant stick.
- Tauri IPC arg names are camelCase on the TS side (`trackUri`), snake_case
  in Rust — the conversion is automatic.
- Rust `()` command results arrive as `null` over IPC (`unitSchema`).
- CSP: any new remote asset host (images, etc.) must be added to BOTH
  `csp` and `devCsp` in tauri.conf.json, or it will silently fail to load.
  React `style={{…}}` attributes only work because style-src keeps
  'unsafe-inline' — do not remove it without replacing all inline styles.
- The tray template PNGs are generated (stdlib-only rasterizer mirroring
  the Logo.tsx SVG geometry): `python3 scripts/gen-tray-icons.py
  src-tauri/icons/tray`. Re-run it if the logo ever changes.

## MVP status & post-MVP backlog
The MVP (M0–M6) is feature-complete: auth wizard, playback view, presets +
Library, auto-skip engine, tray mode, polish + strict CSP. Known post-MVP
items, in no particular order:
- Custom tray popover from the mockup (M5 shipped the native menu only).
- System notifications; global keyboard shortcuts (launch at login shipped
  in M14).
- i18n — all user-facing strings already live in `src/lib/copy.ts` /
  `errorCopy.ts`, so this is extraction, not a rewrite.
- Fade-out skip; preset export/import.
- Stable dev signing identity so the macOS keychain consent survives
  dev rebuilds (see Gotchas).
- Tauri isolation pattern (AGENTS.md security wishlist).

## Facts every future ticket needs
- UI states (`App.tsx`, plain state, no router): `ConnState` phases
  `loading → disconnected → connected`; the old "connecting" phase is gone
  (the wizard owns connect progress locally). Two tabs once connected,
  preset-DB health banner, auth-lost recovery (`handleAuthLost` →
  disconnected + `appCopy.sessionExpired` → wizard opens on step 3).
- Closing the window only hides it — never assume a fresh mount per
  launch; the webview keeps running.
- The in-app automation pill updates via the `automation://enabled` event
  (tray sync) — any redesigned toggle must keep the `useAutomation` wiring.
- Log lines to watch when debugging: `cued: automation decided …` /
  `cued: automation executed …` on stderr of the dev process.

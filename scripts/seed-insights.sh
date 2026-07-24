#!/usr/bin/env bash
#
# Dev-only helper (M10): inject synthetic listening-insights events so a
# suggestion pattern forms instantly, instead of doing 5–10 real plays.
#
# Usage:
#   scripts/seed-insights.sh <skip|start|auto> [spotify:track:REAL_URI]
#
#   skip   → 6 plays skipping at 1:30  → a "skip point" suggestion
#   start  → 6 plays jumping to 0:30   → a "start point" suggestion
#   auto   → 10 plays skipped at 0:05  → a whole-song "auto-skip" suggestion
#
# Omit the URI to seed a demo track (shows up in the Library → Suggestions
# section, no playback needed). Pass a REAL track URI (in Spotify: right-click
# the song → Share → hold Option/Alt → "Copy Spotify URI") to make the Now
# Playing card appear (and auto-skip actually fire) while THAT song plays.
#
# After seeding, bring Cued to the front (or open the Library tab) so the
# opportunistic analysis runs. Quit Cued first if a write ever reports "locked".
set -euo pipefail

KIND="${1:-}"
URI="${2:-spotify:track:cued-demo-${KIND}}"
DB="$HOME/Library/Application Support/app.cued.desktop/cued.db"

case "$KIND" in
  skip|start|auto) ;;
  *) echo "usage: $0 <skip|start|auto> [spotify:track:REAL_URI]" >&2; exit 2 ;;
esac
command -v sqlite3 >/dev/null || { echo "sqlite3 not found on PATH" >&2; exit 1; }

NOW_MS=$(( $(date +%s) * 1000 ))
DAY_MS=$(( 24 * 60 * 60 * 1000 ))
DURATION=200000
TITLE="Cued demo — ${KIND}"

# Per-kind event shape and play count.
case "$KIND" in
  skip)  TYPE="skip_next";    FROM=90000; TO="NULL";  PLAYS=6  ;;
  start) TYPE="seek_forward"; FROM=3000;  TO="30000"; PLAYS=6  ;;
  auto)  TYPE="skip_next";    FROM=5000;  TO="NULL";  PLAYS=10 ;;
esac

SQL_FILE="$(mktemp)"
trap 'rm -f "$SQL_FILE"' EXIT

{
  # Match the app's v3 schema so seeding works even before first launch.
  cat <<'SCHEMA'
CREATE TABLE IF NOT EXISTS tracks (
  uri TEXT PRIMARY KEY, title TEXT NOT NULL, artists TEXT NOT NULL,
  cover_url TEXT, duration_ms INTEGER NOT NULL, last_seen INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS listening_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT, track_uri TEXT NOT NULL, type TEXT NOT NULL,
  from_ms INTEGER NOT NULL, to_ms INTEGER, duration_ms INTEGER NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS idx_listening_events_track_uri ON listening_events(track_uri);
CREATE TABLE IF NOT EXISTS suggestions (
  track_uri TEXT NOT NULL, type TEXT NOT NULL, status TEXT NOT NULL,
  shown_count INTEGER NOT NULL DEFAULT 0, value_start_ms INTEGER, value_end_ms INTEGER,
  plays_total INTEGER NOT NULL DEFAULT 0, plays_matching INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL, PRIMARY KEY (track_uri, type));
CREATE TABLE IF NOT EXISTS analysis_state (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
SCHEMA
  echo "PRAGMA user_version = 3;"
  echo "INSERT INTO tracks (uri, title, artists, cover_url, duration_ms, last_seen)"
  echo "VALUES ('$URI', '$TITLE', '[\"Cued\"]', NULL, $DURATION, $NOW_MS)"
  echo "ON CONFLICT(uri) DO UPDATE SET last_seen = $NOW_MS;"
  # One event per play, spaced a day apart so each is its own session.
  for ((i = 0; i < PLAYS; i++)); do
    AT=$(( NOW_MS - i * DAY_MS ))
    echo "INSERT INTO listening_events (track_uri, type, from_ms, to_ms, duration_ms, created_at)"
    echo "VALUES ('$URI', '$TYPE', $FROM, $TO, $DURATION, $AT);"
  done
} > "$SQL_FILE"

sqlite3 "$DB" < "$SQL_FILE"
echo "Seeded $PLAYS '$KIND' plays for $URI"
echo "→ Bring Cued to the front (or open the Library tab) to run analysis."

import { useEffect, useState } from "react";
import { getInsightsEnabled } from "../lib/insights";
import { appCopy, suggestionsCopy } from "../lib/copy";
import { friendlyAuthMessage } from "../lib/errorCopy";
import { formatTimeMs } from "../lib/playback";
import {
  isValidPreset,
  parseTimeMs,
  presetMatchesQuery,
} from "../lib/presetLogic";
import { deletePreset, listPresets, savePreset, type Preset } from "../lib/presets";
import {
  acceptSuggestion,
  analyzeSuggestions,
  dismissSuggestion,
  getSuggestionToggles,
  librarySuggestions,
  listSuggestions,
  setAutoSkipApplied,
  type Suggestion,
  type SuggestionToggles,
  type SuggestionType,
} from "../lib/suggestions";

/**
 * The Library tab: the collapsible "Suggestions (n)" section on top (M10),
 * then all stored presets.
 */
export function Library() {
  return (
    <div className="flex w-full flex-col gap-4">
      <SuggestionsSection />
      <PresetLibrary />
    </div>
  );
}

/**
 * All stored presets, newest first: cover, title, artists, start/skip chips,
 * case-insensitive search, inline edit (m:ss fields) and inline
 * delete-confirm. Purely local — no Spotify API involved.
 */
function PresetLibrary() {
  const [presets, setPresets] = useState<Preset[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  useEffect(() => {
    let cancelled = false;
    listPresets()
      .then((list) => {
        if (!cancelled) setPresets(list);
      })
      .catch((err: unknown) => {
        if (!cancelled) setLoadError(friendlyAuthMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const replacePreset = (updated: Preset) => {
    setPresets(
      (list) =>
        list?.map((p) => (p.trackUri === updated.trackUri ? updated : p)) ??
        null,
    );
  };

  const removePreset = (trackUri: string) => {
    setPresets((list) => list?.filter((p) => p.trackUri !== trackUri) ?? null);
  };

  if (loadError) {
    return (
      <p role="alert" className="text-center text-sm text-amber">
        {loadError}
      </p>
    );
  }
  if (presets === null) {
    return <p className="text-center text-sm text-text-mut">Loading presets…</p>;
  }
  if (presets.length === 0) {
    return (
      <div className="flex flex-col items-center gap-2 rounded-xl border border-hairline bg-surface px-6 py-10 text-center">
        <span aria-hidden className="text-2xl text-text-mut">
          ♪
        </span>
        <p className="text-sm font-medium text-text">
          {appCopy.libraryEmptyTitle}
        </p>
        <p className="max-w-xs text-sm leading-relaxed text-text-mut">
          {appCopy.libraryEmptyHint}
        </p>
      </div>
    );
  }

  const shown = presets.filter((p) =>
    presetMatchesQuery(p.title, p.artists, query),
  );

  return (
    <div className="flex w-full flex-col gap-3">
      <input
        type="search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search by title or artist"
        spellCheck={false}
        className="w-full rounded-lg border border-hairline bg-surface px-4 py-2 text-sm text-text placeholder:text-text-mut focus:border-accent focus:outline-none"
      />
      {shown.length === 0 ? (
        <p className="py-6 text-center text-sm text-text-mut">
          No presets match “{query.trim()}”.
        </p>
      ) : (
        <ul className="flex flex-col gap-2">
          {shown.map((preset) => (
            <LibraryRow
              key={preset.trackUri}
              preset={preset}
              onSaved={replacePreset}
              onDeleted={removePreset}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

type RowMode = "view" | "edit" | "confirm-delete";

function LibraryRow({
  preset,
  onSaved,
  onDeleted,
}: {
  preset: Preset;
  onSaved: (updated: Preset) => void;
  onDeleted: (trackUri: string) => void;
}) {
  const [mode, setMode] = useState<RowMode>("view");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const enterView = () => {
    setMode("view");
    setError(null);
  };

  const handleDelete = () => {
    setBusy(true);
    setError(null);
    deletePreset(preset.trackUri)
      .then(() => {
        onDeleted(preset.trackUri);
      })
      .catch((err: unknown) => {
        setBusy(false);
        setError(friendlyAuthMessage(err));
      });
  };

  return (
    <li className="rounded-lg border border-hairline bg-surface p-3">
      <div className="flex items-center gap-3">
        {preset.coverUrl ? (
          <img
            src={preset.coverUrl}
            alt=""
            className="h-10 w-10 shrink-0 rounded object-cover"
          />
        ) : (
          <div
            aria-hidden
            className="flex h-10 w-10 shrink-0 items-center justify-center rounded bg-surface-2 text-sm text-text-mut"
          >
            ♪
          </div>
        )}
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium text-text">
            {preset.title}
          </p>
          <p className="truncate text-xs text-text-mut">
            {preset.artists.join(", ")}
          </p>
        </div>
        <span className="rounded-full bg-accent/15 px-2 py-0.5 text-[11px] font-semibold tabular-nums text-accent-hi">
          Start {formatTimeMs(preset.startMs)}
        </span>
        <span className="rounded-full bg-amber/15 px-2 py-0.5 text-[11px] font-semibold tabular-nums text-amber">
          Skip {formatTimeMs(preset.skipMs)}
        </span>
        {mode === "view" && (
          <span className="flex gap-2">
            <RowAction label="Edit" onClick={() => setMode("edit")} />
            <RowAction label="Delete" onClick={() => setMode("confirm-delete")} />
          </span>
        )}
      </div>

      {mode === "confirm-delete" && (
        <div className="mt-2 flex items-center justify-end gap-3 text-xs">
          <span className="text-text-mut">Delete this preset?</span>
          <button
            type="button"
            onClick={handleDelete}
            disabled={busy}
            className="rounded-full bg-amber px-3 py-1 font-semibold text-ground transition-colors hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber disabled:opacity-50"
          >
            {busy ? "Deleting…" : "Delete"}
          </button>
          <RowAction label="Cancel" onClick={enterView} disabled={busy} />
        </div>
      )}

      {mode === "edit" && (
        <RowEditor
          preset={preset}
          busy={busy}
          setBusy={setBusy}
          setError={setError}
          onSaved={(updated) => {
            onSaved(updated);
            enterView();
          }}
          onCancel={enterView}
        />
      )}

      {error && (
        <p role="alert" className="mt-2 text-right text-xs text-amber">
          {error}
        </p>
      )}
    </li>
  );
}

function RowAction({
  label,
  onClick,
  disabled = false,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="text-xs text-text-mut underline underline-offset-2 transition-colors hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:opacity-50"
    >
      {label}
    </button>
  );
}

/** Inline m:ss editor with the same validation rules as the timeline. */
function RowEditor({
  preset,
  busy,
  setBusy,
  setError,
  onSaved,
  onCancel,
}: {
  preset: Preset;
  busy: boolean;
  setBusy: (b: boolean) => void;
  setError: (e: string | null) => void;
  onSaved: (updated: Preset) => void;
  onCancel: () => void;
}) {
  const [startText, setStartText] = useState(formatTimeMs(preset.startMs));
  const [skipText, setSkipText] = useState(formatTimeMs(preset.skipMs));

  const handleSave = () => {
    const startMs = parseTimeMs(startText);
    const skipMs = parseTimeMs(skipText);
    if (startMs === null || skipMs === null) {
      setError("Times must look like 3:05 (minutes:seconds).");
      return;
    }
    if (!isValidPreset(startMs, skipMs, preset.durationMs)) {
      setError(
        `Start must come first, at least 10 s before skip, within the track (${formatTimeMs(preset.durationMs)}).`,
      );
      return;
    }
    setBusy(true);
    setError(null);
    savePreset({
      trackUri: preset.trackUri,
      title: preset.title,
      artists: preset.artists,
      coverUrl: preset.coverUrl,
      durationMs: preset.durationMs,
      startMs,
      skipMs,
    })
      .then((updated) => {
        setBusy(false);
        onSaved(updated);
      })
      .catch((err: unknown) => {
        setBusy(false);
        setError(friendlyAuthMessage(err));
      });
  };

  const field = (
    label: string,
    value: string,
    onChange: (v: string) => void,
  ) => (
    <label className="flex items-center gap-1.5 text-xs text-text-mut">
      {label}
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={busy}
        size={5}
        spellCheck={false}
        className="w-14 rounded border border-hairline bg-surface-2 px-2 py-1 text-center text-xs tabular-nums text-text focus:border-accent focus:outline-none"
      />
    </label>
  );

  return (
    <div className="mt-2 flex items-center justify-end gap-3">
      {field("Start", startText, setStartText)}
      {field("Skip", skipText, setSkipText)}
      <button
        type="button"
        onClick={handleSave}
        disabled={busy}
        className="rounded-full bg-accent px-3 py-1 text-xs font-semibold text-ground transition-colors hover:bg-accent-hi focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:opacity-50"
      >
        {busy ? "Saving…" : "Save"}
      </button>
      <RowAction label="Cancel" onClick={onCancel} disabled={busy} />
    </div>
  );
}

const TOGGLES_ON: SuggestionToggles = {
  skipPoints: true,
  startPoints: true,
  autoSkip: true,
};

/**
 * Collapsible "Suggestions (n)" section (M10): a muted header + count chip
 * over normal rows, each with a small ✦ glyph — no tinted rows. Active
 * suggestions can be accepted or dismissed inline; an applied auto-skip shows
 * a subtle badge and can be turned off. Empty → the whole section is hidden.
 */
function SuggestionsSection() {
  const [items, setItems] = useState<Suggestion[] | null>(null);
  const [toggles, setToggles] = useState<SuggestionToggles>(TOGGLES_ON);
  const [insightsOn, setInsightsOn] = useState(true);
  const [open, setOpen] = useState(true);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      // Opening the Library is one of the moments we refresh analysis.
      await analyzeSuggestions().catch(() => undefined);
      const [list, t, on] = await Promise.allSettled([
        listSuggestions(),
        getSuggestionToggles(),
        getInsightsEnabled(),
      ]);
      if (cancelled) return;
      setItems(list.status === "fulfilled" ? list.value : []);
      if (t.status === "fulfilled") setToggles(t.value);
      if (on.status === "fulfilled") setInsightsOn(on.value);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  if (items === null) {
    return null; // the section is secondary — stay quiet until loaded
  }
  const shown = librarySuggestions(items, toggles, insightsOn);
  if (shown.length === 0) {
    return null; // empty section is hidden entirely
  }

  const remove = (trackUri: string, type: SuggestionType) => {
    setItems(
      (prev) =>
        prev?.filter((s) => !(s.trackUri === trackUri && s.type === type)) ??
        null,
    );
  };
  const setStatus = (
    trackUri: string,
    type: SuggestionType,
    status: Suggestion["status"],
  ) => {
    setItems(
      (prev) =>
        prev?.map((s) =>
          s.trackUri === trackUri && s.type === type ? { ...s, status } : s,
        ) ?? null,
    );
  };

  return (
    <section className="overflow-hidden rounded-xl border border-hairline bg-surface">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-2 px-4 py-2.5 text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi"
      >
        <span className="text-xs font-semibold uppercase tracking-[0.15em] text-text-mut">
          {suggestionsCopy.librarySectionTitle}
        </span>
        <span className="rounded-full bg-surface-2 px-1.5 py-0.5 text-[11px] font-semibold tabular-nums text-text-mut">
          {shown.length}
        </span>
        <span
          aria-hidden
          className={`ml-auto text-text-mut transition-transform ${open ? "rotate-180" : ""}`}
        >
          <svg viewBox="0 0 16 16" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
            <path d="M4 6l4 4 4-4" />
          </svg>
        </span>
      </button>
      {open && (
        <ul className="flex flex-col border-t border-hairline">
          {shown.map((s) => (
            <SuggestionRow
              key={s.trackUri + s.type}
              suggestion={s}
              onRemove={remove}
              onSetStatus={setStatus}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function SuggestionRow({
  suggestion,
  onRemove,
  onSetStatus,
}: {
  suggestion: Suggestion;
  onRemove: (trackUri: string, type: SuggestionType) => void;
  onSetStatus: (
    trackUri: string,
    type: SuggestionType,
    status: Suggestion["status"],
  ) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const s = suggestion;
  const appliedAutoSkip = s.type === "auto_skip" && s.status === "applied";

  const run = (op: () => Promise<unknown>, after: () => void) => {
    setBusy(true);
    setError(null);
    op()
      .then(() => after())
      .catch((err: unknown) => {
        setError(friendlyAuthMessage(err));
      })
      .finally(() => setBusy(false));
  };

  const accept = () =>
    run(
      () => acceptSuggestion(s.trackUri, s.type),
      () => {
        // Skip/start became presets (dropped from this section); an accepted
        // auto-skip stays as the applied badge row.
        if (s.type === "auto_skip") onSetStatus(s.trackUri, s.type, "applied");
        else onRemove(s.trackUri, s.type);
      },
    );
  const dismiss = () =>
    run(
      () => dismissSuggestion(s.trackUri, s.type),
      () => onRemove(s.trackUri, s.type),
    );
  const turnOff = () =>
    run(
      () => setAutoSkipApplied(s.trackUri, false),
      () => onSetStatus(s.trackUri, s.type, "active"),
    );

  return (
    <li className="flex items-center gap-3 border-b border-hairline px-4 py-3 last:border-b-0">
      {s.coverUrl ? (
        <img src={s.coverUrl} alt="" className="h-10 w-10 shrink-0 rounded object-cover" />
      ) : (
        <div aria-hidden className="flex h-10 w-10 shrink-0 items-center justify-center rounded bg-surface-2 text-sm text-text-mut">
          ♪
        </div>
      )}
      <div className="min-w-0 flex-1">
        <p className="flex items-center gap-1.5 truncate text-sm font-medium text-text">
          <span aria-hidden className="text-accent/65">
            ✦
          </span>
          {s.title}
        </p>
        <p className="truncate text-xs text-text-mut">{evidenceLine(s)}</p>
      </div>
      {appliedAutoSkip ? (
        <>
          <span className="rounded-full bg-surface-2 px-2 py-0.5 text-[11px] font-medium text-text-mut">
            {suggestionsCopy.autoSkippedBadge}
          </span>
          <RowAction label={suggestionsCopy.turnOff} onClick={turnOff} disabled={busy} />
        </>
      ) : (
        <>
          <button
            type="button"
            onClick={accept}
            disabled={busy}
            className="rounded-full bg-accent px-3 py-1 text-xs font-semibold text-ground transition-colors hover:bg-accent-hi focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:opacity-50"
          >
            {acceptLabel(s.type)}
          </button>
          <RowAction label={suggestionsCopy.dismiss} onClick={dismiss} disabled={busy} />
        </>
      )}
      {error !== null && (
        <p role="alert" className="ml-2 text-xs text-amber">
          {error}
        </p>
      )}
    </li>
  );
}

function acceptLabel(type: SuggestionType): string {
  switch (type) {
    case "skip_point":
      return suggestionsCopy.setSkipPoint;
    case "start_point":
      return suggestionsCopy.setStartPoint;
    case "auto_skip":
      return suggestionsCopy.autoSkipIt;
  }
}

function evidenceLine(s: Suggestion): string {
  const evidence = `${s.playsMatching}/${s.playsTotal} recent plays`;
  switch (s.type) {
    case "skip_point":
      return s.valueStartMs !== null && s.valueEndMs !== null
        ? `Skip ${formatTimeMs(s.valueStartMs)}–${formatTimeMs(s.valueEndMs)} · ${evidence}`
        : evidence;
    case "start_point":
      return s.valueStartMs !== null
        ? `Start ${formatTimeMs(s.valueStartMs)} · ${evidence}`
        : evidence;
    case "auto_skip":
      return `Skipped early · ${evidence}`;
  }
}

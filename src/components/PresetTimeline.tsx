import { useRef, useState } from "react";
import { formatTimeMs } from "../lib/playback";
import { fakeWaveHeights, heatGradient } from "../lib/heatmap";
import {
  clampSkipMs,
  clampStartMs,
  HANDLE_STEP_LARGE_MS,
  HANDLE_STEP_MS,
  snapMsToSecond,
  timelineMsFromPointer,
} from "../lib/presetLogic";

export type HandleKind = "start" | "skip";

interface SharedProps {
  durationMs: number;
  /** Live playhead position; null hides the playhead. */
  positionMs: number | null;
  /**
   * Normalized skip-density curve (100 buckets in [0,1]) for the "Most
   * Replayed"-style overlay, or null/undefined to collapse the reserved band.
   */
  heatmap?: number[] | null;
}

interface ViewProps extends SharedProps {
  /** Calm framed strip; the active region shows only when a preset exists. */
  mode: "view";
  startMs: number | null;
  skipMs: number | null;
}

interface EditProps extends SharedProps {
  /** Full editor: glyph end-caps, keyboard interaction, micro-feedback. */
  mode: "edit";
  startMs: number;
  skipMs: number;
  onChangeStart: (ms: number) => void;
  onChangeSkip: (ms: number) => void;
  /** A handle received focus — it becomes the preview target. */
  onFocusHandle: (which: HandleKind) => void;
}

type PresetTimelineProps = ViewProps | EditProps;

/** The active region, or null when there is no preset to frame. */
type Region = { startMs: number; skipMs: number };

/** Bars in the fake waveform (fine rounded pills). */
const BAR_COUNT = 72;
/** Bar height as a % of the strip at wave value 1, and the quiet-bar floor. */
const BAR_MAX_H = 82;
const BAR_MIN_H = 16;
/** The fixed, per-track-identical waveform envelope (computed once). */
const WAVE = fakeWaveHeights(BAR_COUNT);

/**
 * The Now Playing timeline — "Studio Bracket". A framed 48 px strip filled with
 * a FAKE waveform (a fixed shape, identical for every track, so it never
 * implies real audio) whose bars tint from grey toward dark red where the
 * listener skips most — the skip-density heatmap lives IN the waveform color,
 * not a separate band. The active region is bracketed by full-height glyph
 * end-caps and thin frame rails; a near-white playhead and calm fixed readouts
 * complete it. VIEW is calm (dimmed trim zones + thin frame, no caps); EDIT adds
 * the draggable caps (arrow keys ±1 s, shift ±5 s), preview target focus and
 * the release pulse. Clamping guarantees the handle pair can never be invalid.
 */
export function PresetTimeline(props: PresetTimelineProps) {
  const pct = (ms: number) =>
    props.durationMs > 0 ? Math.min(100, Math.max(0, (ms / props.durationMs) * 100)) : 0;

  if (props.mode === "view") {
    return <ViewBar {...props} pct={pct} />;
  }
  return <EditBar {...props} pct={pct} />;
}

type PctFn = (ms: number) => number;

// ---------------------------------------------------------------------------
// Shared strip skeleton (comb + zones + frame + playhead + heatmap + readouts)
// ---------------------------------------------------------------------------

function Strip({
  durationMs,
  positionMs,
  heatmap,
  region,
  pct,
  frameStrong,
  grabbing,
  pulsing,
  onPulseEnd,
  caps,
}: {
  durationMs: number;
  positionMs: number | null;
  heatmap?: number[] | null;
  region: Region | null;
  pct: PctFn;
  frameStrong: boolean;
  grabbing: HandleKind | null;
  pulsing: boolean;
  onPulseEnd: () => void;
  caps?: React.ReactNode;
}) {
  const startPct = region ? pct(region.startMs) : 0;
  const skipPct = region ? pct(region.skipMs) : 0;
  const regionPct = Math.max(0, skipPct - startPct);
  const frameClass = frameStrong ? "bg-accent-hi/70" : "bg-accent-hi/40";
  // The quiet hint appears only in VIEW (no caps) when there is real density.
  const hasHeat = !caps && !!heatmap && heatmap.some((v) => v > 0);

  return (
    <div>
      {/* Outer box is NOT clipped, so caps/playhead-dot can overflow slightly. */}
      <div className="group/strip relative h-12">
        {/* Clipped visuals: strip fill + grey fake waveform + soft heat wash. */}
        <div className="absolute inset-0 overflow-hidden rounded-[9px] bg-strip">
          <WaveTexture
            startPct={startPct}
            skipPct={skipPct}
            hasRegion={region !== null}
          />
          {/* The heatmap: a smooth gradient over the bars (decoupled from bar
              thickness, full 100-bucket resolution), red where skips cluster. */}
          <div
            aria-hidden
            className="pointer-events-none absolute inset-0"
            style={{ background: heatGradient(heatmap) }}
          />
        </div>

        {region && (
          <>
            {/* Full-region frame: 1.5 px top/bottom rails. */}
            <div
              aria-hidden
              className={`pointer-events-none absolute top-0 h-[1.5px] ${frameClass}`}
              style={{ left: `${startPct}%`, width: `${regionPct}%` }}
            />
            <div
              aria-hidden
              className={`pointer-events-none absolute bottom-0 h-[1.5px] ${frameClass}`}
              style={{ left: `${startPct}%`, width: `${regionPct}%` }}
            />
            {/* Grab-brighten + release-pulse overlay (opacity only). */}
            <div
              aria-hidden
              onAnimationEnd={onPulseEnd}
              className={`pointer-events-none absolute inset-y-0 bg-accent transition-opacity duration-150 motion-reduce:transition-none ${
                grabbing !== null ? "opacity-[0.12]" : "opacity-0"
              } ${pulsing ? "anim-zone-pulse" : ""}`}
              style={{ left: `${startPct}%`, width: `${regionPct}%` }}
            />
          </>
        )}

        {caps}

        {positionMs !== null && (
          <div
            aria-hidden
            className="pointer-events-none absolute top-0 z-30 h-full w-[1.5px] -translate-x-1/2 rounded-full bg-text/90"
            style={{ left: `${pct(positionMs)}%` }}
          >
            <span className="absolute -top-[3px] left-1/2 h-2 w-2 -translate-x-1/2 rounded-full bg-text shadow-[0_0_6px_var(--text)]" />
          </div>
        )}

        {hasHeat && (
          <span className="pointer-events-none absolute -top-6 left-1/2 -translate-x-1/2 whitespace-nowrap rounded bg-surface-2 px-1.5 py-0.5 text-[10px] font-medium text-text-mut opacity-0 transition-opacity duration-150 group-hover/strip:opacity-100 motion-reduce:transition-none">
            You usually skip this part
          </span>
        )}
      </div>

      <Readouts
        region={region}
        positionMs={positionMs}
        durationMs={durationMs}
      />
    </div>
  );
}

/**
 * The FAKE waveform — fine grey rounded pill bars in a flex row (real px
 * geometry, so the pills stay crisp; no non-uniform SVG stretching). Bar
 * heights come from a FIXED, symmetric envelope ([`WAVE`], identical for every
 * track — it reads as decorative and encodes no audio, so it never masquerades
 * as a real waveform). The bars carry NO data: the skip-density heatmap is a
 * separate soft gradient laid over them (see `Strip`). Trim zones (outside the
 * active region) are dimmed so the framed region still reads.
 */
function WaveTexture({
  startPct,
  skipPct,
  hasRegion,
}: {
  startPct: number;
  skipPct: number;
  hasRegion: boolean;
}) {
  return (
    <div aria-hidden className="flex h-full w-full items-center gap-[2px] px-2">
      {WAVE.map((h, i) => {
        const posPct = ((i + 0.5) / BAR_COUNT) * 100;
        const inRegion = !hasRegion || (posPct >= startPct && posPct <= skipPct);
        return (
          <span
            key={i}
            className="flex-1 rounded-full bg-text-mut"
            style={{
              height: `${Math.max(BAR_MIN_H, h * BAR_MAX_H)}%`,
              opacity: inRegion ? 0.7 : 0.3,
            }}
          />
        );
      })}
    </div>
  );
}

/** Fixed readouts below the strip: Start (green) / Skip (amber) / duration. */
function Readouts({
  region,
  positionMs,
  durationMs,
}: {
  region: Region | null;
  positionMs: number | null;
  durationMs: number;
}) {
  return (
    <div className="mt-2 flex items-center justify-between text-[11px] font-medium tabular-nums">
      <div className="flex items-center gap-3">
        {region ? (
          <>
            <span className="text-accent-hi">
              Start {formatTimeMs(region.startMs)}
            </span>
            <span className="text-amber">Skip {formatTimeMs(region.skipMs)}</span>
          </>
        ) : (
          <span className="text-text-mut">
            {positionMs !== null ? formatTimeMs(positionMs) : "–:––"}
          </span>
        )}
      </div>
      <span className="text-text-mut">{formatTimeMs(durationMs)}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// VIEW — calm, no caps
// ---------------------------------------------------------------------------

function ViewBar({
  positionMs,
  durationMs,
  startMs,
  skipMs,
  heatmap,
  pct,
}: ViewProps & { pct: PctFn }) {
  // A stored preset always has both points; neutral presets are never stored,
  // so the presence of values IS the presence of a region to frame.
  const region: Region | null =
    startMs !== null && skipMs !== null ? { startMs, skipMs } : null;
  return (
    <Strip
      durationMs={durationMs}
      positionMs={positionMs}
      heatmap={heatmap}
      region={region}
      pct={pct}
      frameStrong={false}
      grabbing={null}
      pulsing={false}
      onPulseEnd={() => undefined}
    />
  );
}

// ---------------------------------------------------------------------------
// EDIT — glyph end-caps, drag/keyboard, micro-feedback
// ---------------------------------------------------------------------------

function EditBar({
  durationMs,
  positionMs,
  startMs,
  skipMs,
  heatmap,
  onChangeStart,
  onChangeSkip,
  onFocusHandle,
  pct,
}: EditProps & { pct: PctFn }) {
  const barRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef<HandleKind | null>(null);
  const [grabbing, setGrabbing] = useState<HandleKind | null>(null);
  const [pulsing, setPulsing] = useState(false);

  const dispatch = (which: HandleKind, desiredMs: number) => {
    if (which === "start") {
      onChangeStart(clampStartMs(desiredMs, skipMs));
    } else {
      onChangeSkip(clampSkipMs(desiredMs, startMs, durationMs));
    }
  };

  const dragTo = (which: HandleKind, clientX: number) => {
    const rect = barRef.current?.getBoundingClientRect();
    if (!rect) return;
    dispatch(
      which,
      snapMsToSecond(
        timelineMsFromPointer(clientX, rect.left, rect.width, durationMs),
      ),
    );
  };

  const handlePointerDown =
    (which: HandleKind) => (e: React.PointerEvent<HTMLButtonElement>) => {
      // macOS WebKit does not focus buttons on click — without this the handle
      // never receives keyboard events (arrow keys).
      e.currentTarget.focus();
      e.currentTarget.setPointerCapture(e.pointerId);
      draggingRef.current = which;
      setGrabbing(which);
      dragTo(which, e.clientX);
    };

  // WebKit's default mousedown moves focus away from buttons right after our
  // pointerdown focus() — suppress it so the handle stays focused (and stays
  // the preview target). Dragging is unaffected (it runs on pointer events).
  const keepFocus = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
  };

  const handlePointerMove = (e: React.PointerEvent<HTMLButtonElement>) => {
    if (draggingRef.current !== null) {
      dragTo(draggingRef.current, e.clientX);
    }
  };

  const endDrag = () => {
    if (draggingRef.current !== null) {
      draggingRef.current = null;
      setGrabbing(null);
      setPulsing(true); // one-shot release pulse (cleared on animationend)
    }
  };

  const handleKeyDown =
    (which: HandleKind) => (e: React.KeyboardEvent<HTMLButtonElement>) => {
      let direction = 0;
      if (e.key === "ArrowLeft" || e.key === "ArrowDown") direction = -1;
      else if (e.key === "ArrowRight" || e.key === "ArrowUp") direction = 1;
      else return;
      e.preventDefault();
      const step = e.shiftKey ? HANDLE_STEP_LARGE_MS : HANDLE_STEP_MS;
      const current = which === "start" ? startMs : skipMs;
      dispatch(which, current + direction * step);
    };

  const cap = (which: HandleKind, valueMs: number) => {
    const isStart = which === "start";
    return (
      <button
        type="button"
        role="slider"
        aria-label={isStart ? "Start point" : "Skip point"}
        aria-valuemin={0}
        aria-valuemax={durationMs}
        aria-valuenow={valueMs}
        aria-valuetext={formatTimeMs(valueMs)}
        onPointerDown={handlePointerDown(which)}
        onPointerMove={handlePointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onMouseDown={keepFocus}
        onKeyDown={handleKeyDown(which)}
        onFocus={() => onFocusHandle(which)}
        className="tl-cap group absolute top-0 z-20 h-full w-[13px] -translate-x-1/2 cursor-ew-resize touch-none focus:outline-none"
        // Instant tracking while dragging; the 90 ms glide (`.tl-cap`) applies
        // only to keyboard nudges and the release settle, never to the pointer.
        style={{
          left: `${pct(valueMs)}%`,
          transition: grabbing === which ? "none" : undefined,
        }}
      >
        {/* Soft glow via a radial gradient (NOT filter:blur — a blurred layer
            leaves ghost trails when the cap moves fast in WebKit). currentColor
            comes from the text-* class; only opacity animates. */}
        <span
          aria-hidden
          className={`tl-glow pointer-events-none absolute -inset-2 rounded-full opacity-0 group-hover:opacity-50 group-active:opacity-80 ${
            isStart ? "text-accent" : "text-amber"
          }`}
          style={{
            background: "radial-gradient(closest-side, currentColor, transparent)",
          }}
        />
        {/* The knob: full-height cap, rounded OUTER corners, colorblind-safe glyph. */}
        <span
          className={`tl-cap-knob relative flex h-full w-full items-center justify-center text-[11px] font-bold leading-none text-ground group-hover:scale-[1.15] group-focus:scale-[1.1] group-active:scale-[1.2] group-focus:ring-2 group-focus:ring-text/60 ${
            isStart
              ? "rounded-l-[7px] bg-accent-hi"
              : "rounded-r-[7px] bg-amber"
          }`}
        >
          {isStart ? "▶" : "»"}
        </span>
        {/* Mono time tooltip above the cap — hover or drag only. */}
        <span
          className={`pointer-events-none absolute -top-6 left-1/2 -translate-x-1/2 whitespace-nowrap rounded bg-surface-2 px-1.5 py-0.5 text-[10px] font-medium tabular-nums text-text opacity-0 transition-opacity duration-150 group-hover:opacity-100 motion-reduce:transition-none ${
            grabbing === which ? "opacity-100" : ""
          }`}
        >
          {formatTimeMs(valueMs)}
        </span>
      </button>
    );
  };

  return (
    <Strip
      durationMs={durationMs}
      positionMs={positionMs}
      heatmap={heatmap}
      region={{ startMs, skipMs }}
      pct={pct}
      frameStrong
      grabbing={grabbing}
      pulsing={pulsing}
      onPulseEnd={() => setPulsing(false)}
      caps={
        <div ref={barRef} className="absolute inset-0">
          {cap("start", startMs)}
          {cap("skip", skipMs)}
        </div>
      }
    />
  );
}

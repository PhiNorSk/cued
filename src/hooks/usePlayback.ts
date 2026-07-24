import { useEffect, useRef, useState } from "react";
import { onPlaybackState, playerWake } from "../lib/player";
import {
  resolveNowPlayingView,
  UI_TICK_MS,
  type NowPlayingView,
  type PlaybackState,
} from "../lib/playback";

interface UsePlaybackResult {
  /** What the Now Playing card should render right now. */
  view: NowPlayingView;
  /** Wall-clock ms of the last UI tick — interpolation input. */
  nowMs: number;
}

/**
 * Subscribe to the Rust playback engine: consume `playback://state` events,
 * send wake signals on mount and window focus, and re-render a few times per
 * second so the progress bar advances between events. `onAuthLost` fires
 * once when the engine reports that re-authentication is required.
 */
export function usePlayback(onAuthLost: () => void): UsePlaybackResult {
  const [current, setCurrent] = useState<PlaybackState | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const lastWithTrackRef = useRef<PlaybackState | null>(null);
  const idleSinceRef = useRef<number | null>(null);
  const onAuthLostRef = useRef(onAuthLost);
  onAuthLostRef.current = onAuthLost;

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    onPlaybackState((state) => {
      if (state.status === "authLost") {
        onAuthLostRef.current();
        return;
      }
      if (state.track !== null) {
        lastWithTrackRef.current = state;
        idleSinceRef.current = null;
      } else {
        idleSinceRef.current ??= Date.now();
      }
      setCurrent(state);
    })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err: unknown) => {
        console.warn("cued: could not subscribe to playback events", err);
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const wake = () => {
      playerWake().catch((err: unknown) => {
        console.warn("cued: wake signal failed", err);
      });
    };
    wake();
    window.addEventListener("focus", wake);
    return () => {
      window.removeEventListener("focus", wake);
    };
  }, []);

  // Local render tick only — all Spotify polling happens in Rust. A fixed
  // interval is fine here because each tick is pure synchronous state.
  useEffect(() => {
    const timer = setInterval(() => {
      setNowMs(Date.now());
    }, UI_TICK_MS);
    return () => {
      clearInterval(timer);
    };
  }, []);

  return {
    view: resolveNowPlayingView(
      current,
      lastWithTrackRef.current,
      idleSinceRef.current,
      nowMs,
    ),
    nowMs,
  };
}

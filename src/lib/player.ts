/** The native player (src-tauri/src/player.rs). */
import { invoke } from "@tauri-apps/api/core";

export interface PlayerState {
  playing: boolean;
  position_ms: number;
  duration_ms: number;
  /** A cleaned sound is loaded. */
  has_cleaned: boolean;
  /** Listening to the cleaned sound (B) rather than the original (A). */
  listen_cleaned: boolean;
  strength: number;
  /** The audio output failed; playback stopped. */
  error: string | null;
}

export const playerOpen = (path: string) => invoke<PlayerState>("player_open", { path });
export const playerClose = () => invoke<void>("player_close");
export const playerPlay = () => invoke<PlayerState>("player_play");
export const playerPause = () => invoke<PlayerState>("player_pause");
export const playerSeek = (ms: number) => invoke<PlayerState>("player_seek", { ms });
export const playerState = () => invoke<PlayerState>("player_state");
export const playerLoadCleaned = (path: string) => invoke<PlayerState>("player_load_cleaned", { path });
export const playerListen = (cleaned: boolean) => invoke<PlayerState>("player_listen", { cleaned });
export const playerStrength = (strength: number) => invoke<PlayerState>("player_strength", { strength });

/**
 * The playhead between two polls: the last reported position plus the time since it was
 * received while playing, never past the end.
 */
export function estimate(state: PlayerState, receivedAt: number, now: number): number {
  if (!state.playing) return state.position_ms;
  return Math.min(state.duration_ms, state.position_ms + Math.max(0, now - receivedAt));
}

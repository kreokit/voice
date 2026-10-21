/** Files Voice opens: audio or video with sound. */

export const AUDIO_EXTENSIONS = ["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "aif", "aiff", "caf", "wma"];
export const VIDEO_EXTENSIONS = ["mp4", "mov", "m4v", "mkv", "webm", "avi"];
export const MEDIA_EXTENSIONS = [...AUDIO_EXTENSIONS, ...VIDEO_EXTENSIONS];

const extension = (path: string) => path.split(".").pop()?.toLowerCase() ?? "";

/** By extension only; ffprobe decides what really has sound. */
export const isMediaPath = (path: string) => MEDIA_EXTENSIONS.includes(extension(path));

/** The last part of a path, with either separator. */
export const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** "0:04", "12:05", "1:02:03". */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** Playhead time with tenths: "0:04.2", "1:02:03.5". */
export function formatTime(ms: number): string {
  const tenths = Math.max(0, Math.floor(ms / 100));
  return `${formatDuration(Math.floor(tenths / 10) * 1000)}.${tenths % 10}`;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** "just now", "5 min ago", "3 h ago", "yesterday", "Monday", "12 Oct", "12 Oct 2025" (from KreoKit Cutout). */
export function relativeTime(ms: number, now: number): string {
  const diff = now - ms;
  if (diff < MINUTE) return "just now";
  if (diff < HOUR) return `${Math.floor(diff / MINUTE)} min ago`;
  const then = new Date(ms);
  const today = new Date(now);
  const startOfToday = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
  if (ms >= startOfToday) return `${Math.floor(diff / HOUR)} h ago`;
  if (ms >= startOfToday - DAY) return "yesterday";
  if (ms >= startOfToday - 6 * DAY) return then.toLocaleDateString("en-GB", { weekday: "long" });
  const sameYear = then.getFullYear() === today.getFullYear();
  return then.toLocaleDateString("en-GB", { day: "numeric", month: "short", ...(sameYear ? {} : { year: "numeric" }) });
}

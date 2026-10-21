import { Clapperboard } from "lucide-react";
import { fileName, formatDuration } from "../lib/files";
import { describeAudio } from "../lib/media";
import { OpenFile, useSession } from "../store/session";
import { TopBar } from "./TopBar";
import "../styles/editor.css";

/** The open file: its sound and, for videos, a note that the picture stays untouched. */
export function Editor({ file }: { file: OpenFile }) {
  const close = useSession((s) => s.close);
  const { info } = file;
  return (
    <div className="screen editor">
      <TopBar title={<span className="editor-title">{fileName(file.path)}</span>} onHome={close} />
      <main className="editor-main">
        <section className="editor-facts" aria-label="About this file">
          <span>{formatDuration(info.duration_ms)}</span>
          <span>{describeAudio(info.audio)}</span>
          {info.video && (
            <span className="editor-video-note">
              <Clapperboard size={14} strokeWidth={1.75} aria-hidden />
              Video {info.video.width}×{info.video.height}: the picture is kept as it is; only the sound changes.
            </span>
          )}
        </section>
      </main>
    </div>
  );
}

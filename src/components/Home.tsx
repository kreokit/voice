import { useCallback, useEffect, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { AudioLines, Clapperboard, Ellipsis, FolderOpen, FolderSearch, Trash2, TriangleAlert, Upload } from "lucide-react";
import { fileName, formatDuration, relativeTime } from "../lib/files";
import { RecentEntry, clearRecent, listRecent, removeRecent, removedMessage } from "../lib/media";
import { openPaths, relocateAndOpen } from "../store/session";
import { Button, Menu, MenuEntry, toast, useContextMenu } from "../ui";
import { CLEAR_TEXT, ConfirmDialog, REMOVE_TEXT } from "./ConfirmDialog";
import { TopBar, chooseFiles, pickFile } from "./TopBar";
import "../styles/home.css";

/** Home: drop or choose recordings, or reopen a recent one. */
export function Home() {
  const [recent, setRecent] = useState<RecentEntry[] | null>(null);
  const [dragging, setDragging] = useState(false);
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState<RecentEntry | null>(null);
  const [clearing, setClearing] = useState(false);

  const refresh = useCallback(() => {
    listRecent().then(setRecent, () => setRecent([]));
  }, []);
  useEffect(refresh, [refresh]);

  const run = useCallback(
    async (work: () => Promise<unknown>) => {
      setBusy(true);
      try {
        await work();
      } finally {
        setBusy(false);
        refresh();
      }
    },
    [refresh],
  );

  // Dropping one or many files adds them to Recent and opens the first.
  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "enter" || payload.type === "over") return setDragging(true);
      setDragging(false);
      if (payload.type === "drop") void run(() => openPaths(payload.paths));
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [run]);

  const locate = (entry: RecentEntry) =>
    run(async () => {
      const path = await pickFile(`Locate ${fileName(entry.path)}`);
      if (path) await relocateAndOpen(entry.path, path);
    });

  return (
    <div className="home">
      <TopBar
        title={
          <span className="home-brand" data-tauri-drag-region>
            KreoKit Voice
          </span>
        }
      />

      <main className="home-main">
        <section className={dragging ? "dropzone dragging" : "dropzone"} aria-label="Add recordings">
          <div className="dropzone-icon">
            <Upload size={22} strokeWidth={1.75} aria-hidden />
          </div>
          <h1 className="dropzone-title">{dragging ? "Drop to open" : "Drop a recording to clean up its sound"}</h1>
          <p className="dropzone-text">
            Audio or video, one file or several. Everything runs on this computer; your recordings never leave it.
          </p>
          <Button variant="primary" onClick={() => void run(chooseFiles)} disabled={busy}>
            Choose files
          </Button>
          <p className="dropzone-formats">WAV · MP3 · M4A · FLAC · MP4 · MOV and more</p>
        </section>

        <section className="recent" aria-labelledby="recent-title">
          <h2 id="recent-title" className="section-title">
            Recent files
          </h2>
          {recent === null ? null : recent.length === 0 ? (
            <p className="recent-empty">Files you open appear here.</p>
          ) : (
            <ul className="recent-list">
              {recent.map((entry) => (
                <RecentRow
                  key={entry.path}
                  entry={entry}
                  onOpen={() => void run(() => openPaths([entry.path]))}
                  onLocate={() => void locate(entry)}
                  onRemove={() => setRemoving(entry)}
                />
              ))}
            </ul>
          )}
          {recent && recent.length > 0 && (
            <div className="recent-footer">
              <Button variant="ghost" size="sm" icon={Trash2} onClick={() => setClearing(true)}>
                Clear all recent…
              </Button>
            </div>
          )}
        </section>
      </main>

      {removing && (
        <ConfirmDialog
          title="Remove from Recent?"
          text={REMOVE_TEXT}
          confirmLabel="Remove"
          onClose={() => setRemoving(null)}
          onConfirm={() =>
            void run(async () => {
              try {
                toast(removedMessage(await removeRecent(removing.path)), "success");
              } catch (e) {
                toast(String(e), "error");
              }
            })
          }
        />
      )}
      {clearing && (
        <ConfirmDialog
          title="Clear all recent files?"
          text={CLEAR_TEXT}
          confirmLabel="Clear all"
          onClose={() => setClearing(false)}
          onConfirm={() =>
            void run(async () => {
              try {
                toast(removedMessage(await clearRecent(), true), "success");
              } catch (e) {
                toast(String(e), "error");
              }
            })
          }
        />
      )}
    </div>
  );
}

function RecentRow({
  entry,
  onOpen,
  onLocate,
  onRemove,
}: {
  entry: RecentEntry;
  onOpen: () => void;
  onLocate: () => void;
  onRemove: () => void;
}) {
  const items = (): MenuEntry[] => [
    entry.exists
      ? { label: "Open", icon: FolderOpen, onSelect: onOpen }
      : { label: "Locate…", icon: FolderSearch, onSelect: onLocate },
    "separator",
    { label: "Remove from Recent…", icon: Trash2, onSelect: onRemove },
  ];
  const { onContextMenu, menu } = useContextMenu(items, "File actions");
  const Icon = entry.is_video ? Clapperboard : AudioLines;
  return (
    <li className={entry.exists ? "recent-row" : "recent-row missing"} onContextMenu={onContextMenu}>
      <button
        type="button"
        className="recent-open"
        onClick={entry.exists ? onOpen : undefined}
        aria-disabled={!entry.exists}
        title={entry.path}
      >
        <span className="recent-icon">
          <Icon size={18} strokeWidth={1.75} aria-hidden />
        </span>
        <span className="recent-info">
          <span className="recent-name">{fileName(entry.path)}</span>
          <span className="recent-meta">
            {formatDuration(entry.duration_ms)} · {entry.is_video ? "Video" : "Audio"}
          </span>
        </span>
        {entry.exists ? (
          <span className="recent-time">{relativeTime(entry.opened_at, Date.now())}</span>
        ) : (
          <span className="recent-missing">
            <TriangleAlert size={12} strokeWidth={2} aria-hidden /> Moved or deleted
          </span>
        )}
      </button>
      {!entry.exists && (
        <Button variant="secondary" size="sm" onClick={onLocate}>
          Locate…
        </Button>
      )}
      <Menu buttonLabel={`More for ${fileName(entry.path)}`} icon={Ellipsis} variant="ghost" align="end" items={items()} />
      {menu}
    </li>
  );
}

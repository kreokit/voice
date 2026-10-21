import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Editor } from "./components/Editor";
import { Home } from "./components/Home";
import { chooseFiles } from "./components/TopBar";
import { openPaths, useSession } from "./store/session";
import { ToastRegion } from "./ui";

function App() {
  const file = useSession((s) => s.file);

  // ⌘O / Ctrl+O opens files from any screen.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void chooseFiles();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Development aid: KREOKIT_DEV_OPEN=/path/file.wav opens it at startup.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    invoke<string | null>("dev_startup_file").then((path) => path && void openPaths([path]));
  }, []);

  return (
    <>
      {file ? <Editor key={file.path} file={file} /> : <Home />}
      <ToastRegion />
    </>
  );
}

export default App;

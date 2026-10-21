import { TopBar } from "./components/TopBar";
import { ToastRegion } from "./ui";

/** The app shell. Home (drop zone + recent files) arrives in milestone 1. */
function App() {
  return (
    <>
      <div className="screen">
        <TopBar title="KreoKit Voice" />
        <main className="shell-empty">
          <p>Clean voice recordings, offline. Coming together in milestone 1.</p>
        </main>
      </div>
      <ToastRegion />
    </>
  );
}

export default App;

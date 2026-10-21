import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/inter";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/shell.css";
import App from "./App";

// macOS draws the window buttons inside our top bar (tauri.macos.conf.json).
if (navigator.userAgent.includes("Mac")) document.documentElement.classList.add("platform-mac");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import OverlayApp from "./OverlayApp";
import "./styles/app.css";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

function pickRoot() {
  try {
    const w = getCurrentWebviewWindow();
    if (w.label === "overlay") return OverlayApp;
  } catch {
    // ignore: fallback to main app
  }
  return App;
}

const Root = pickRoot();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);

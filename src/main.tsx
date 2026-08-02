import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Palette from "./Palette";
import Editor from "./Editor";
import "./global.css";

// Both windows load the same bundle; the window label decides which one this is.
const label = getCurrentWindow().label;
document.documentElement.dataset.window = label;

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{label === "editor" ? <Editor /> : <Palette />}</React.StrictMode>,
);

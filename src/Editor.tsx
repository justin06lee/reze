import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as api from "./lib/api";
import { accelerator, isModifierOnly, prettyHotkey } from "./lib/hotkey";
import { BUILTINS, collectVariables, expandIncludes } from "./lib/template";
import { useLibrary } from "./lib/useLibrary";
import { emptyMacro, type Library, type Macro } from "./lib/types";
import "./editor.css";

type SaveState = "idle" | "saving" | "saved" | "error";

export default function Editor() {
  const { library, setLibrary, error, setError } = useLibrary();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [showSettings, setShowSettings] = useState(false);
  const [trusted, setTrusted] = useState(true);

  const saveTimer = useRef<number | null>(null);

  const macros = library?.macros ?? [];
  const selected = macros.find((m) => m.id === selectedId) ?? null;

  useEffect(() => {
    if (!selectedId && macros.length > 0) setSelectedId(macros[0].id);
  }, [macros, selectedId]);

  // Permission can be granted while the app is running, so re-check whenever
  // the user comes back to this window rather than only at startup.
  useEffect(() => {
    const check = () => api.accessibilityStatus().then(setTrusted).catch(() => {});
    check();
    window.addEventListener("focus", check);
    return () => window.removeEventListener("focus", check);
  }, []);

  /** Apply a change immediately and debounce the write to disk. */
  const update = useCallback(
    (next: Library) => {
      setLibrary(next);
      setSaveState("saving");
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => {
        api
          .saveLibrary(next)
          .then(() => setSaveState("saved"))
          .catch((e) => {
            setSaveState("error");
            setError(String(e));
          });
      }, 400);
    },
    [setLibrary, setError],
  );

  const patchMacro = useCallback(
    (id: string, patch: Partial<Macro>) => {
      if (!library) return;
      update({
        ...library,
        macros: library.macros.map((m) => (m.id === id ? { ...m, ...patch } : m)),
      });
    },
    [library, update],
  );

  const addMacro = useCallback(() => {
    if (!library) return;
    const fresh = { ...emptyMacro(), name: "untitled" };
    update({ ...library, macros: [fresh, ...library.macros] });
    setSelectedId(fresh.id);
  }, [library, update]);

  const duplicateMacro = useCallback(
    (m: Macro) => {
      if (!library) return;
      const copy = { ...m, id: crypto.randomUUID(), name: `${m.name} copy`, usageCount: 0 };
      update({ ...library, macros: [copy, ...library.macros] });
      setSelectedId(copy.id);
    },
    [library, update],
  );

  const deleteMacro = useCallback(
    (m: Macro) => {
      if (!library) return;
      if (!confirm(`Delete "${m.name}"?`)) return;
      update({ ...library, macros: library.macros.filter((x) => x.id !== m.id) });
      setSelectedId(null);
    },
    [library, update],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "n") {
        e.preventDefault();
        addMacro();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [addMacro]);

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return macros;
    return macros.filter((m) =>
      [m.name, m.description, ...m.tags].join(" ").toLowerCase().includes(q),
    );
  }, [macros, filter]);

  const analysis = useMemo(() => {
    if (!selected) return null;
    const { text, missing, cyclic } = expandIncludes(selected.body, macros);
    return { text, missing, cyclic, vars: collectVariables(text) };
  }, [selected, macros]);

  if (!library) {
    return (
      <div className="e-loading">
        <span>{error ?? "Loading…"}</span>
      </div>
    );
  }

  return (
    <div className="editor">
      <header className="e-titlebar" data-tauri-drag-region>
        <span className="e-brand">Reze</span>
        <span className="e-save" data-state={saveState}>
          {saveState === "saving" ? "Saving…" : saveState === "saved" ? "Saved" : ""}
          {saveState === "error" ? "Save failed" : ""}
        </span>
        <button className="e-ghost" onClick={() => setShowSettings((s) => !s)}>
          {showSettings ? "Macros" : "Settings"}
        </button>
      </header>

      {!trusted && (
        <div className="e-banner">
          <div>
            <strong>Accessibility permission needed.</strong> Without it macOS silently
            blocks the paste keystroke — the palette will open but nothing will be inserted.
          </div>
          <div className="e-banner-actions">
            <button onClick={() => api.requestAccessibility()}>Request</button>
            <button onClick={() => api.openAccessibilitySettings()}>Open Settings</button>
          </div>
        </div>
      )}

      {error && (
        <div className="e-banner e-banner--error">
          <div>{error}</div>
          <div className="e-banner-actions">
            <button onClick={() => setError(null)}>Dismiss</button>
          </div>
        </div>
      )}

      {showSettings ? (
        <SettingsPane library={library} update={update} />
      ) : (
        <div className="e-main">
          <aside className="e-sidebar">
            <div className="e-sidebar-top">
              <input
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder="Filter"
                spellCheck={false}
              />
              <button className="e-new" onClick={addMacro} title="New macro (⌘N)">
                +
              </button>
            </div>
            <ul>
              {visible.map((m) => (
                <li
                  key={m.id}
                  data-selected={m.id === selectedId}
                  onClick={() => setSelectedId(m.id)}
                >
                  <span className="e-item-name">{m.name || "untitled"}</span>
                  <span className="e-item-meta">
                    {m.tags.map((t) => `#${t}`).join(" ")}
                    {m.usageCount > 0 && <em> · {m.usageCount}×</em>}
                  </span>
                </li>
              ))}
              {visible.length === 0 && <li className="e-item-empty">No macros</li>}
            </ul>
          </aside>

          {selected ? (
            <section className="e-detail">
              <div className="e-fields">
                <label className="e-field">
                  <span>Trigger</span>
                  <input
                    value={selected.name}
                    onChange={(e) => patchMacro(selected.id, { name: e.target.value })}
                    placeholder="full analysis"
                    spellCheck={false}
                  />
                </label>
                <label className="e-field">
                  <span>Description</span>
                  <input
                    value={selected.description}
                    onChange={(e) => patchMacro(selected.id, { description: e.target.value })}
                    placeholder="What this is for"
                  />
                </label>
                <label className="e-field e-field--narrow">
                  <span>Tags</span>
                  <input
                    value={selected.tags.join(", ")}
                    onChange={(e) =>
                      patchMacro(selected.id, {
                        tags: e.target.value
                          .split(",")
                          .map((t) => t.trim())
                          .filter(Boolean),
                      })
                    }
                    placeholder="review, analysis"
                    spellCheck={false}
                  />
                </label>
              </div>

              <textarea
                className="e-body"
                value={selected.body}
                onChange={(e) => patchMacro(selected.id, { body: e.target.value })}
                placeholder={"Fully analyze {{target}}…\n\nUse {{> another macro}} to include one."}
                spellCheck={false}
              />

              <div className="e-inspector">
                <div className="e-chips">
                  {analysis?.vars.map((v) => (
                    <span className="e-chip" key={v.name}>
                      {v.name}
                      {v.default && <em> = {v.default}</em>}
                    </span>
                  ))}
                  {analysis?.missing.map((name) => (
                    <span className="e-chip e-chip--bad" key={`miss-${name}`}>
                      missing: {name}
                    </span>
                  ))}
                  {analysis?.cyclic.map((name) => (
                    <span className="e-chip e-chip--bad" key={`cycle-${name}`}>
                      cycle: {name}
                    </span>
                  ))}
                  {analysis && analysis.vars.length === 0 && analysis.missing.length === 0 && (
                    <span className="e-chip e-chip--muted">no variables</span>
                  )}
                </div>
                <details>
                  <summary>Expanded preview</summary>
                  <pre>{analysis?.text}</pre>
                </details>
              </div>

              <div className="e-actions">
                <button onClick={() => duplicateMacro(selected)}>Duplicate</button>
                <button className="e-danger" onClick={() => deleteMacro(selected)}>
                  Delete
                </button>
              </div>
            </section>
          ) : (
            <section className="e-detail e-detail--empty">
              <p>Select a macro, or create one with ⌘N.</p>
            </section>
          )}
        </div>
      )}
    </div>
  );
}

function SettingsPane({
  library,
  update,
}: {
  library: Library;
  update: (next: Library) => void;
}) {
  const [recording, setRecording] = useState(false);
  const [path, setPath] = useState("");
  const [hotkeyError, setHotkeyError] = useState<string | null>(null);

  useEffect(() => {
    api.libraryPath().then(setPath).catch(() => {});
  }, []);

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      if (isModifierOnly(e)) return;
      const accel = accelerator(e);
      if (!accel) {
        setHotkeyError("Include at least one modifier (⌘, ⌥, ⌃ or ⇧).");
        return;
      }
      setRecording(false);
      api
        .setHotkey(accel)
        .then(() => {
          setHotkeyError(null);
          update({ ...library, settings: { ...library.settings, hotkey: accel } });
        })
        .catch((err) => setHotkeyError(String(err)));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, library, update]);

  const s = library.settings;

  return (
    <div className="e-settings">
      <section>
        <h2>Hotkey</h2>
        <p>Opens the palette over whatever app you are in.</p>
        <div className="e-row">
          <button className="e-hotkey" data-recording={recording} onClick={() => setRecording(true)}>
            {recording ? "Press a combination…" : prettyHotkey(s.hotkey)}
          </button>
          <code>{s.hotkey}</code>
        </div>
        {hotkeyError && <p className="e-warn">{hotkeyError}</p>}
      </section>

      <section>
        <h2>Delivery</h2>
        <label className="e-check">
          <input
            type="checkbox"
            checked={s.pasteMode === "paste"}
            onChange={(e) =>
              update({
                ...library,
                settings: { ...s, pasteMode: e.target.checked ? "paste" : "copy" },
              })
            }
          />
          <span>
            Paste directly into the focused app
            <em>Off: the expansion only goes to the clipboard.</em>
          </span>
        </label>
        <label className="e-check">
          <input
            type="checkbox"
            checked={s.restoreClipboard}
            onChange={(e) =>
              update({ ...library, settings: { ...s, restoreClipboard: e.target.checked } })
            }
          />
          <span>
            Restore my clipboard afterwards
            <em>Puts back whatever you had copied once the paste lands.</em>
          </span>
        </label>
      </section>

      <section>
        <h2>Template syntax</h2>
        <ul className="e-syntax">
          <li>
            <code>{"{{name}}"}</code> a variable you get prompted for
          </li>
          <li>
            <code>{"{{name|default}}"}</code> same, pre-filled
          </li>
          <li>
            <code>{"{{> other macro}}"}</code> inline another macro by its trigger
          </li>
          <li>
            <code>{BUILTINS.map((b) => `{{${b}}}`).join(" ")}</code> filled automatically
          </li>
        </ul>
      </section>

      <section>
        <h2>Storage</h2>
        <p>
          This file is the source of truth. Edit it in any text editor and Reze picks up the
          change immediately.
        </p>
        <div className="e-row">
          <code className="e-path">{path}</code>
          <button onClick={() => api.revealLibrary()}>Reveal in Finder</button>
        </div>
      </section>
    </div>
  );
}

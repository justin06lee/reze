import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import * as api from "./lib/api";
import { AttachmentMedia, AttachmentTile, ClipIcon } from "./Attachments";
import { accelerator, isModifierOnly, prettyHotkey } from "./lib/hotkey";
import { BUILTINS, collectVariables, expandIncludes } from "./lib/template";
import { useLibrary } from "./lib/useLibrary";
import {
  emptyMacro,
  formatBytes,
  type GrabTools,
  type HotkeySlot,
  type Imported,
  type Library,
  type LoginItemState,
  type Macro,
} from "./lib/types";
import "./editor.css";

type SaveState = "idle" | "saving" | "saved" | "error";

export default function Editor() {
  const { library, setLibrary, error, setError } = useLibrary();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [showSettings, setShowSettings] = useState(false);
  const [trusted, setTrusted] = useState(true);
  const [armedDelete, setArmedDelete] = useState<string | null>(null);
  // True while files are hovering over the window, so the drop target can show
  // itself. Tauri handles the drag natively, so there is no HTML drag event.
  const [dropping, setDropping] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const saveTimer = useRef<number | null>(null);
  const armTimer = useRef<number | null>(null);

  const macros = library?.macros ?? [];
  const selected = macros.find((m) => m.id === selectedId) ?? null;

  useEffect(() => {
    if (!selectedId && macros.length > 0) setSelectedId(macros[0].id);
  }, [macros, selectedId]);

  // Never leave a delete armed on a macro the user has navigated away from.
  useEffect(() => setArmedDelete(null), [selectedId]);

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

  /**
   * Copy a macro into one you own.
   *
   * For a folder macro this is the way to make it yours: its files are copied
   * into the asset library so the copy no longer depends on the folder, and the
   * `folder` mark is dropped so it stops being regenerated and starts being
   * saved. That makes "Duplicate" the answer to "I want to add text to this one".
   */
  const duplicateMacro = useCallback(
    async (m: Macro) => {
      if (!library) return;
      let attachments = m.attachments;
      if (m.folder) {
        const paths = m.attachments.map((a) => a.path).filter((p): p is string => Boolean(p));
        const imported = await api.importFiles(paths).catch((e) => {
          setNotice(String(e));
          return null;
        });
        if (!imported) return;
        if (imported.errors.length > 0) setNotice(imported.errors.join(" · "));
        attachments = imported.attachments;
      }
      const copy: Macro = {
        ...m,
        id: crypto.randomUUID(),
        name: m.folder ? m.name : `${m.name} copy`,
        // The path it came from is the folder macro's description; a copy of it
        // would be describing somewhere the copy no longer depends on.
        description: m.folder ? "" : m.description,
        attachments,
        folder: undefined,
        usageCount: 0,
      };
      update({ ...library, macros: [copy, ...library.macros] });
      setSelectedId(copy.id);
    },
    [library, update],
  );

  // Two-step rather than window.confirm(): wry ships no WKUIDelegate, so a
  // webview confirm() never shows a panel and silently returns false — which
  // made the delete button do nothing at all.
  const deleteMacro = useCallback(
    (m: Macro) => {
      if (!library) return;
      if (armedDelete !== m.id) {
        setArmedDelete(m.id);
        if (armTimer.current) clearTimeout(armTimer.current);
        armTimer.current = window.setTimeout(() => setArmedDelete(null), 3000);
        return;
      }
      if (armTimer.current) clearTimeout(armTimer.current);
      setArmedDelete(null);
      update({ ...library, macros: library.macros.filter((x) => x.id !== m.id) });
      setSelectedId(null);
    },
    [library, update, armedDelete],
  );

  // ------------------------------------------------------------ attachments

  /**
   * Add imported files to the selected macro.
   *
   * With nothing selected, the drop makes a macro instead of being refused —
   * dragging a screenshot onto an empty editor should produce something, and
   * the file's own name is the obvious trigger to give it.
   */
  const attach = useCallback(
    (imported: Imported) => {
      if (!library) return;
      setNotice(imported.errors.length > 0 ? imported.errors.join(" · ") : null);
      if (imported.attachments.length === 0) {
        if (imported.errors.length === 0) setNotice("Nothing there to attach.");
        return;
      }

      const target = library.macros.find((m) => m.id === selectedId);
      if (target) {
        update({
          ...library,
          macros: library.macros.map((m) =>
            m.id === target.id
              ? { ...m, attachments: [...m.attachments, ...imported.attachments] }
              : m,
          ),
        });
        return;
      }

      const first = imported.attachments[0].name;
      const fresh = {
        ...emptyMacro(),
        name: (first.includes(".") ? first.slice(0, first.lastIndexOf(".")) : first) || "untitled",
        attachments: imported.attachments,
      };
      update({ ...library, macros: [fresh, ...library.macros] });
      setSelectedId(fresh.id);
    },
    [library, selectedId, update],
  );

  // The drop and paste listeners are registered once and must not be torn down
  // and rebuilt on every keystroke in the body field, so they reach the current
  // `attach` through a ref rather than closing over it.
  const attachRef = useRef(attach);
  useEffect(() => {
    attachRef.current = attach;
  }, [attach]);

  const addFiles = useCallback(async () => {
    const picked = await open({ multiple: true, title: "Attach to macro" }).catch((e) => {
      setNotice(String(e));
      return null;
    });
    if (!picked) return;
    const paths = Array.isArray(picked) ? picked : [picked];
    attachRef.current(await api.importFiles(paths));
  }, []);

  const pasteFiles = useCallback(async () => {
    attachRef.current(await api.importClipboard());
  }, []);

  const detach = useCallback(
    (macro: Macro, attachmentId: string) =>
      patchMacro(macro.id, {
        // The file itself is left on disk; the sweep after this save collects it.
        attachments: macro.attachments.filter((a) => a.id !== attachmentId),
      }),
    [patchMacro],
  );

  /**
   * Files dropped on the window.
   *
   * Tauri intercepts the drag natively, so the webview never sees an HTML drop
   * event — this is the only way to know it happened.
   */
  useEffect(() => {
    const un = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "enter" || payload.type === "over") {
        setDropping(true);
      } else if (payload.type === "leave") {
        setDropping(false);
      } else {
        setDropping(false);
        api
          .importFiles(payload.paths)
          .then((r) => attachRef.current(r))
          .catch((e) => setNotice(String(e)));
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  /**
   * ⌘V with files on the clipboard.
   *
   * The event only tells us files are *there*; the bytes come from reading the
   * real pasteboard natively, which sees a Finder-copied file the webview would
   * hand over as nothing. Pasting text is left entirely alone.
   */
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      if (!e.clipboardData?.types.includes("Files")) return;
      e.preventDefault();
      pasteFiles().catch((err) => setNotice(String(err)));
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, [pasteFiles]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === "n") {
        e.preventDefault();
        addMacro();
      } else if (e.key === "q") {
        // No application menu on a menu-bar app, so ⌘Q would otherwise do
        // nothing at all from here.
        e.preventDefault();
        api.quit();
      } else if (e.key === "w") {
        e.preventDefault();
        getCurrentWindow().hide();
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

  // Listed apart because they are a different kind of thing, and because a
  // folder of two thousand images would otherwise bury the ten you wrote.
  const yours = useMemo(() => visible.filter((m) => !m.folder), [visible]);
  const fromFolders = useMemo(() => visible.filter((m) => m.folder), [visible]);

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

      {notice && (
        <div className="e-banner">
          <div>{notice}</div>
          <div className="e-banner-actions">
            <button onClick={() => setNotice(null)}>Dismiss</button>
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
              {yours.map((m) => (
                <MacroRow
                  key={m.id}
                  macro={m}
                  selected={m.id === selectedId}
                  onSelect={() => setSelectedId(m.id)}
                />
              ))}
              {fromFolders.length > 0 && (
                <li className="e-item-head">From folders · {fromFolders.length}</li>
              )}
              {fromFolders.map((m) => (
                <MacroRow
                  key={m.id}
                  macro={m}
                  selected={m.id === selectedId}
                  onSelect={() => setSelectedId(m.id)}
                />
              ))}
              {visible.length === 0 && <li className="e-item-empty">No macros</li>}
            </ul>
          </aside>

          {selected?.folder ? (
            <FolderDetail macro={selected} onAdopt={() => duplicateMacro(selected)} />
          ) : selected ? (
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

              <div className="e-attachments">
                <div className="e-attachments-head">
                  <span>
                    Attachments
                    {selected.attachments.length > 0 && (
                      <em> · {selected.attachments.length}</em>
                    )}
                  </span>
                  <button onClick={addFiles}>Add file…</button>
                  <button onClick={() => pasteFiles()}>Paste</button>
                </div>
                {selected.attachments.length > 0 ? (
                  <div className="e-tiles">
                    {selected.attachments.map((a) => (
                      <AttachmentTile
                        key={a.id}
                        attachment={a}
                        onRemove={() => detach(selected, a.id)}
                      />
                    ))}
                  </div>
                ) : (
                  <p className="e-attachments-empty">
                    Drop an image or video anywhere in this window, or press ⌘V with one
                    copied. They are pasted after the text when the macro runs.
                  </p>
                )}
              </div>

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
                <button
                  className="e-danger"
                  data-armed={armedDelete === selected.id}
                  onClick={() => deleteMacro(selected)}
                >
                  {armedDelete === selected.id ? "Click again to delete" : "Delete"}
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

      {dropping && (
        <div className="e-drop">
          <div className="e-drop-card">Drop to attach</div>
        </div>
      )}
    </div>
  );
}

/** One macro in the sidebar. Folder macros are marked, not hidden. */
function MacroRow({
  macro,
  selected,
  onSelect,
}: {
  macro: Macro;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <li data-selected={selected} onClick={onSelect}>
      <span className="e-item-name">
        {macro.attachments.length > 0 && (
          <span className="e-item-clip">
            <ClipIcon />
          </span>
        )}
        {macro.name || "untitled"}
      </span>
      <span className="e-item-meta">
        {macro.tags.map((t) => `#${t}`).join(" ")}
        {macro.usageCount > 0 && <em> · {macro.usageCount}×</em>}
      </span>
    </li>
  );
}

/**
 * A macro that came from a watched folder.
 *
 * Deliberately not the editing pane: the folder is what this macro is made of,
 * so every field here would be overwritten by the next scan. Showing the real
 * fields greyed out would be a lie about what is possible — this says what the
 * macro is, where it came from, and the two things you can actually do.
 */
function FolderDetail({ macro, onAdopt }: { macro: Macro; onAdopt: () => void }) {
  const file = macro.attachments[0];
  return (
    <section className="e-detail e-detail--folder">
      <div className="e-folder-head">
        <h2>{macro.name}</h2>
        <p>
          From <code>{macro.folder}</code>
        </p>
        <p className="e-note">
          Made from the file, so it changes when the file does. Rename the file to rename the
          trigger; move it out of the folder to remove the macro.
        </p>
      </div>

      {file && (
        <div className="e-folder-preview">
          <AttachmentMedia attachment={file} playable />
        </div>
      )}

      <div className="e-folder-facts">
        <span>
          <em>Path</em> {macro.description}
        </span>
        {macro.tags.length > 0 && (
          <span>
            <em>Tags</em> {macro.tags.map((t) => `#${t}`).join(" ")}
          </span>
        )}
        {file && (
          <span>
            <em>File</em> {formatBytes(file.bytes)} · {file.mime}
          </span>
        )}
      </div>

      <div className="e-actions">
        {file?.path && <button onClick={() => api.revealPath(file.path!)}>Reveal in Finder</button>}
        <button onClick={onAdopt}>Make an editable copy</button>
      </div>
    </section>
  );
}

function SettingsPane({
  library,
  update,
}: {
  library: Library;
  update: (next: Library) => void;
}) {
  const [recording, setRecording] = useState<HotkeySlot | null>(null);
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
      const next = { ...library.settings, [recording]: accel };
      setRecording(null);
      // Register before saving: if the combination is already taken by another
      // app, the old one keeps working and nothing is written.
      api
        .setHotkeys(next.hotkey, next.expandHotkey, next.clipboardHotkey)
        .then(() => {
          setHotkeyError(null);
          update({ ...library, settings: next });
        })
        .catch((err) => setHotkeyError(String(err)));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, library, update]);

  const s = library.settings;

  const recorder = (slot: HotkeySlot) => (
    <div className="e-row">
      <button
        className="e-hotkey"
        data-recording={recording === slot}
        onClick={() => setRecording(slot)}
      >
        {recording === slot ? "Press a combination…" : prettyHotkey(s[slot])}
      </button>
      <code>{s[slot]}</code>
    </div>
  );

  return (
    <div className="e-settings">
      <section>
        <h2>Palette hotkey</h2>
        <p>Opens the search palette over whatever app you are in.</p>
        {recorder("hotkey")}
      </section>

      <section>
        <h2>Expand in place</h2>
        <p>
          Type a trigger like <code>full analysis</code> straight into any app, then press this.
          Reze reads back the words before your cursor, and if they name a macro it replaces
          them with the expansion — no window, no search. Nothing happens if they do not match
          anything, and your text is left as it was.
        </p>
        {recorder("expandHotkey")}
        {hotkeyError && <p className="e-warn">{hotkeyError}</p>}
        <p className="e-note">
          A plain key like Tab cannot be used: a global shortcut swallows that key in every
          application, so it needs at least one modifier.
        </p>

        <label className="e-check" style={{ marginTop: 16 }}>
          <input
            type="checkbox"
            checked={s.trackTyping}
            onChange={(e) =>
              update({ ...library, settings: { ...s, trackTyping: e.target.checked } })
            }
          />
          <span>
            Watch typing so this works in terminals
            <em>
              Terminals and TUIs do not let anything select or copy the line you are editing, so
              the only way to know what you typed is to have watched you type it. Reze keeps the
              last 160 characters in memory — never written to disk, never sent anywhere — and
              discards them on Enter, Tab, Escape, any arrow key, any click, any shortcut, and
              whenever you switch apps. macOS blocks this outright while a password field is
              focused. Turn it off and expansion falls back to selecting the words at your
              caret, which works in normal text fields but not in a terminal.
            </em>
          </span>
        </label>
        <p className="e-note">Takes effect next time Reze starts.</p>
      </section>

      <ClipboardSection library={library} update={update} recorder={recorder} />

      <FoldersSection library={library} />

      <GrabSection />

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

      <StartupSection />

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


/**
 * Clipboard history.
 *
 * The one feature here that records something whether or not you asked it to in
 * the moment, so it says plainly what it keeps and where — and offers the two
 * things a person actually wants from that: a limit, and a way to wipe it.
 */
function ClipboardSection({
  library,
  update,
  recorder,
}: {
  library: Library;
  update: (next: Library) => void;
  recorder: (slot: HotkeySlot) => React.ReactNode;
}) {
  const [armed, setArmed] = useState(false);
  const [cleared, setCleared] = useState(false);
  const s = library.settings;

  const clear = async () => {
    if (!armed) {
      setArmed(true);
      return;
    }
    await api.clearClips().catch(() => {});
    setArmed(false);
    setCleared(true);
  };

  return (
    <section>
      <h2>Clipboard history</h2>
      <p>
        Everything you copy, with the app you copied it from. Press this anywhere and pick one
        to paste it back where your cursor is — text, images, and files all come back as what
        they were.
      </p>
      {recorder("clipboardHotkey")}

      <label className="e-check" style={{ marginTop: 16 }}>
        <input
          type="checkbox"
          checked={s.clipboardHistory}
          onChange={(e) =>
            update({ ...library, settings: { ...s, clipboardHistory: e.target.checked } })
          }
        />
        <span>
          Remember what I copy
          <em>
            Kept in <code>~/.reze/clipboard.json</code> on this machine, in plain text —
            nothing is sent anywhere. Apps that mark a copy as a password, and anything Reze
            itself puts on the clipboard, are skipped. Turn this off and the list stops growing;
            what is already in it stays until you clear it.
          </em>
        </span>
      </label>

      <div className="e-row">
        <label className="e-limit">
          <span>Keep</span>
          <input
            type="number"
            min={1}
            max={1000}
            value={s.clipboardLimit}
            onChange={(e) => {
              const n = Number(e.target.value);
              if (!Number.isFinite(n)) return;
              update({
                ...library,
                settings: { ...s, clipboardLimit: Math.min(1000, Math.max(1, Math.round(n))) },
              });
            }}
          />
          <span>items</span>
        </label>
        <button className="e-danger" onClick={clear} data-armed={armed || undefined}>
          {armed ? "Really forget everything?" : "Clear history"}
        </button>
        {armed && <button onClick={() => setArmed(false)}>Cancel</button>}
        {cleared && !armed && <span className="e-badge">cleared</span>}
      </div>
    </section>
  );
}

/**
 * Watched folders.
 *
 * Saved through their own command rather than the debounced library write: the
 * backend has to re-point the file watcher and rescan, and doing that on a
 * timer after a click would leave the list looking applied before it was.
 */
function FoldersSection({ library }: { library: Library }) {
  const [error, setError] = useState<string | null>(null);
  const folders = library.settings.folders;

  const apply = (next: string[]) =>
    api.setFolders(next).catch((e) => setError(String(e)));

  const add = async () => {
    setError(null);
    const picked = await open({ directory: true, multiple: false, title: "Watch folder" }).catch(
      (e) => {
        setError(String(e));
        return null;
      },
    );
    if (typeof picked !== "string" || folders.includes(picked)) return;
    apply([...folders, picked]);
  };

  return (
    <section>
      <h2>Folders</h2>
      <p>
        Every file in these becomes a macro on its own, named after the file, with its
        subfolders as tags. Add a file and it appears; delete one and it goes. They are read
        where they lie — nothing is copied.
      </p>

      <ul className="e-folders">
        {folders.map((folder, i) => (
          <li key={folder}>
            <code>{folder}</code>
            {i === 0 && <span className="e-badge">grabs land here</span>}
            <button onClick={() => api.revealPath(folder)}>Reveal</button>
            <button onClick={() => apply(folders.filter((f) => f !== folder))}>Remove</button>
          </li>
        ))}
        {folders.length === 0 && (
          <li className="e-folders-empty">
            No folders watched. Grabbing needs one to save into.
          </li>
        )}
      </ul>

      <div className="e-row">
        <button onClick={add}>Add folder…</button>
      </div>
      {error && <p className="e-warn">{error}</p>}
      <p className="e-note">
        Macros from a folder cannot be edited here — the folder is what they are made of, so
        rename or delete the file instead.
      </p>
    </section>
  );
}

/**
 * Grabbing a link.
 *
 * The tools are looked up rather than bundled, so this section's real job is to
 * say plainly which of them are missing and what to type to get them.
 */
function GrabSection() {
  const [tools, setTools] = useState<GrabTools | null>(null);

  useEffect(() => {
    // Re-checked when the window comes forward, so installing a tool and
    // switching back shows it without a restart.
    const check = () => api.grabTools().then(setTools).catch(() => {});
    check();
    window.addEventListener("focus", check);
    return () => window.removeEventListener("focus", check);
  }, []);

  const missing = tools
    ? [
        !tools.ytDlp && "yt-dlp",
        !tools.galleryDl && "gallery-dl",
        !tools.ffmpeg && "ffmpeg",
      ].filter(Boolean)
    : [];

  return (
    <section>
      <h2>Downloading a link</h2>
      <p>
        Paste the URL of a post — YouTube, X, Instagram, Reddit, TikTok, or a link straight to
        an image — into the palette instead of a search. Reze recognises it, downloads whatever
        the post is made of into the first folder above, and asks what to call it. It is a
        macro the moment you press return.
      </p>

      {tools && (
        <ul className="e-tools">
          <ToolRow name="yt-dlp" at={tools.ytDlp} does="video, and most sites" />
          <ToolRow name="gallery-dl" at={tools.galleryDl} does="images" />
          <ToolRow name="ffmpeg" at={tools.ffmpeg} does="video above 720p" />
        </ul>
      )}
      {missing.length > 0 && (
        <p className="e-warn">
          Missing {missing.join(", ")} — install with{" "}
          <code>brew install {missing.join(" ")}</code>
        </p>
      )}
      <p className="e-note">
        Only public posts. Nothing here logs in or works around access control, so anything you
        could not open in a browser cannot be grabbed either.
      </p>
    </section>
  );
}

function ToolRow({ name, at, does }: { name: string; at: string | null; does: string }) {
  return (
    <li data-found={at ? "true" : "false"}>
      <span className="e-tool-dot" aria-hidden />
      <code>{name}</code>
      <em>{does}</em>
      <span className="e-tool-at">{at ?? "not installed"}</span>
    </li>
  );
}

/**
 * Launch at login.
 *
 * The OS owns this, not `macros.json`, so there is nothing to save — the
 * checkbox reflects what the system reports and re-reads it whenever the window
 * comes forward, because the user can flip it in System Settings behind us.
 */
function StartupSection() {
  // Null until the first read lands. Starting at any real value would flash the
  // wrong answer — most visibly "not available" on a machine where it is.
  const [state, setState] = useState<LoginItemState | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(
    () => api.loginItemStatus().then(setState).catch(() => {}),
    [],
  );

  useEffect(() => {
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [refresh]);

  const toggle = async (next: boolean) => {
    setError(null);
    try {
      await api.setLoginItem(next);
    } catch (e) {
      setError(String(e));
    }
    // Re-read either way: the truth is whatever macOS now says, not what we asked for.
    refresh();
  };

  return (
    <section>
      <h2>Startup</h2>
      {state === null ? null : state === "unsupported" ? (
        <p className="e-note">
          Not available from here — this needs macOS 13 or later, and the installed{" "}
          <code>Reze.app</code> rather than a dev build. Add Reze under System Settings →
          General → Login Items by hand instead.
        </p>
      ) : (
        <>
          <label className="e-check">
            <input
              type="checkbox"
              checked={state === "enabled"}
              onChange={(e) => toggle(e.target.checked)}
            />
            <span>
              Start Reze when I log in
              <em>
                Comes back in the menu bar on every login, so the hotkeys are live without
                opening anything. macOS lists it under System Settings → General → Login Items.
              </em>
            </span>
          </label>

          {state === "requiresApproval" && (
            <p className="e-warn">
              Registered, but login items are switched off for Reze in System Settings — only
              you can turn that back on.
            </p>
          )}
          {error && <p className="e-warn">{error}</p>}
          {(state === "requiresApproval" || error) && (
            <div className="e-row">
              <button onClick={() => api.openLoginItemsSettings()}>Open Login Items</button>
            </div>
          )}

          <p className="e-note">
            Reinstalling Reze replaces the app bundle and clears this, the same way it clears
            the Accessibility grant — tick it again after <code>make update</code>.
          </p>
        </>
      )}
    </section>
  );
}

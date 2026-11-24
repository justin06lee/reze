import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import * as api from "./lib/api";
import { AttachmentMedia, AttachmentTile, ClipIcon } from "./Attachments";
import Onboarding from "./Onboarding";
import { accelerator, isModifierOnly, prettyHotkey } from "./lib/hotkey";
import { BUILTINS, collectVariables, expandIncludes } from "./lib/template";
import { useLibrary } from "./lib/useLibrary";
import { INSTALL, isMac, MOD, REVEAL } from "./lib/platform";
import {
  emptyMacro,
  formatBytes,
  type Device,
  type GrabTools,
  type HotkeySlot,
  type Imported,
  type ImportOutcome,
  type ImportPreview,
  type Library,
  type LoginItemState,
  type Macro,
  type Tailnet,
} from "./lib/types";
import "./editor.css";

type SaveState = "idle" | "saving" | "saved" | "error";

/**
 * What has been done in the window and has not reached the disk: which macros
 * were added or edited, which deleted, and whether a setting changed.
 *
 * Ids, not macros. What an edited macro now says is read off the library the
 * window is showing at the moment it is saved.
 */
type Unsaved = { touched: Set<string>; removed: Set<string>; settings: boolean };

const nothingUnsaved = (): Unsaved => ({ touched: new Set(), removed: new Set(), settings: false });

/** Add to `unsaved` whatever turns `before` into `after`. */
function note(unsaved: Unsaved, before: Library, after: Library) {
  const was = new Map(before.macros.map((m) => [m.id, m]));
  for (const m of after.macros) {
    // Every edit makes a new object for the macro it edits and leaves the
    // rest as they were, so the same object is the same macro, untouched.
    if (!m.folder && was.get(m.id) !== m) {
      unsaved.touched.add(m.id);
      unsaved.removed.delete(m.id);
    }
    was.delete(m.id);
  }
  for (const m of was.values()) {
    if (m.folder) continue;
    unsaved.removed.add(m.id);
    unsaved.touched.delete(m.id);
  }
  if (after.settings !== before.settings) unsaved.settings = true;
}

/**
 * A library arriving from disk, with what the window has done and not yet
 * saved laid back over it.
 *
 * Without this, a macro synced in from another machine mid-sentence would
 * redraw the field being typed in as it was at the last save, and the next
 * keystroke would be typed into that.
 */
function overlay(incoming: Library, shown: Library, unsaved: Unsaved[]): Library {
  const touched = (id: string) => unsaved.some((u) => u.touched.has(id));
  const removed = (id: string) => unsaved.some((u) => u.removed.has(id));
  const ours = new Map(shown.macros.map((m) => [m.id, m]));
  const theirs = new Set(incoming.macros.map((m) => m.id));
  return {
    ...incoming,
    settings: unsaved.some((u) => u.settings)
      ? // The folder list is set by a command of its own, not saved from here.
        { ...shown.settings, folders: incoming.settings.folders }
      : incoming.settings,
    macros: [
      // Made here and not saved yet, so the disk has not heard of them.
      ...shown.macros.filter((m) => touched(m.id) && !removed(m.id) && !theirs.has(m.id)),
      ...incoming.macros
        .filter((m) => !removed(m.id))
        .map((m) => (touched(m.id) ? (ours.get(m.id) ?? m) : m)),
    ],
  };
}

export default function Editor() {
  // The library as this window has it, which runs ahead of the one on disk by
  // whatever is in `unsaved` — and, for the moment a save takes, `saving`.
  const shown = useRef<Library | null>(null);
  const unsaved = useRef(nothingUnsaved());
  const saving = useRef<Unsaved[]>([]);
  const arriving = useCallback((incoming: Library) => {
    const ahead = [...saving.current, unsaved.current];
    shown.current = shown.current ? overlay(incoming, shown.current, ahead) : incoming;
    return shown.current;
  }, []);

  const { library, setLibrary, error, setError } = useLibrary(arriving);
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

  /** Send what has been done since the last save, and only that. */
  const write = useCallback(async () => {
    const library = shown.current;
    const changes = unsaved.current;
    const any = changes.touched.size > 0 || changes.removed.size > 0 || changes.settings;
    if (!library || !any) return;
    unsaved.current = nothingUnsaved();
    saving.current.push(changes);
    try {
      await api.saveLibrary({
        macros: library.macros.filter((m) => changes.touched.has(m.id)),
        removed: [...changes.removed],
        settings: changes.settings ? library.settings : null,
      });
      const more = unsaved.current;
      if (more.touched.size + more.removed.size === 0 && !more.settings) setSaveState("saved");
    } catch (e) {
      // Still unsaved, then: back with whatever has been done since, which
      // has the last word on anything both mention.
      const since = unsaved.current;
      for (const id of changes.touched) if (!since.removed.has(id)) since.touched.add(id);
      for (const id of changes.removed) if (!since.touched.has(id)) since.removed.add(id);
      since.settings ||= changes.settings;
      setSaveState("error");
      setError(String(e));
    } finally {
      saving.current = saving.current.filter((sent) => sent !== changes);
    }
  }, [setError]);

  /** Apply a change immediately and debounce the write to disk. */
  const update = useCallback(
    (next: Library) => {
      // Against the library this render drew, which is the one `next` was
      // made from — not `shown`, which a library arriving since may have moved
      // on. A macro that had just come in would otherwise be missing from
      // `next` and noted as deleted.
      if (library) note(unsaved.current, library, next);
      shown.current = next;
      setLibrary(next);
      setSaveState("saving");
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(write, 400);
    },
    [library, setLibrary, write],
  );

  /**
   * Write a pending change now instead of when its timer fires.
   *
   * For anything that rewrites the library behind the editor's back — an
   * import, or undoing one — and would otherwise be working from a library
   * that does not have the last thing typed in it.
   */
  const flush = useCallback(async () => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
    await write();
  }, [write]);

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

  /**
   * Ctrl+V on Linux, where the paste event above never sees a file: WebKitGTK
   * hands the page a copied file as nothing at all — no "Files", not even its
   * URI list. So the keystroke asks the real clipboard itself. Files or a
   * picture there are attached; anything else is typed in exactly where the
   * paste would have put it.
   */
  useEffect(() => {
    if (isMac) return;
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.shiftKey || e.altKey || e.metaKey || e.key.toLowerCase() !== "v") return;
      const field = document.activeElement as HTMLElement | null;
      // Only where a paste could go — not, say, the hotkey recorder's button.
      const editable =
        !field ||
        field === document.body ||
        field.isContentEditable ||
        field instanceof HTMLTextAreaElement ||
        field instanceof HTMLInputElement;
      if (!editable) return;
      e.preventDefault();
      api
        .importClipboard()
        .then(async (imported) => {
          if (imported.attachments.length > 0 || imported.errors.length > 0) {
            attachRef.current(imported);
            return;
          }
          const text = await api.readClipboard();
          if (text && field && field !== document.body) {
            field.focus();
            document.execCommand("insertText", false, text);
          }
        })
        .catch((err) => setNotice(String(err)));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

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
        api.closeEditor();
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

  // Not set up on this machine yet: the welcome, in place of everything else.
  if (!library.settings.onboarded) {
    return (
      <div className="editor">
        <header className="e-titlebar" data-tauri-drag-region>
          <span className="e-brand">Reze</span>
        </header>
        <Onboarding library={library} update={update} />
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
        <SettingsPane library={library} update={update} flush={flush} />
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
              <button className="e-new" onClick={addMacro} title={`New macro (${MOD}N)`}>
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
                    Drop an image or video anywhere in this window, or press {MOD}V with one
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
              <p>Select a macro, or create one with {MOD}N.</p>
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
        {file?.path && <button onClick={() => api.revealPath(file.path!)}>{REVEAL}</button>}
        <button onClick={onAdopt}>Make an editable copy</button>
      </div>
    </section>
  );
}

function SettingsPane({
  library,
  update,
  flush,
}: {
  library: Library;
  update: (next: Library) => void;
  flush: () => Promise<void>;
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
        setHotkeyError(
          isMac
            ? "Include at least one modifier (⌘, ⌥, ⌃ or ⇧)."
            : "Include at least one modifier (Ctrl, Alt, Super or Shift).",
        );
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
              whenever you switch apps.{" "}
              {isMac
                ? "macOS blocks this outright while a password field is focused."
                : "X11 has no equivalent of macOS's secure input, so a password typed into a field passes through it too — in memory only, and gone at the Enter that submits it."}{" "}
              Turn it off and expansion falls back to selecting the words at your caret, which
              works in normal text fields but not in a terminal.
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

      <DevicesSection library={library} update={update} flush={flush} />

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
          <button onClick={() => api.revealLibrary()}>{REVEAL}</button>
        </div>
      </section>

      <section>
        <h2>Welcome</h2>
        <p>The tour this window opened on the first time: the keys, the permission, your other machines.</p>
        <div className="e-row">
          <button onClick={() => update({ ...library, settings: { ...s, onboarded: false } })}>
            Show it again
          </button>
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
            nothing is sent anywhere unless you share it below. Apps that mark a copy as a
            password, and anything Reze
            itself puts on the clipboard, are skipped. Turn this off and the list stops growing;
            what is already in it stays until you clear it.
          </em>
        </span>
      </label>

      <label className="e-check">
        <input
          type="checkbox"
          checked={s.clipboardSync && s.shareOverTailscale && s.clipboardHistory}
          disabled={!s.clipboardHistory}
          onChange={(e) => {
            const on = e.target.checked;
            update({
              ...library,
              settings: {
                ...s,
                clipboardSync: on,
                shareOverTailscale: on || s.shareOverTailscale,
              },
            });
          }}
        />
        <span>
          Share the clipboard between my devices
          <em>
            Copy on one machine, paste on another: what you copy here is on the clipboard of
            your other machines a moment later, and what you copy there is on this one. Text
            and images — not files, and not what a password manager marks as a password. It
            travels over Tailscale between machines signed in as you, takes this being on at
            both ends, and needs the history above, since the last thing in it is what is
            handed over.
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
          <code>{installHint(missing as string[])}</code>
        </p>
      )}
      <p className="e-note">
        Only public posts. Nothing here logs in or works around access control, so anything you
        could not open in a browser cannot be grabbed either.
      </p>
    </section>
  );
}

/**
 * The command that installs what is missing. One package manager on macOS; on
 * Linux ffmpeg is a system package, while the downloaders come from pipx —
 * a distribution's yt-dlp is months behind the sites it has to keep up with.
 */
function installHint(missing: string[]): string {
  if (isMac) return `${INSTALL} ${missing.join(" ")}`;
  const tools = missing.filter((m) => m !== "ffmpeg");
  return [
    missing.includes("ffmpeg") && "sudo apt install ffmpeg",
    tools.length > 0 && `${INSTALL} ${tools.join(" ")}`,
  ]
    .filter(Boolean)
    .join(" && ");
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
        isMac ? (
          <p className="e-note">
            Not available from here — this needs macOS 13 or later, and the installed{" "}
            <code>Reze.app</code> rather than a dev build. Add Reze under System Settings →
            General → Login Items by hand instead.
          </p>
        ) : (
          <p className="e-note">
            Not available from a dev build — run <code>make</code> to install Reze first.
          </p>
        )
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
                {isMac
                  ? "Comes back in the menu bar on every login, so the hotkeys are live without opening anything. macOS lists it under System Settings → General → Login Items."
                  : "Comes back in the panel on every login, so the hotkeys are live without opening anything. Written as ~/.config/autostart/reze.desktop, which GNOME Tweaks lists under Startup Applications."}
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

          {isMac && (
            <p className="e-note">
              Reinstalling Reze replaces the app bundle and clears this, the same way it clears
              the Accessibility grant — tick it again after <code>make update</code>.
            </p>
          )}
        </>
      )}
    </section>
  );
}

/** `macOS`, `linux` and `windows`, the way Tailscale spells them, as names. */
function osName(os: string): string {
  const known: Record<string, string> = { macos: "macOS", linux: "Linux", windows: "Windows" };
  return known[os.toLowerCase()] ?? os;
}

const count = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** The first few of a list of triggers, and how many were left out. */
function some(names: string[], most = 4): string {
  if (names.length <= most) return names.join(", ");
  return `${names.slice(0, most).join(", ")} and ${names.length - most} more`;
}

/**
 * What a machine has to say for itself in the list. `revision` is what this
 * machine's library holds, when this machine syncs: with it, the line also
 * says where the two stand.
 */
function says(device: Device, revision: string | null): string {
  if (!device.online) return "offline";
  if (!device.reze) return "Reze is not answering";
  const { macros, attachments } = device.reze;
  const has =
    attachments > 0
      ? `${count(macros, "macro")} · ${count(attachments, "attachment")}`
      : count(macros, "macro");
  if (revision === null) return has;
  // Also what a Reze from before sync existed comes out as, which is fair:
  // it is not syncing, and updating it is how to switch it on.
  if (!device.reze.sync) return `${has} · sync is off there`;
  if (device.trouble) return `${has} · not syncing`;
  return `${has} · ${device.reze.revision === revision ? "in sync" : "syncing…"}`;
}

/**
 * Your other machines: keeping them the same, and importing from one.
 *
 * Tailscale owns the answer to "which machines are mine", so like the login
 * item it is read back rather than remembered: on opening, whenever the window
 * comes forward, and whenever the backend notices the tailnet come or go — or
 * a sync change where a machine stands.
 *
 * Importing is never one click. Choosing a machine fetches its library and
 * says what would happen — which macros are new, which would be overwritten,
 * which only this machine has — and the import is the button under that.
 * Sync is the opposite on purpose: one tick, and from then on nobody is asked.
 */
function DevicesSection({
  library,
  update,
  flush,
}: {
  library: Library;
  update: (next: Library) => void;
  flush: () => Promise<void>;
}) {
  // Null until the first answer, for the reason StartupSection starts at null.
  const [net, setNet] = useState<Tailnet | null>(null);
  const [looking, setLooking] = useState(false);
  // The machine whose import is open, and what importing from it would do.
  const [from, setFrom] = useState<Device | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [replace, setReplace] = useState(false);
  const [withSettings, setWithSettings] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<ImportOutcome | null>(null);
  const [undone, setUndone] = useState(false);

  // Each look takes seconds and several can be in flight — a focus, then the
  // tailnet changing under it. Only the last one asked gets to answer.
  const asked = useRef(0);
  const look = useCallback(() => {
    const mine = ++asked.current;
    setLooking(true);
    api
      .tailnetStatus()
      .then((next) => mine === asked.current && setNet(next))
      .catch(() => {})
      .finally(() => mine === asked.current && setLooking(false));
  }, []);

  useEffect(() => {
    look();
    window.addEventListener("focus", look);
    const changed = listen("tailnet-changed", look);
    return () => {
      window.removeEventListener("focus", look);
      changed.then((un) => un());
    };
  }, [look]);

  // The same guard for previews: clicking a second machine while the first is
  // still being read must not show the first one's library under its name.
  const previewing = useRef("");
  const choose = (device: Device) => {
    previewing.current = device.address;
    setFrom(device);
    setPreview(null);
    setReplace(false);
    setWithSettings(false);
    setError(null);
    setDone(null);
    setUndone(false);
    api
      .importPreview(device.address)
      .then((p) => previewing.current === device.address && setPreview(p))
      .catch((e) => previewing.current === device.address && setError(String(e)));
  };

  const close = () => {
    previewing.current = "";
    setFrom(null);
    setPreview(null);
    setError(null);
  };

  const run = async () => {
    if (!from) return;
    setBusy(true);
    setError(null);
    try {
      await flush();
      setDone(await api.importLibrary(from.address, replace, withSettings));
      previewing.current = "";
      setFrom(null);
      setPreview(null);
      look(); // its counts, and this machine's, may both have just changed
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  };

  const undo = async () => {
    setError(null);
    try {
      await flush();
      await api.undoImport();
      setDone(null);
      setUndone(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const s = library.settings;
  const running = net?.state === "running" ? net : null;
  // Sync is fetching both ways, so it needs this machine to be answering.
  const syncing = s.sync && s.shareOverTailscale;

  // With nothing only here, the two ways of importing come to the same macros.
  const choice = preview !== null && preview.onlyHere.length > 0;
  const idle =
    preview !== null &&
    preview.added.length === 0 &&
    preview.changed.length === 0 &&
    !(replace && choice) &&
    !withSettings;

  const did = done
    ? [
        done.added > 0 && `${done.added} added`,
        done.updated > 0 && `${done.updated} updated`,
        done.removed > 0 && `${done.removed} removed`,
        done.attachments > 0 && `${count(done.attachments, "file")} fetched`,
      ].filter(Boolean)
    : [];

  return (
    <section>
      <h2>Other devices</h2>
      <p>
        Keep one library on all your machines, or copy it across once — macros and their
        attachments, and for a copy the settings too if you want them. It travels over
        Tailscale, between machines signed in as you, with Reze running on both.
      </p>

      {net === null ? (
        <p className="e-note">Looking for Tailscale…</p>
      ) : net.state === "missing" ? (
        <p className="e-note">
          Tailscale is not installed on this machine. Install it here and on the other one, sign
          in to both as the same user, and your machines appear here.
        </p>
      ) : net.state === "stopped" ? (
        <p className="e-note">
          Tailscale is installed but not connected. Connect it and your other machines appear
          here.
        </p>
      ) : (
        <>
          <div className="e-row e-tailnet">
            <span className="e-device-dot" data-on="true" aria-hidden />
            <span>
              Tailscale is connected. This machine is <code>{net.name}</code>
            </span>
            <button onClick={look} disabled={looking}>
              {looking ? "Looking…" : "Look again"}
            </button>
          </div>

          <ul className="e-devices">
            {net.devices.map((device) => (
              <li key={device.address} data-reze={device.reze ? "true" : "false"}>
                <div className="e-device">
                  <span className="e-device-dot" data-on={device.reze ? "true" : "false"} aria-hidden />
                  <code>{device.name}</code>
                  <em>{osName(device.os)}</em>
                  <span className="e-device-says">
                    {says(device, syncing ? net.revision : null)}
                  </span>
                  {device.reze && from?.address !== device.address && (
                    <button onClick={() => choose(device)} disabled={busy}>
                      Import…
                    </button>
                  )}
                </div>

                {from?.address === device.address && (
                  <div className="e-import">
                    {preview === null ? (
                      !error && <p className="e-note">Reading {device.name}’s library…</p>
                    ) : (
                      <>
                        <ul className="e-import-facts">
                          <Fact n={preview.added.length} names={preview.added}>
                            new
                          </Fact>
                          <Fact n={preview.changed.length} names={preview.changed}>
                            different here — {device.name}’s version wins
                          </Fact>
                          <Fact n={preview.same}>already the same</Fact>
                          <Fact n={preview.onlyHere.length} names={preview.onlyHere}>
                            only on this machine
                          </Fact>
                        </ul>

                        {choice && (
                          <>
                            <label className="e-check">
                              <input
                                type="radio"
                                checked={!replace}
                                onChange={() => setReplace(false)}
                              />
                              <span>
                                Add and update
                                <em>
                                  Keeps the {count(preview.onlyHere.length, "macro")} only this
                                  machine has.
                                </em>
                              </span>
                            </label>
                            <label className="e-check">
                              <input
                                type="radio"
                                checked={replace}
                                onChange={() => setReplace(true)}
                              />
                              <span>
                                Replace this library
                                <em>
                                  Removes the {count(preview.onlyHere.length, "macro")} only this
                                  machine has, leaving a copy of {device.name}’s.
                                  {syncing && " With sync on, they go from your other machines too."}
                                </em>
                              </span>
                            </label>
                          </>
                        )}

                        <label className="e-check">
                          <input
                            type="checkbox"
                            checked={withSettings}
                            onChange={(e) => setWithSettings(e.target.checked)}
                          />
                          <span>
                            Bring its settings too
                            <em>
                              How expansions are delivered, whether typing is watched, and the
                              clipboard history’s switch and limit
                              {preview.hotkeys
                                ? " — and the three hotkeys."
                                : ". Not the hotkeys: macOS and Linux have different ones on purpose."}{" "}
                              Watched folders stay as they are, since they are paths on that
                              machine.
                            </em>
                          </span>
                        </label>

                        <div className="e-row">
                          <button
                            className={replace && choice ? "e-danger" : undefined}
                            onClick={run}
                            disabled={busy || idle}
                          >
                            {busy
                              ? "Importing…"
                              : replace && choice
                                ? "Replace this library"
                                : "Import"}
                          </button>
                          <button onClick={close} disabled={busy}>
                            Cancel
                          </button>
                          <span>
                            {idle
                              ? `Nothing to bring over — everything ${device.name} has is already here.`
                              : preview.attachments > 0
                                ? `${count(preview.attachments, "file")} to fetch · ${formatBytes(preview.bytes)}`
                                : ""}
                          </span>
                        </div>
                      </>
                    )}
                    {error && (
                      <>
                        <p className="e-warn">{error}</p>
                        {preview === null && (
                          <div className="e-row">
                            <button onClick={close}>Close</button>
                          </div>
                        )}
                      </>
                    )}
                  </div>
                )}

                {syncing && device.reze?.sync && device.trouble && (
                  <p className="e-warn">
                    Not syncing with {device.name}: {device.trouble}.
                  </p>
                )}
              </li>
            ))}
            {net.devices.length === 0 && (
              <li className="e-devices-empty">
                No other computers are signed in to this tailnet as you.
              </li>
            )}
          </ul>
        </>
      )}

      {done && (
        <div className="e-row e-imported">
          <span>
            Imported from <code>{done.from}</code>
            {did.length > 0 ? ` — ${did.join(", ")}.` : " — nothing needed changing."}
          </span>
          {/* Not while syncing: what an import brought is on the other machines
              by now, and putting this one back would only have them return it. */}
          {!syncing && <button onClick={undo}>Undo</button>}
        </div>
      )}
      {done && done.missing.length > 0 && (
        <p className="e-warn">
          {done.from} no longer has {count(done.missing.length, "attached file")} its library
          names, so {done.missing.length === 1 ? "it was" : "they were"} left off:{" "}
          {some(done.missing)}.
        </p>
      )}
      {done?.warnings.map((w) => (
        <p className="e-warn" key={w}>
          {w}
        </p>
      ))}
      {undone && <p className="e-note">Put back the way it was before the import.</p>}
      {!from && error && <p className="e-warn">{error}</p>}

      <label className="e-check" style={{ marginTop: 16 }}>
        <input
          type="checkbox"
          checked={syncing}
          onChange={(e) => {
            const on = e.target.checked;
            // The offer to undo an import does not outlive this: from here on
            // the newest thing set aside is the library as sync found it.
            if (on) setDone(null);
            update({
              ...library,
              settings: { ...s, sync: on, shareOverTailscale: on || s.shareOverTailscale },
            });
          }}
        />
        <span>
          Keep my devices in sync
          <em>
            A macro added, changed or deleted on one machine is added, changed or deleted on
            every other that has this ticked too — within seconds while both are awake, and
            when they next meet otherwise. Where two machines changed the same macro, the later
            change wins. Macros and their attachments only: settings, watched folders, the
            clipboard history and use counts stay each machine’s own. Ticking it sets the
            library aside in <code>~/.reze/backups</code> first.
          </em>
        </span>
      </label>

      <label className="e-check">
        <input
          type="checkbox"
          checked={s.shareOverTailscale}
          onChange={(e) =>
            update({
              ...library,
              settings: {
                ...s,
                shareOverTailscale: e.target.checked,
                sync: e.target.checked && s.sync,
              },
            })
          }
        />
        <span>
          Let my other devices import from this one
          <em>
            Reze listens on this machine’s Tailscale address
            {running ? ` (${running.address}, port ${running.port})` : ""} and nowhere else, and
            answers only devices signed in to Tailscale as you. It hands over the macros you
            wrote and their attachments — never the clipboard history, and nothing from watched
            folders. Off, nothing listens at all, and so nothing syncs.
          </em>
        </span>
      </label>
      {running?.problem && s.shareOverTailscale && <p className="e-warn">{running.problem}</p>}
    </section>
  );
}

/** One line of what an import would do: how many, what of, and which. */
function Fact({
  n,
  names,
  children,
}: {
  n: number;
  names?: string[];
  children: React.ReactNode;
}) {
  return (
    <li data-none={n === 0 ? "true" : "false"}>
      <strong>{n}</strong>
      <span>{children}</span>
      {names && names.length > 0 && <em>{some(names)}</em>}
    </li>
  );
}

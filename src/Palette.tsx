import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as api from "./lib/api";
import { fuzzyMatch } from "./lib/fuzzy";
import {
  builtinValues,
  builtinsUsed,
  collectVariables,
  expandIncludes,
  render,
  type Variable,
} from "./lib/template";
import { useLibrary } from "./lib/useLibrary";
import { AttachmentMedia, AttachmentTile, ClipIcon } from "./Attachments";
import type { Attachment, Grabbed, GrabTools, Macro } from "./lib/types";
import "./palette.css";

type Stage = "search" | "fill" | "naming";

type Ranked = { macro: Macro; indices: number[] };

/**
 * A whole query that is nothing but a web link.
 *
 * Anchored deliberately: a macro body can mention a URL, and a trigger could
 * contain one. Only a query that *is* a link means "download this", so the
 * search never gets hijacked by a word that happens to look like an address.
 */
const LINK = /^https?:\/\/\S+$/i;

export default function Palette() {
  const { library } = useLibrary();
  const macros = library?.macros ?? [];

  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [stage, setStage] = useState<Stage>("search");
  const [active, setActive] = useState<Macro | null>(null);
  const [expanded, setExpanded] = useState("");
  const [vars, setVars] = useState<Variable[]>([]);
  const [values, setValues] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  // The download flow: a link pasted into the search field, the wait, and the
  // chance to name what came back before it becomes a macro.
  const [grabbing, setGrabbing] = useState(false);
  const [grabbed, setGrabbed] = useState<Grabbed | null>(null);
  const [grabName, setGrabName] = useState("");
  const [tools, setTools] = useState<GrabTools | null>(null);

  // Set when an in-place expansion is in flight: the characters either side of
  // the trigger that Rust selected, which must be put back around the
  // expansion. A ref rather than state because `choose` reads it in the same
  // tick it is set.
  const affixRef = useRef<{ head: string; tail: string } | null>(null);

  const rootRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const firstFieldRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  const reset = useCallback(() => {
    affixRef.current = null;
    setQuery("");
    setSelected(0);
    setStage("search");
    setActive(null);
    setValues({});
    setError(null);
    setGrabbed(null);
    setGrabName("");
    requestAnimationFrame(() => searchRef.current?.focus());
  }, []);

  useEffect(() => {
    searchRef.current?.focus();
    const un = listen("palette-opened", reset);
    return () => {
      un.then((f) => f());
    };
  }, [reset]);

  // Keep the native window exactly as tall as the content, so the blurred
  // surface never extends past what is drawn.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const sync = () => api.resizePalette(el.getBoundingClientRect().height).catch(() => {});
    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const results = useMemo<Ranked[]>(() => {
    if (!query.trim()) {
      return [...macros]
        .sort((a, b) => b.usageCount - a.usageCount || a.name.localeCompare(b.name))
        .map((macro) => ({ macro, indices: [] }));
    }
    const scored: (Ranked & { score: number })[] = [];
    for (const macro of macros) {
      const onName = fuzzyMatch(query, macro.name);
      if (onName) {
        scored.push({ macro, indices: onName.indices, score: onName.score + 25 });
        continue;
      }
      // Fall back to tags and description so you can find a macro by what it is
      // for, not only by what you named it.
      const haystack = [...macro.tags, macro.description].join(" ");
      const loose = fuzzyMatch(query, haystack);
      if (loose) scored.push({ macro, indices: [], score: loose.score - 20 });
    }
    return scored
      .sort((a, b) => b.score - a.score || b.macro.usageCount - a.macro.usageCount)
      .map(({ macro, indices }) => ({ macro, indices }));
  }, [query, macros]);

  useEffect(() => setSelected(0), [query]);

  useEffect(() => {
    listRef.current
      ?.querySelector('[data-selected="true"]')
      ?.scrollIntoView({ block: "nearest" });
  }, [selected, results]);

  const current = results[selected]?.macro ?? null;

  const send = useCallback(
    async (body: string, copyOnly: boolean, macro?: Macro) => {
      // An in-place expansion replaces a whole selection, so the text either
      // side of the trigger goes back exactly as it was.
      const affix = affixRef.current;
      const text = affix ? affix.head + body + affix.tail : body;
      const files = macro?.attachments ?? [];
      try {
        // "Copy only" is either an explicit ⌘↵ or the global preference.
        await api.deliver(text, copyOnly || library?.settings.pasteMode === "copy", files);
        if (macro) api.bumpUsage(macro.id).catch(() => {});
        reset();
      } catch (e) {
        // The one failure worth explaining in place: without Accessibility the
        // keystroke is swallowed by the OS and nothing at all happens. It also
        // has to be re-granted after every update, which is otherwise baffling.
        setError(
          String(e) === "accessibility-denied"
            ? "Accessibility permission is missing — macOS grants it per build, so a new version needs it again. Open the editor to fix. ⌘↵ still copies."
            : String(e),
        );
      }
    },
    [reset, library],
  );

  const choose = useCallback(
    async (macro: Macro, copyOnly: boolean) => {
      const { text } = expandIncludes(macro.body, macros);
      const needed = collectVariables(text);
      if (needed.length > 0) {
        setActive(macro);
        setExpanded(text);
        setVars(needed);
        setValues(Object.fromEntries(needed.map((v) => [v.name, v.default])));
        setStage("fill");
        requestAnimationFrame(() => firstFieldRef.current?.select());
        return;
      }
      const clipboard = builtinsUsed(text).includes("clipboard")
        ? await api.readClipboard().catch(() => "")
        : "";
      const filled = render(text, builtinValues(builtinsUsed(text), clipboard));
      await send(filled, copyOnly, macro);
    },
    [macros, send],
  );

  const submitFilled = useCallback(
    async (copyOnly: boolean) => {
      if (!active) return;
      const clipboard = builtinsUsed(expanded).includes("clipboard")
        ? await api.readClipboard().catch(() => "")
        : "";
      const filled = render(expanded, {
        ...values,
        ...builtinValues(builtinsUsed(expanded), clipboard),
      });
      await send(filled, copyOnly, active);
    },
    [active, expanded, values, send],
  );

  const link = query.trim();
  const isLink = LINK.test(link);

  // Checked only once a link is actually in the box, so the ordinary path never
  // pays for it — and re-checked each time, since installing a tool while the
  // app runs should not need a restart.
  useEffect(() => {
    if (!isLink || tools) return;
    api.grabTools().then(setTools).catch(() => {});
  }, [isLink, tools]);

  const startGrab = useCallback(async () => {
    setError(null);
    setGrabbing(true);
    try {
      const result = await api.grabUrl(link);
      setGrabbed(result);
      setGrabName(result.suggestedName);
      setStage("naming");
      // Selected rather than merely focused: the offered name is usually right,
      // so typing should replace it and Enter alone should accept it.
      requestAnimationFrame(() => nameRef.current?.select());
    } catch (e) {
      setError(String(e));
    } finally {
      setGrabbing(false);
    }
  }, [link]);

  /** Accept the name and close. The file is already a macro either way. */
  const submitName = useCallback(async () => {
    if (!grabbed) return;
    const name = grabName.trim();
    if (name && name !== grabbed.suggestedName) {
      try {
        await api.renameGrab(grabbed.path, name);
      } catch (e) {
        setError(String(e));
        return; // stay, so the name can be fixed rather than silently lost
      }
    }
    reset();
    api.hidePalette();
  }, [grabbed, grabName, reset]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      // Leaving during a download abandons the naming step, not the download —
      // it finishes in the background and keeps the name it was given.
      if (stage === "naming" || grabbing) {
        reset();
        api.hidePalette();
      } else if (stage === "fill") {
        // Backing out of an in-place expansion abandons it — the caret is
        // already back in the other app.
        affixRef.current = null;
        setStage("search");
        setActive(null);
        requestAnimationFrame(() => searchRef.current?.focus());
      } else {
        api.hidePalette();
      }
      return;
    }

    if (stage === "naming") {
      if (e.key === "Enter") {
        e.preventDefault();
        submitName();
      }
      return;
    }

    if (stage === "fill") {
      if (e.key === "Enter") {
        e.preventDefault();
        submitFilled(e.metaKey || e.ctrlKey);
      }
      return;
    }

    // A link in the box means one action, so the list keys have nothing to move
    // through and Enter means download rather than expand.
    if (isLink) {
      if (e.key === "Enter" && !grabbing) {
        e.preventDefault();
        startGrab();
      }
      return;
    }

    const move = (delta: number) => {
      e.preventDefault();
      setSelected((s) => {
        if (results.length === 0) return 0;
        return (s + delta + results.length) % results.length;
      });
    };

    if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) move(1);
    else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) move(-1);
    else if (e.key === "Enter" && current) {
      e.preventDefault();
      choose(current, e.metaKey || e.ctrlKey);
    }
  };

  // Expand-in-place: Rust has already selected the trigger words in the other
  // app and matched a macro; the template engine lives here, so it finishes the
  // job. The window stays hidden unless values still need filling in.
  useEffect(() => {
    const selected = listen<{ id: string; head: string; tail: string }>(
      "expand-selected",
      async (e) => {
        const macro = macros.find((m) => m.id === e.payload.id);
        if (!macro) return;
        affixRef.current = { head: e.payload.head, tail: e.payload.tail };
        const { text } = expandIncludes(macro.body, macros);
        if (collectVariables(text).length > 0) await api.showPalette().catch(() => {});
        await choose(macro, false);
      },
    );

    const failed = listen<string>("expand-failed", (e) => {
      // "no macro matched" is an ordinary miss and stays quiet. A missing
      // permission is not — nothing will ever work until it is granted.
      if (e.payload !== "accessibility-denied") return;
      setError(
        "Accessibility permission is missing — macOS grants it per build, so a new version needs it again. Open the editor to fix.",
      );
      api.showPalette().catch(() => {});
    });

    return () => {
      selected.then((un) => un());
      failed.then((un) => un());
    };
  }, [macros, choose]);

  const preview = useMemo(() => {
    if (stage === "fill") return render(expanded, values);
    if (!current) return "";
    return expandIncludes(current.body, macros).text;
  }, [stage, expanded, values, current, macros]);

  // Whichever macro the preview pane is describing, so its files show alongside.
  const shown = stage === "fill" ? active : current;

  return (
    <div className="palette" ref={rootRef} onKeyDown={onKeyDown}>
      {stage === "naming" ? (
        <div className="p-search p-search--fill">
          <span className="p-back">↓</span>
          <span className="p-fill-title">Downloaded</span>
        </div>
      ) : stage === "search" ? (
        <div className="p-search">
          <SearchIcon />
          <input
            ref={searchRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search macros…"
            spellCheck={false}
            autoComplete="off"
          />
          {query && !isLink && <kbd className="p-count">{results.length}</kbd>}
        </div>
      ) : (
        <div className="p-search p-search--fill">
          <span className="p-back">↩</span>
          <span className="p-fill-title">{active?.name}</span>
        </div>
      )}

      {error && <div className="p-error">{error}</div>}

      {stage === "search" && isLink && (
        <GrabPanel link={link} grabbing={grabbing} tools={tools} />
      )}

      {stage === "naming" && grabbed && (
        <div className="p-body p-body--naming">
          <div className="p-name-fields">
            <label>
              <span>Call it</span>
              <input
                ref={nameRef}
                value={grabName}
                onChange={(e) => setGrabName(e.target.value)}
                spellCheck={false}
                autoComplete="off"
              />
            </label>
            <p className="p-name-note">
              This renames the file, which is what the macro is named after.
            </p>
          </div>
          <div className="p-preview">
            <div className="p-name-preview">
              <AttachmentMedia attachment={grabbed.attachment} />
            </div>
          </div>
        </div>
      )}

      {stage === "search" && !isLink && results.length === 0 && (
        <div className="p-empty">
          {macros.length === 0 ? "No macros yet." : "Nothing matches."}
          <button onClick={() => api.openEditor()}>Open editor</button>
        </div>
      )}

      {stage === "search" && !isLink && results.length > 0 && (
        <div className="p-body">
          <ul className="p-list" ref={listRef}>
            {results.map(({ macro, indices }, i) => (
              <li
                key={macro.id}
                data-selected={i === selected}
                onMouseMove={() => setSelected(i)}
                onClick={() => choose(macro, false)}
              >
                <span className="p-name">
                  <Highlight text={macro.name} indices={indices} />
                </span>
                {(macro.tags.length > 0 || macro.attachments.length > 0) && (
                  <span className="p-tags">
                    {macro.attachments.length > 0 && (
                      <span className="p-clip">
                        <ClipIcon />
                        {macro.attachments.length}
                      </span>
                    )}
                    {macro.tags.map((t) => `#${t}`).join(" ")}
                  </span>
                )}
              </li>
            ))}
          </ul>
          <div className="p-preview">
            {current?.description && <div className="p-desc">{current.description}</div>}
            <MediaStrip attachments={shown?.attachments ?? []} />
            {preview && <pre>{preview}</pre>}
          </div>
        </div>
      )}

      {stage === "fill" && (
        <div className="p-body">
          <div className="p-fields">
            {vars.map((v, i) => (
              <label key={v.name}>
                <span>{v.name}</span>
                <input
                  ref={i === 0 ? firstFieldRef : undefined}
                  value={values[v.name] ?? ""}
                  placeholder={v.default || "…"}
                  onChange={(e) => setValues((prev) => ({ ...prev, [v.name]: e.target.value }))}
                  spellCheck={false}
                />
              </label>
            ))}
          </div>
          <div className="p-preview">
            <MediaStrip attachments={shown?.attachments ?? []} />
            {preview && <pre>{preview}</pre>}
          </div>
        </div>
      )}

      <div className="p-footer">
        {stage === "naming" ? (
          <>
            <span>
              <kbd>↵</kbd> save
            </span>
            <span>
              <kbd>esc</kbd> keep this name
            </span>
          </>
        ) : isLink ? (
          <span>
            <kbd>↵</kbd> {grabbing ? "downloading…" : "download"}
          </span>
        ) : (
          <>
            <span>
              <kbd>↵</kbd> {stage === "fill" ? "paste" : "select"}
            </span>
            <span>
              <kbd>⌘↵</kbd> {shown?.attachments.length && !preview ? "copy file" : "copy only"}
            </span>
            <span>
              <kbd>esc</kbd> {stage === "fill" ? "back" : "close"}
            </span>
          </>
        )}
        <span className="p-spacer" />
        <button className="p-link" onClick={() => api.openEditor()}>
          Edit macros
        </button>
      </div>
    </div>
  );
}

/**
 * The files a macro will paste.
 *
 * Above the body rather than after it: a body runs to hundreds of words and the
 * pane scrolls, so anything below it is invisible until you scroll — and the
 * files are the part you cannot infer from the trigger name. Thumbnails rather
 * than full previews for the same reason: this is a chooser, and one band of
 * them still leaves the body visible underneath.
 *
 * Read-only and non-interactive: the palette closes the instant it loses focus,
 * so a video you could scrub would be a video you could never finish scrubbing.
 */
/**
 * What the palette shows once the query turns out to be a link.
 *
 * The missing-tool warning is up front rather than after a failed download: the
 * check is instant and the download is not, and being told thirty seconds in
 * that nothing was ever going to work is the worse order.
 */
function GrabPanel({
  link,
  grabbing,
  tools,
}: {
  link: string;
  grabbing: boolean;
  tools: GrabTools | null;
}) {
  let host = link;
  try {
    host = new URL(link).host.replace(/^www\./, "");
  } catch {
    // Not worth surfacing — the field still shows the link itself below.
  }

  return (
    <div className="p-grab">
      <div className="p-grab-main">
        <span className="p-grab-title">
          {grabbing ? `Downloading from ${host}…` : `Download from ${host}`}
        </span>
        <span className="p-grab-url">{link}</span>
      </div>

      {grabbing && (
        <div className="p-grab-bar" aria-hidden>
          <span />
        </div>
      )}

      {!grabbing && tools && !tools.ytDlp && !tools.galleryDl && (
        <p className="p-grab-warn">
          Needs <code>yt-dlp</code> for video and <code>gallery-dl</code> for images —
          <code>brew install yt-dlp gallery-dl</code>
        </p>
      )}
    </div>
  );
}

function MediaStrip({ attachments }: { attachments: Attachment[] }) {
  if (attachments.length === 0) return null;
  return (
    <div className="p-media">
      {attachments.map((a) => (
        <AttachmentTile key={a.id} attachment={a} />
      ))}
    </div>
  );
}

function Highlight({ text, indices }: { text: string; indices: number[] }) {
  if (indices.length === 0) return <>{text}</>;
  const set = new Set(indices);
  return (
    <>
      {[...text].map((ch, i) =>
        set.has(i) ? (
          <mark key={i}>{ch}</mark>
        ) : (
          <span key={i}>{ch}</span>
        ),
      )}
    </>
  );
}

function SearchIcon() {
  return (
    <svg viewBox="0 0 20 20" className="p-icon" aria-hidden>
      <circle cx="9" cy="9" r="6" fill="none" stroke="currentColor" strokeWidth="1.8" />
      <path d="M13.5 13.5 L17 17" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
    </svg>
  );
}

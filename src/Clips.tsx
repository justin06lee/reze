import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as api from "./lib/api";
import { clipKindLabel, clipWhen, formatBytes, type ClipEntry } from "./lib/types";
import "./clips.css";

/**
 * The clipboard history, drawn.
 *
 * The list is the whole feature, so the row does the work: what it was, what it
 * came from, and when. The source app is the part that makes a wall of similar
 * snippets readable — three near-identical URLs are three different things once
 * one is badged Safari and two are badged Slack.
 */

/** Everything remembered, kept in step with the recorder. */
export function useClips(): ClipEntry[] {
  const [clips, setClips] = useState<ClipEntry[]>([]);

  useEffect(() => {
    let alive = true;
    api
      .clipboardHistory()
      .then((list) => alive && setClips(list))
      .catch(() => {});

    const changed = listen<ClipEntry[]>("clipboard-changed", (e) => setClips(e.payload));
    return () => {
      alive = false;
      changed.then((un) => un());
    };
  }, []);

  return clips;
}

/**
 * An `asset://` URL, resolved once.
 *
 * Both the images and the app icons come off disk rather than through the IPC
 * bridge — a screenshot is megabytes, and the same six app icons are wanted by
 * every row on screen.
 */
function useClipFile(file: string | undefined, icon: boolean): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!file) {
      setUrl(null);
      return;
    }
    let alive = true;
    const resolve = icon ? api.clipIconUrl(file) : api.clipUrl(file);
    resolve.then((u) => alive && setUrl(u)).catch(() => {});
    return () => {
      alive = false;
    };
  }, [file, icon]);
  return url;
}

/**
 * The thumbnail, with the source app badged onto its corner — the same shape
 * macOS uses in its own clipboard list, and for the same reason: the badge says
 * where it came from without costing the row a line of text.
 */
export function ClipThumb({ entry }: { entry: ClipEntry }) {
  const image = useClipFile(entry.kind === "image" ? entry.file : undefined, false);
  const icon = useClipFile(entry.source?.icon, true);
  const [broken, setBroken] = useState(false);

  return (
    <div className="c-thumb">
      {image && !broken ? (
        <img
          className="c-shot"
          src={image}
          alt={entry.preview || "Copied image"}
          draggable={false}
          onError={() => setBroken(true)}
        />
      ) : (
        <PageGlyph entry={entry} />
      )}
      {icon && (
        <img className="c-badge" src={icon} alt={entry.source?.name ?? ""} draggable={false} />
      )}
    </div>
  );
}

/** A sheet of paper, for everything with no picture of its own. */
function PageGlyph({ entry }: { entry: ClipEntry }) {
  const ext =
    entry.kind === "file" && entry.preview.includes(".")
      ? entry.preview.split(".").pop()!.slice(0, 4).toUpperCase()
      : null;
  return (
    <div className="c-page">
      <svg viewBox="0 0 24 30" aria-hidden>
        <path
          d="M4 1.5h11l5 5v22a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-26a1 1 0 0 1 1-1Z"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.4"
        />
        <path d="M15 1.5v5h5" fill="none" stroke="currentColor" strokeWidth="1.4" />
      </svg>
      {ext && <span className="c-ext">{ext}</span>}
    </div>
  );
}

/** One row of the list. */
export function ClipRow({ entry }: { entry: ClipEntry }) {
  const title = entry.preview.trim() || (entry.kind === "image" ? "Image" : "Empty");
  const from = entry.source?.name;
  return (
    <>
      <ClipThumb entry={entry} />
      <span className="c-lines">
        <span className="c-title">{title}</span>
        <span className="c-meta">
          {clipKindLabel(entry)}
          {from ? ` · ${from}` : ""} · Copied {clipWhen(entry.copiedAt)}
        </span>
      </span>
    </>
  );
}

/**
 * The pane beside the list: the whole thing, as far as it was kept.
 *
 * Text arrives truncated — the backend keeps the rest — so the pane says so
 * rather than quietly showing a fragment as if it were the entry.
 */
export function ClipPreview({ entry }: { entry: ClipEntry }) {
  const image = useClipFile(entry.kind === "image" ? entry.file : undefined, false);
  // Counted in code points, like the backend did: `.length` is UTF-16 units,
  // and an entry full of emoji would otherwise report a negative remainder.
  const hidden = entry.chars - [...entry.preview].length;

  return (
    <>
      <div className="c-head">
        {entry.source?.name && <span className="c-from">{entry.source.name}</span>}
        <span className="c-when">
          {clipKindLabel(entry)} · {formatBytes(entry.bytes)} · {clipWhen(entry.copiedAt)}
        </span>
      </div>

      {entry.kind === "image" && image && (
        <img className="c-full" src={image} alt={entry.preview || "Copied image"} />
      )}

      {entry.kind === "file" && (
        <ul className="c-paths">
          {(entry.paths ?? []).map((path) => (
            <li key={path}>{path}</li>
          ))}
        </ul>
      )}

      {entry.kind === "text" && <pre>{entry.preview}</pre>}
      {hidden > 0 && (
        <p className="c-more">
          … and {hidden.toLocaleString()} more characters — all of it gets pasted.
        </p>
      )}
    </>
  );
}

/** A page with a corner turned, for the clipboard mode's search field. */
export function ClipboardIcon() {
  return (
    <svg viewBox="0 0 20 20" className="p-icon" aria-hidden>
      <rect
        x="4"
        y="3.5"
        width="12"
        height="14"
        rx="2"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <path
        d="M7.5 1.8h5a.7.7 0 0 1 .7.7v1.3a.7.7 0 0 1-.7.7h-5a.7.7 0 0 1-.7-.7V2.5a.7.7 0 0 1 .7-.7Z"
        fill="currentColor"
      />
      <path
        d="M7 9.5h6M7 12.5h4"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

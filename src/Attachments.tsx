import { useEffect, useState } from "react";
import * as api from "./lib/api";
import { formatBytes, type Attachment } from "./lib/types";
import "./attachments.css";

/**
 * Attachment previews, shared by both windows.
 *
 * Files are drawn straight off disk over the `asset://` protocol rather than
 * read into memory as data URLs: a twelve-megabyte screen recording has no
 * business travelling through the IPC bridge, and the protocol serves byte
 * ranges, which is what makes a video seekable at all.
 */

/** The asset URL for `attachment`, once it has been resolved. */
export function useAttachmentUrl(attachment: Attachment): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    api
      .attachmentUrl(attachment)
      .then((u) => alive && setUrl(u))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [attachment.file, attachment.path]);
  return url;
}

/**
 * How far into a video to seek for its thumbnail.
 *
 * A `<video>` with no `poster` draws an empty grey rectangle until something
 * decodes a frame, and `preload="metadata"` alone never does. Asking for a
 * timestamp makes WebKit fetch and paint that frame, which is the whole
 * thumbnail. Slightly past zero because the very first frame of a recording is
 * often a black lead-in.
 */
const POSTER_AT = "#t=0.1";

/**
 * The attachment itself, drawn according to its kind.
 *
 * `playable` decides whether it can be scrubbed — the palette closes the moment
 * it loses focus, so controls there would be a trap. Either way it seeks to
 * [`POSTER_AT`] so you can see what the file is without playing it.
 */
export function AttachmentMedia({
  attachment,
  playable = false,
}: {
  attachment: Attachment;
  playable?: boolean;
}) {
  const url = useAttachmentUrl(attachment);
  const [broken, setBroken] = useState(false);

  if (!url || broken) {
    return <FileGlyph attachment={attachment} missing={broken} />;
  }

  switch (attachment.kind) {
    case "image":
      return (
        <img
          className="a-media"
          src={url}
          alt={attachment.name}
          draggable={false}
          onError={() => setBroken(true)}
        />
      );
    case "video":
      return playable ? (
        <video
          className="a-media"
          src={`${url}${POSTER_AT}`}
          controls
          preload="metadata"
          onError={() => setBroken(true)}
        />
      ) : (
        <div className="a-video-still">
          <video
            className="a-media"
            src={`${url}${POSTER_AT}`}
            preload="metadata"
            muted
            onError={() => setBroken(true)}
          />
          <span className="a-play" aria-hidden>
            ▶
          </span>
        </div>
      );
    case "audio":
      return playable ? (
        <audio className="a-audio" src={url} controls preload="metadata" />
      ) : (
        <FileGlyph attachment={attachment} />
      );
    default:
      return <FileGlyph attachment={attachment} />;
  }
}

/**
 * Stand-in for anything with no visual form — a PDF, a zip, or a file whose
 * bytes have gone missing since the macro was made.
 */
function FileGlyph({ attachment, missing = false }: { attachment: Attachment; missing?: boolean }) {
  const ext = attachment.name.includes(".")
    ? attachment.name.split(".").pop()!.slice(0, 5).toUpperCase()
    : "FILE";
  return (
    <div className="a-glyph" data-missing={missing || undefined}>
      <span className="a-ext">{missing ? "?" : ext}</span>
    </div>
  );
}

/**
 * One attachment as a tile, with its name underneath — the editor's view.
 *
 * `onRemove` is what makes it editable; without it the tile is inert, which is
 * what the palette wants.
 */
export function AttachmentTile({
  attachment,
  onRemove,
}: {
  attachment: Attachment;
  onRemove?: () => void;
}) {
  return (
    <figure className="a-tile" title={`${attachment.name} · ${formatBytes(attachment.bytes)}`}>
      <div className="a-frame">
        <AttachmentMedia attachment={attachment} playable={Boolean(onRemove)} />
        {onRemove && (
          <button className="a-remove" onClick={onRemove} title="Remove" aria-label="Remove">
            ×
          </button>
        )}
      </div>
      <figcaption>
        <span className="a-name">{attachment.name}</span>
        <span className="a-size">{formatBytes(attachment.bytes)}</span>
      </figcaption>
    </figure>
  );
}

/** Paperclip, for the "this macro carries files" marker in a list row. */
export function ClipIcon() {
  return (
    <svg viewBox="0 0 16 16" className="a-clip" aria-hidden>
      <path
        d="M10.6 4.3 5.5 9.4a1.6 1.6 0 0 0 2.3 2.3l5.4-5.4a3 3 0 0 0-4.3-4.3L3.2 7.7a4.5 4.5 0 0 0 6.4 6.4l4.5-4.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

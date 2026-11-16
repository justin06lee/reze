import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getLibrary } from "./api";
import type { Library } from "./types";

/**
 * The library, kept in sync with the file on disk.
 *
 * The JSON file is the source of truth — both windows and any text editor the
 * user has open are just views onto it, so every change arrives through the
 * same `library-changed` event regardless of who made it.
 *
 * `arriving` sees each library on its way in and may hand back another in its
 * place. The editor uses it to keep what has been typed and not yet saved from
 * being wiped by a library that was written before it was.
 */
export function useLibrary(arriving?: (incoming: Library) => Library) {
  const [library, setLibrary] = useState<Library | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Through a ref, so that a caller passing a new function every render does
  // not resubscribe — and refetch — every render.
  const through = useRef(arriving);
  useEffect(() => {
    through.current = arriving;
  });

  useEffect(() => {
    let alive = true;
    const take = (incoming: Library) => setLibrary(through.current?.(incoming) ?? incoming);
    getLibrary()
      .then((lib) => alive && take(lib))
      .catch((e) => alive && setError(String(e)));

    const changed = listen<Library>("library-changed", (e) => take(e.payload));
    const broken = listen<string>("library-error", (e) => setError(e.payload));

    return () => {
      alive = false;
      changed.then((un) => un());
      broken.then((un) => un());
    };
  }, []);

  return { library, setLibrary, error, setError };
}

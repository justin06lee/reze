import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getLibrary } from "./api";
import type { Library } from "./types";

/**
 * The library, kept in sync with the file on disk.
 *
 * The JSON file is the source of truth — both windows and any text editor the
 * user has open are just views onto it, so every change arrives through the
 * same `library-changed` event regardless of who made it.
 */
export function useLibrary() {
  const [library, setLibrary] = useState<Library | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    getLibrary()
      .then((lib) => alive && setLibrary(lib))
      .catch((e) => alive && setError(String(e)));

    const changed = listen<Library>("library-changed", (e) => setLibrary(e.payload));
    const broken = listen<string>("library-error", (e) => setError(e.payload));

    return () => {
      alive = false;
      changed.then((un) => un());
      broken.then((un) => un());
    };
  }, []);

  return { library, setLibrary, error, setError };
}

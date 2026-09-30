/**
 * Where the app is running, for the handful of things that are said or shown
 * differently: the modifier key's name, and what the file manager is called.
 * Behaviour is decided in Rust; this is only ever about words.
 */
export const isMac = navigator.userAgent.includes("Mac");

/** The primary shortcut modifier as a key cap: ⌘ on macOS, Ctrl elsewhere. */
export const MOD = isMac ? "⌘" : "Ctrl+";

/** What "show this in the file manager" is called here. */
export const REVEAL = isMac ? "Reveal in Finder" : "Show in Files";

/** How the download tools get installed here — see `INSTALL` in grab.rs. */
export const INSTALL = isMac ? "brew install" : "pipx install";

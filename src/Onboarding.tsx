import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as api from "./lib/api";
import { prettyHotkey } from "./lib/hotkey";
import { isMac } from "./lib/platform";
import type { Device, Library, LoginItemState, Settings, Tailnet } from "./lib/types";
import "./onboarding.css";

type Step = "welcome" | "access" | "devices" | "startup";

/** The steps there are on this machine, in order. */
const STEPS: Step[] = isMac
  ? ["welcome", "access", "devices", "startup"]
  : // Nothing to grant on Linux: X11 lets any client send a keystroke.
    ["welcome", "devices", "startup"];

const os = (name: string) =>
  ({ macos: "macOS", linux: "Linux", windows: "Windows" })[name.toLowerCase()] ?? name;

/** The other machines a Reze is answering on. */
const answering = (net: Tailnet | null): Device[] =>
  net?.state === "running" ? net.devices.filter((d) => d.reze) : [];

/**
 * The welcome: what the editor opens on until it has been gone through once.
 *
 * Four things, and only the ones that cannot be found out by using the app:
 * the three keys that open it, the permission without which nothing pastes,
 * the other machines it could be keeping in step with, and whether it starts
 * by itself. Everything here is also in Settings — this only puts in one
 * place what would otherwise be discovered one disappointment at a time.
 *
 * Nothing is saved until the end. Leaving halfway leaves the library as it
 * was, and the welcome waiting for next time.
 */
export default function Onboarding({
  library,
  update,
}: {
  library: Library;
  update: (next: Library) => void;
}) {
  const s = library.settings;
  const [step, setStep] = useState<Step>("welcome");
  const [trusted, setTrusted] = useState<boolean | null>(null);
  const [net, setNet] = useState<Tailnet | null>(null);
  // Null until chosen: until then it follows what the tailnet turns out to hold.
  const [sync, setSync] = useState<boolean | null>(null);
  const [login, setLogin] = useState<LoginItemState | null>(null);
  const [loginError, setLoginError] = useState<string | null>(null);

  const at = STEPS.indexOf(step);
  const last = at === STEPS.length - 1;

  // Asked for at once rather than on reaching the step: the answer takes a
  // couple of seconds, which is about how long the first step takes to read.
  const asked = useRef(0);
  const look = useCallback(() => {
    const mine = ++asked.current;
    api
      .tailnetStatus()
      .then((next) => mine === asked.current && setNet(next))
      .catch(() => mine === asked.current && setNet({ state: "stopped" }));
  }, []);

  useEffect(() => {
    look();
    const changed = listen("tailnet-changed", look);
    return () => {
      changed.then((un) => un());
    };
  }, [look]);

  // The grant is given in System Settings, in another window, so it is watched
  // for while this step is up rather than read once.
  useEffect(() => {
    if (step !== "access") return;
    const check = () => api.accessibilityStatus().then(setTrusted).catch(() => {});
    check();
    const timer = window.setInterval(check, 1000);
    return () => clearInterval(timer);
  }, [step]);

  const readLogin = useCallback(
    () => api.loginItemStatus().then(setLogin).catch(() => {}),
    [],
  );
  useEffect(() => {
    if (step === "startup") readLogin();
  }, [step, readLogin]);

  const setLoginItem = async (on: boolean) => {
    setLoginError(null);
    try {
      await api.setLoginItem(on);
    } catch (e) {
      setLoginError(String(e));
    }
    readLogin();
  };

  const others = answering(net);
  // A machine of yours that already syncs is a library waiting to be joined;
  // that, or sync being on here already, is what ticks the box unasked.
  const wantsSync = sync ?? (s.sync || others.some((d) => d.reze?.sync));

  const finish = (settings: Partial<Settings>) =>
    update({ ...library, settings: { ...s, ...settings, onboarded: true } });

  const done = () =>
    finish(
      net?.state === "running"
        ? { sync: wantsSync, shareOverTailscale: wantsSync || s.shareOverTailscale }
        : {},
    );

  return (
    <div className="ob">
      <div className="ob-card">
        <ol className="ob-steps" aria-label={`Step ${at + 1} of ${STEPS.length}`}>
          {STEPS.map((name, i) => (
            <li key={name} data-state={i < at ? "done" : i === at ? "now" : "next"} />
          ))}
        </ol>

        {step === "welcome" && (
          <>
            <h1>Short triggers, full prompts.</h1>
            <p>
              Reze keeps the things you type again and again — prompts, snippets, the images
              that go with them — and puts them into whatever app you are in. It lives in the
              menu bar; these three keys are how you reach it.
            </p>
            <ul className="ob-keys">
              <li>
                <kbd>{prettyHotkey(s.hotkey)}</kbd>
                <span>
                  The palette
                  <em>Type a trigger, press Enter. It is pasted where your cursor was.</em>
                </span>
              </li>
              <li>
                <kbd>{prettyHotkey(s.expandHotkey)}</kbd>
                <span>
                  Expand in place
                  <em>Type the trigger right where you are, then this. No window at all.</em>
                </span>
              </li>
              <li>
                <kbd>{prettyHotkey(s.clipboardHotkey)}</kbd>
                <span>
                  Clipboard history
                  <em>Everything you copied, with the app you copied it from.</em>
                </span>
              </li>
            </ul>
          </>
        )}

        {step === "access" && (
          <>
            <h1>Let Reze paste for you.</h1>
            <p>
              Pasting into another app means pressing {"⌘V"} on your behalf, and macOS only
              lets an app do that with Accessibility access. Without it the palette opens and
              nothing is inserted.
            </p>
            <div className="ob-status" data-ok={trusted ? "true" : "false"}>
              <span className="ob-dot" aria-hidden />
              <span>
                {trusted === null
                  ? "Checking…"
                  : trusted
                    ? "Access granted. Nothing more to do here."
                    : "Not granted yet."}
              </span>
              {trusted === false && (
                <>
                  <button onClick={() => api.requestAccessibility()}>Ask macOS</button>
                  <button onClick={() => api.openAccessibilitySettings()}>Open Settings</button>
                </>
              )}
            </div>
            {trusted === false && (
              <p className="ob-note">
                In System Settings → Privacy &amp; Security → Accessibility, switch Reze on.
                This page notices by itself when you have.
              </p>
            )}
          </>
        )}

        {step === "devices" && (
          <>
            <h1>One library on all your machines.</h1>
            {net === null ? (
              <p>Looking for Tailscale…</p>
            ) : net.state !== "running" ? (
              <>
                <p>
                  With <a onClick={() => openUrl("https://tailscale.com/download")}>Tailscale</a>{" "}
                  on your machines, Reze keeps the same macros on all of them: add one here and
                  it is on the others in seconds.
                </p>
                <p className="ob-note">
                  {net.state === "missing"
                    ? "Tailscale is not installed on this machine, so there is nothing to set up now."
                    : "Tailscale is installed here but not connected, so there is nothing to set up now."}{" "}
                  Settings → Other devices has the switch whenever you want it.
                </p>
              </>
            ) : (
              <>
                <p>
                  Tailscale is connected, and this machine is <code>{net.name}</code>.{" "}
                  {others.length === 0
                    ? "No other machine of yours has a Reze answering yet."
                    : others.length === 1
                      ? "One other machine of yours has a Reze answering:"
                      : `${others.length} other machines of yours have a Reze answering:`}
                </p>
                {others.length > 0 && (
                  <ul className="ob-devices">
                    {others.map((d) => (
                      <li key={d.address}>
                        <span className="ob-dot" data-ok="true" aria-hidden />
                        <code>{d.name}</code>
                        <em>{os(d.os)}</em>
                        <span>
                          {d.reze!.macros} {d.reze!.macros === 1 ? "macro" : "macros"}
                          {d.reze!.sync ? " · syncing" : ""}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
                <label className="e-check">
                  <input
                    type="checkbox"
                    checked={wantsSync}
                    onChange={(e) => setSync(e.target.checked)}
                  />
                  <span>
                    Keep my devices in sync
                    <em>
                      A macro added, changed or deleted on one machine is added, changed or
                      deleted on every other that has this on too. Macros and their attachments
                      only — settings and the clipboard history stay each machine’s own.{" "}
                      {others.length === 0
                        ? "Your other machines join when you switch it on there."
                        : "Nothing is lost by joining: the two libraries are merged, and this one is set aside first."}
                    </em>
                  </span>
                </label>
              </>
            )}
          </>
        )}

        {step === "startup" && (
          <>
            <h1>There when you log in.</h1>
            <p>
              Reze only works while it is running. Started at login it sits in the menu bar
              and stays out of the way; close this window and it is still there.
            </p>
            {login === "unsupported" ? (
              <p className="ob-note">
                Starting at login is not available from this build. Settings → Startup says
                why.
              </p>
            ) : (
              <label className="e-check">
                <input
                  type="checkbox"
                  checked={login === "enabled"}
                  disabled={login === null}
                  onChange={(e) => setLoginItem(e.target.checked)}
                />
                <span>
                  Start Reze at login
                  <em>
                    {login === "requiresApproval"
                      ? "Registered, but switched off for Reze in System Settings → Login Items."
                      : "You can change this at any time in Settings → Startup."}
                  </em>
                </span>
              </label>
            )}
            {loginError && <p className="e-warn">{loginError}</p>}
          </>
        )}

        <footer className="ob-foot">
          {at > 0 ? (
            <button onClick={() => setStep(STEPS[at - 1])}>Back</button>
          ) : (
            <button className="ob-quiet" onClick={() => finish({})}>
              Skip the tour
            </button>
          )}
          <span />
          <button className="ob-go" onClick={() => (last ? done() : setStep(STEPS[at + 1]))}>
            {last
              ? "Start using Reze"
              : step === "access" && trusted === false
                ? "Later"
                : "Continue"}
          </button>
        </footer>
      </div>
    </div>
  );
}

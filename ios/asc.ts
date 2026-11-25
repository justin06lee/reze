// What getting the phone app onto TestFlight needs from App Store Connect,
// through its API. `ios/testflight.sh` runs these in order; each can be run
// alone:
//
//   bun ios/asc.ts ids        register the two identifiers, with App Groups on
//   bun ios/asc.ts check      say what is still missing, and who has to do it
//   bun ios/asc.ts profiles   make and install the two App Store profiles
//   bun ios/asc.ts testers    an internal TestFlight group with the account holder in it
//   bun ios/asc.ts status     the builds App Store Connect has
//   bun ios/asc.ts wait <n>   until build <n> is processed
//
// The key (`~/.appstoreconnect/private_keys/AuthKey_<id>.p8`) is never in
// this repo. Two things the API will not do, and a person signed in to
// Apple's sites has to, once — `check` says so when they are missing:
//
//   - make the App Group and give it to both identifiers
//     (developer.apple.com → Identifiers)
//   - make the app's record (appstoreconnect.apple.com → Apps → +)

import { execFileSync } from "node:child_process";
import { createHash, createPrivateKey, sign } from "node:crypto";
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const APP = "com.justin06lee.reze.ios";
const KEYBOARD = "com.justin06lee.reze.ios.keyboard";
const GROUP = "group.com.justin06lee.reze";
/** Each identifier, what to call it, and the App Store profile it is signed with. */
export const TARGETS = [
  { identifier: APP, name: "Reze", profile: "Reze App Store" },
  { identifier: KEYBOARD, name: "Reze Keyboard", profile: "Reze Keyboard App Store" },
];
const TESTERS = "Justin";

const keyId = process.env.ASC_KEY_ID || "Y688NH26RQ";
const issuer = process.env.ASC_ISSUER_ID || "fd10fb1e-e9c4-4570-bda4-e93f2e10c8ff";
const keyPath = process.env.ASC_KEY_PATH || join(homedir(), ".appstoreconnect/private_keys", `AuthKey_${keyId}.p8`);
/** The keychain holding the team's Apple Distribution key, shared by its apps. */
const KEYCHAIN = process.env.SIGNING_KEYCHAIN || join(homedir(), "Library/Keychains/midna-signing.keychain-db");
const PROFILES = join(homedir(), "Library/Developer/Xcode/UserData/Provisioning Profiles");

function jwt(): string {
  const b64 = (v: unknown) => Buffer.from(JSON.stringify(v)).toString("base64url");
  const now = Math.floor(Date.now() / 1000);
  const unsigned = `${b64({ alg: "ES256", kid: keyId, typ: "JWT" })}.${b64({ iss: issuer, iat: now, exp: now + 600, aud: "appstoreconnect-v1" })}`;
  const key = createPrivateKey(readFileSync(keyPath, "utf8"));
  return `${unsigned}.${sign("sha256", Buffer.from(unsigned), { key, dsaEncoding: "ieee-p1363" }).toString("base64url")}`;
}

async function asc(method: string, path: string, body?: unknown): Promise<any> {
  const res = await fetch(`https://api.appstoreconnect.apple.com${path}`, {
    method,
    headers: { authorization: `Bearer ${jwt()}`, ...(body ? { "content-type": "application/json" } : {}) },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${text.slice(0, 400)}`);
  return text ? JSON.parse(text) : {};
}

async function bundle(identifier: string): Promise<any | null> {
  const found = await asc("GET", `/v1/bundleIds?filter[identifier]=${identifier}&limit=50`);
  return found.data.find((b: any) => b.attributes.identifier === identifier) ?? null;
}

async function record(): Promise<any | null> {
  const found = await asc("GET", `/v1/apps?filter[bundleId]=${APP}&fields[apps]=name,bundleId`);
  return found.data.find((a: any) => a.attributes.bundleId === APP) ?? null;
}

/** Register the identifiers and turn App Groups on for each. */
async function ids() {
  for (const target of TARGETS) {
    let b = await bundle(target.identifier);
    if (!b) {
      b = (await asc("POST", "/v1/bundleIds", {
        data: { type: "bundleIds", attributes: { identifier: target.identifier, name: target.name, platform: "IOS" } },
      })).data;
      console.log(`registered ${target.identifier}`);
    }
    const on = (await asc("GET", `/v1/bundleIds/${b.id}/bundleIdCapabilities`)).data.map((c: any) => c.attributes.capabilityType);
    if (!on.includes("APP_GROUPS")) {
      await asc("POST", "/v1/bundleIdCapabilities", {
        data: { type: "bundleIdCapabilities", attributes: { capabilityType: "APP_GROUPS" }, relationships: { bundleId: { data: { type: "bundleIds", id: b.id } } } },
      });
      console.log(`turned App Groups on for ${target.identifier}`);
    }
  }
}

/** The SHA-1 of the Apple Distribution identity in the signing keychain. */
function identity(): string {
  const listed = execFileSync("security", ["find-identity", "-v", "-p", "codesigning", KEYCHAIN]).toString();
  const sha = listed.split("\n").find((l) => l.includes("Apple Distribution"))?.trim().split(/\s+/)[1];
  if (!sha) throw new Error(`no Apple Distribution identity in ${KEYCHAIN} (is it unlocked?)`);
  return sha;
}

/** The groups a profile lets what it signs into, read from the profile itself. */
function groupsIn(profileContent: string): string[] {
  const plist = Buffer.from(profileContent, "base64").toString("latin1");
  const at = plist.indexOf("<key>com.apple.security.application-groups</key>");
  if (at < 0) return [];
  const list = plist.slice(at, plist.indexOf("</array>", at));
  return [...list.matchAll(/<string>([^<]*)<\/string>/g)].map((m) => m[1]);
}

/**
 * Make the App Store profile for each identifier and install it. A profile
 * cannot be edited, so one that exists is deleted and made again — which is
 * also what picks up an App Group given to the identifier since.
 */
async function profiles() {
  const sha = identity();
  const certs = await asc("GET", "/v1/certificates?filter[certificateType]=DISTRIBUTION&limit=20");
  const cert = certs.data.find(
    (c: any) => createHash("sha1").update(Buffer.from(c.attributes.certificateContent, "base64")).digest("hex").toUpperCase() === sha,
  );
  if (!cert) throw new Error(`the certificate for ${sha.slice(0, 8)}… is not on the team`);

  mkdirSync(PROFILES, { recursive: true });
  let lacking = false;
  for (const target of TARGETS) {
    const b = await bundle(target.identifier);
    if (!b) throw new Error(`${target.identifier} is not registered: bun ios/asc.ts ids`);
    const old = (await asc("GET", `/v1/profiles?filter[name]=${encodeURIComponent(target.profile)}&limit=10`)).data;
    for (const p of old.filter((p: any) => p.attributes.name === target.profile)) await asc("DELETE", `/v1/profiles/${p.id}`);
    const made = (await asc("POST", "/v1/profiles", {
      data: {
        type: "profiles",
        attributes: { name: target.profile, profileType: "IOS_APP_STORE" },
        relationships: { bundleId: { data: { type: "bundleIds", id: b.id } }, certificates: { data: [{ type: "certificates", id: cert.id }] } },
      },
    })).data;

    // The old one's copy goes, so nothing signs with it by name.
    for (const f of readdirSync(PROFILES).filter((f) => f.endsWith(".mobileprovision"))) {
      const plist = execFileSync("security", ["cms", "-D", "-i", join(PROFILES, f)], { stdio: ["ignore", "pipe", "ignore"] }).toString();
      if (/<key>Name<\/key>\s*<string>([^<]*)<\/string>/.exec(plist)?.[1] === target.profile) rmSync(join(PROFILES, f));
    }
    writeFileSync(join(PROFILES, `${made.attributes.uuid}.mobileprovision`), Buffer.from(made.attributes.profileContent, "base64"));

    const has = groupsIn(made.attributes.profileContent).includes(GROUP);
    lacking ||= !has;
    console.log(`"${target.profile}" installed${has ? "" : `, but WITHOUT ${GROUP}`}`);
  }
  if (lacking) {
    console.error(`\n${GROUP} has not been given to both identifiers yet, so these profiles cannot sign the app.`);
    console.error(HOW_GROUP);
    process.exit(1);
  }
}

const HOW_GROUP = `At https://developer.apple.com/account/resources/identifiers/list (signed in as the account holder):
  1. Identifiers → + → App Groups → description "Reze", identifier ${GROUP} → Register.
  2. Identifiers → ${APP} → App Groups → Configure → tick it → Save.
  3. The same for ${KEYBOARD}.
Then: bun ios/asc.ts profiles`;

const HOW_RECORD = `At https://appstoreconnect.apple.com/apps (signed in as the account holder): + → New App →
  platform iOS, a name nobody else has (for instance "Reze Macros"), bundle ID ${APP},
  SKU reze-ios, full access.`;

/** Say what is missing before a build can be uploaded. Exits non-zero if anything is. */
async function check() {
  let missing = 0;
  for (const target of TARGETS) {
    if (!(await bundle(target.identifier))) {
      missing++;
      console.log(`✗ ${target.identifier} is not registered: bun ios/asc.ts ids`);
    } else console.log(`✓ ${target.identifier} is registered`);
  }
  // Whether the group was given to an identifier shows nowhere in the API
  // but in a profile made for it, so that is what is looked at.
  for (const target of TARGETS) {
    const p = (await asc("GET", `/v1/profiles?filter[name]=${encodeURIComponent(target.profile)}&limit=10`)).data
      .find((p: any) => p.attributes.name === target.profile);
    if (p && groupsIn(p.attributes.profileContent).includes(GROUP)) console.log(`✓ "${target.profile}" carries ${GROUP}`);
    else {
      missing++;
      console.log(`✗ "${target.profile}" ${p ? `does not carry ${GROUP}` : "has not been made"}`);
    }
  }
  const app = await record();
  if (app) console.log(`✓ the app's record exists: "${app.attributes.name}" (${app.id})`);
  else {
    missing++;
    console.log(`✗ no record for ${APP} in App Store Connect.\n${HOW_RECORD}`);
  }
  if (missing) process.exit(1);
}

/** An internal TestFlight group that gets every build, with the account holder in it. */
async function testers() {
  const app = await record();
  if (!app) throw new Error(`no App Store Connect record for ${APP} yet`);
  const groups = await asc("GET", `/v1/apps/${app.id}/betaGroups?fields[betaGroups]=name,isInternalGroup`);
  let group = groups.data.find((g: any) => g.attributes.name === TESTERS && g.attributes.isInternalGroup);
  if (!group) {
    group = (await asc("POST", "/v1/betaGroups", {
      data: { type: "betaGroups", attributes: { name: TESTERS, isInternalGroup: true, hasAccessToAllBuilds: true }, relationships: { app: { data: { type: "apps", id: app.id } } } },
    })).data;
    console.log(`made the internal group "${TESTERS}"`);
  }
  const users = await asc("GET", "/v1/users?fields[users]=username,roles,firstName&limit=50");
  const holder = users.data.find((u: any) => u.attributes.roles.includes("ACCOUNT_HOLDER"));
  const email: string | undefined = holder?.attributes.username;
  if (!email) throw new Error("no account holder found on the team");
  const inGroup = await asc("GET", `/v1/betaGroups/${group.id}/betaTesters?fields[betaTesters]=email&limit=50`);
  if (inGroup.data.some((t: any) => t.attributes.email?.toLowerCase() === email.toLowerCase())) {
    console.log(`the account holder is in "${TESTERS}"`);
    return;
  }
  // A tester's record is per app: one from another app cannot be linked to
  // this group, but one made with the group attached can.
  await asc("POST", "/v1/betaTesters", {
    data: { type: "betaTesters", attributes: { email, firstName: holder.attributes.firstName ?? "Justin" }, relationships: { betaGroups: { data: [{ type: "betaGroups", id: group.id }] } } },
  });
  console.log(`added the account holder to "${TESTERS}"`);
}

async function builds(): Promise<any> {
  const app = await record();
  if (!app) return { data: [] };
  return asc(
    "GET",
    `/v1/builds?filter[app]=${app.id}&sort=-uploadedDate&limit=10&fields[builds]=version,processingState,uploadedDate,buildBetaDetail&include=buildBetaDetail&fields[buildBetaDetails]=internalBuildState`,
  );
}

async function status() {
  const found = await builds();
  if (!found.data.length) return console.log("no builds");
  const detail = new Map((found.included ?? []).map((d: any) => [d.id, d.attributes]));
  for (const b of found.data) {
    const d: any = detail.get(b.relationships?.buildBetaDetail?.data?.id) ?? {};
    console.log(`build ${b.attributes.version}: ${b.attributes.processingState}, TestFlight: ${d.internalBuildState ?? "?"}, uploaded ${b.attributes.uploadedDate}`);
  }
}

async function wait(version: string | undefined) {
  if (!version) throw new Error("which build? bun ios/asc.ts wait <number>");
  for (let i = 0; i < 80; i++) {
    const found = (await builds()).data.find((b: any) => b.attributes.version === version);
    const state = found?.attributes.processingState ?? "not visible yet";
    console.log(`${new Date().toTimeString().slice(0, 8)} build ${version}: ${state}`);
    if (state === "VALID") return;
    if (state === "INVALID" || state === "FAILED") throw new Error(`App Store Connect refused build ${version}: ${state}`);
    await new Promise((done) => setTimeout(done, 30_000));
  }
  throw new Error("still processing after forty minutes");
}

const [command, argument] = process.argv.slice(2);
const run = { ids, check, profiles, testers, status, wait: () => wait(argument), identity: async () => console.log(identity()) }[command ?? ""];
if (!run) {
  console.error("usage: bun ios/asc.ts ids|check|profiles|testers|status|wait <build>|identity");
  process.exit(2);
}
await run().catch((error: Error) => {
  console.error(error.message);
  process.exit(1);
});

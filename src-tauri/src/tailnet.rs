//! Your other machines, found over Tailscale.
//!
//! A library is a file, and getting a file from one machine you own to another
//! should not take a cloud account or a USB stick. Tailscale already gives each
//! of your devices an address only your tailnet can reach, encrypts everything
//! sent to it, and can say who is on the other end of a connection. That is
//! all this needs, so nothing here invents discovery, encryption or identity of
//! its own.
//!
//! Every Reze answers on its Tailscale address, and another Reze asks it for
//! its library. Three things keep that small enough to be on by default:
//!
//! - **It listens on the tailnet address and nowhere else.** Not on every
//!   interface: nothing on the local network or the internet can so much as
//!   open a connection.
//! - **It answers only your own devices.** Each connection is put to
//!   `tailscale whois`, and anything not signed in as the same user as this
//!   machine — a colleague on a shared tailnet, a node shared in from someone
//!   else's, a tagged server — is refused whatever it asks for.
//! - **It only hands things out.** No request changes anything here. Importing
//!   is something the *other* machine does, to itself, with what it was given.
//!
//! What it hands out is `macros.json` and the attachment files that names.
//! Never the clipboard history, and never anything from a watched folder.
//!
//! Keeping machines the same — the last section here, deciding with
//! [`crate::sync`] — is that conversation held without being asked, and under
//! the same three rules. Each machine with sync on fetches the others'
//! libraries and changes itself; nothing is ever pushed. What looks like a
//! push, a macro saved here turning up there a second later, is this machine
//! saying there is something to fetch (`/poke`) and that one fetching it.
//!
//! The conversation is plain HTTP — four GETs — so that `curl` can take
//! either side of it when something needs debugging. It is written out by hand
//! at both ends rather than pulled in as a server and a client: one side of
//! each exchange is always this same file, so "HTTP" here only has to mean the
//! twenty lines both ends agree on.
//!
//! Tailscale itself is driven through its CLI, the way [`crate::grab`] drives
//! the downloaders, and for the same reason: it is the interface that is the
//! same on every platform and that Tailscale keeps working.

use crate::assets::Attachment;
use crate::store::Library;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;

/// Where every Reze listens: R-E-Z-E on a phone keypad.
pub const PORT: u16 = 7393;

/// Which version of the conversation this speaks. Raised if an answer ever
/// changes shape, so that two builds which disagree say so instead of guessing.
const PROTOCOL: u32 = 1;

/// How long Tailscale gets to answer its CLI. It is a call to a local socket
/// and returns in milliseconds; this only stops a wedged daemon wedging us.
const CLI_TIMEOUT: Duration = Duration::from_secs(5);

/// How often to look at whether this machine is on a tailnet.
const CHECK: Duration = Duration::from_secs(10);

/// How long to leave it after failing to start listening — the port taken, or
/// Tailscale not yet agreeing the address is its own.
const RETRY: Duration = Duration::from_secs(60);

/// How often to look for a connection: seldom when nothing is happening, and
/// often for a few seconds after one, since an import is a run of requests one
/// after another and each would otherwise wait out a quiet poll.
const QUIET_POLL: Duration = Duration::from_millis(250);
const BUSY_POLL: Duration = Duration::from_millis(8);
const BUSY_FOR: Duration = Duration::from_secs(3);

/// How long to wait on a machine when listing devices. Short, because every
/// machine that is asleep costs this in full and the list waits for the slowest.
const PROBE: Duration = Duration::from_millis(2500);

/// How long a connection may go without a byte moving before it is given up
/// on. Not a limit on a transfer — a large video takes as long as it takes.
const STALL: Duration = Duration::from_secs(20);

/// How long a request has to arrive in full once its connection is open.
const REQUEST: Duration = Duration::from_secs(5);

/// How long an answer about who an address belongs to is believed.
const REMEMBER: Duration = Duration::from_secs(60);

/// The most request, or response head, worth reading.
const MAX_HEAD: u64 = 8 * 1024;

/// The largest library worth reading into memory. Text only — attachments are
/// streamed to disk — so anything near this is not a library.
const MAX_LIBRARY: u64 = 64 * 1024 * 1024;

/// How many connections to be answering at once. An import asks for one thing
/// at a time, so this is only ever reached by something misbehaving.
const MAX_CONNECTIONS: usize = 16;

/// How often to compare notes with the other machines when nothing prompts
/// it. What prompts it — a save here, a word from there — is what makes sync
/// quick; this is what makes it certain, for the machine that was asleep or
/// off the tailnet when the word was sent.
const SYNC_EVERY: Duration = Duration::from_secs(60);

/// How long to wait once prompted. A save is rarely alone — typing is one
/// every half second — and one round can carry all of them.
const SETTLE: Duration = Duration::from_millis(750);

/// How soon to look again after telling a machine there is something to
/// fetch: long enough for it to have done so, so that Settings can say the
/// two agree instead of leaving them a minute apart on the screen.
const FOLLOW_UP: Duration = Duration::from_secs(3);

/// How many times to start a sync over because the library here changed
/// while its files were on their way.
const ATTEMPTS: usize = 4;

// ---------------------------------------------------------------- tailscale

/// The Tailscale app's own binary, which doubles as its CLI.
///
/// Preferred over one on `PATH`: it is by construction the same version as the
/// service it talks to, where a Homebrew `tailscale` installed alongside the
/// app drifts from it.
#[cfg(target_os = "macos")]
const APP_CLI: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";

fn cli() -> Option<String> {
    #[cfg(target_os = "macos")]
    if Path::new(APP_CLI).is_file() {
        return Some(APP_CLI.to_string());
    }
    crate::grab::which("tailscale")
}

/// The CLI, about to be asked `args`.
fn ask(cli: &str, args: &[&str]) -> Command {
    let mut command = Command::new(cli);
    command
        .args(args)
        // On macOS the app's binary is the GUI and the CLI both, and it decides
        // which to be from how it was started. Started by an app that launchd
        // started — which is what Reze is — it takes itself for the GUI, finds
        // one already running, and prints an apology where the JSON should be.
        // This is Tailscale's own switch for saying which was meant. A machine
        // that happens to have it set for every app hides the problem
        // entirely, which is how it went unnoticed on the one this was written
        // on. Nothing reads it on Linux.
        .env("TAILSCALE_BE_CLI", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

/// Run the CLI and return what it printed, or `None` if it failed.
///
/// Standard output only: a CLI that is not the same version as its daemon says
/// so on standard error, on every call, ahead of perfectly good JSON.
fn tailscale(cli: &str, args: &[&str]) -> Option<Vec<u8>> {
    let mut child = ask(cli, args).spawn().ok()?;

    // Drained on a thread of its own. The status of a tailnet with more than a
    // few machines is larger than a pipe holds, and a child blocked writing to
    // a full pipe never exits to be waited for.
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > CLI_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    };
    let out = reader.join().ok()?;
    status.success().then_some(out)
}

/// The parts of `tailscale status --json` this reads.
#[derive(Deserialize)]
struct RawStatus {
    #[serde(rename = "BackendState", default)]
    backend_state: String,
    #[serde(rename = "Self", default)]
    this: Option<RawNode>,
    #[serde(rename = "Peer", default)]
    peers: Option<HashMap<String, RawNode>>,
}

#[derive(Deserialize)]
struct RawNode {
    #[serde(rename = "HostName", default)]
    host_name: String,
    #[serde(rename = "DNSName", default)]
    dns_name: String,
    #[serde(rename = "OS", default)]
    os: String,
    #[serde(rename = "UserID", default)]
    user: i64,
    #[serde(rename = "TailscaleIPs", default)]
    addresses: Option<Vec<String>>,
    #[serde(rename = "Online", default)]
    online: bool,
    #[serde(rename = "Tags", default)]
    tags: Option<Vec<String>>,
}

impl RawNode {
    /// What to call this machine: the first label of its MagicDNS name, which
    /// is unique on the tailnet and is what you would type to reach it. The
    /// hostname is neither — every iPhone reports `localhost`.
    fn label(&self) -> String {
        match self.dns_name.split('.').next().map(str::trim) {
            Some(label) if !label.is_empty() => label.to_string(),
            _ => self.host_name.clone(),
        }
    }

    /// Its IPv4 address. Every node has one unless the tailnet's policy turns
    /// IPv4 off, which is rare enough that such a node is simply not seen.
    fn address(&self) -> Option<Ipv4Addr> {
        self.addresses.iter().flatten().find_map(|a| a.parse().ok())
    }

    fn tagged(&self) -> bool {
        self.tags.as_ref().is_some_and(|tags| !tags.is_empty())
    }
}

/// Operating systems there is no Reze for, and so no point asking.
const NO_REZE: [&str; 3] = ["ios", "android", "tvos"];

/// This machine's place on its tailnet.
#[derive(Debug)]
struct Net {
    name: String,
    address: Ipv4Addr,
    /// Who this machine is signed in as — what "your devices" means.
    user: i64,
    /// The other machines signed in as the same user.
    peers: Vec<Peer>,
}

#[derive(Debug)]
struct Peer {
    name: String,
    address: Ipv4Addr,
    os: String,
    online: bool,
}

/// Read a status. `None` unless Tailscale is up and connected.
fn net_from(raw: &[u8]) -> Option<Net> {
    let status: RawStatus = serde_json::from_slice(raw).ok()?;
    // Stopped, logged out and still-starting all carry a `Self` too, with
    // addresses nothing can be reached on yet.
    if status.backend_state != "Running" {
        return None;
    }
    let this = status.this?;
    let address = this.address()?;

    let mut peers: Vec<Peer> = status
        .peers
        .unwrap_or_default()
        .into_values()
        // The same test the listening side applies to whoever connects — see
        // `owned` — so that everything listed here is something that would
        // answer, and nothing else is offered.
        .filter(|p| p.user == this.user && !p.tagged())
        .filter(|p| !NO_REZE.contains(&p.os.to_lowercase().as_str()))
        .filter_map(|p| {
            Some(Peer {
                name: p.label(),
                address: p.address()?,
                online: p.online,
                os: p.os,
            })
        })
        .collect();
    // Machines that can be asked first; within that, by name, so the list
    // does not reshuffle between two looks at it.
    peers.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.name.cmp(&b.name)));

    Some(Net {
        name: this.label(),
        address,
        user: this.user,
        peers,
    })
}

fn net_via(cli: &str) -> Option<Net> {
    net_from(&tailscale(cli, &["status", "--json"])?)
}

/// Whether a `tailscale whois` answer describes one of `user`'s own machines.
///
/// A tagged node belongs to the tailnet rather than to a person, whoever set
/// it up, so it is never one of "yours".
fn owned(raw: &[u8], user: i64) -> bool {
    #[derive(Deserialize)]
    struct Whois {
        #[serde(rename = "Node")]
        node: Node,
    }
    #[derive(Deserialize)]
    struct Node {
        #[serde(rename = "User")]
        user: i64,
        #[serde(rename = "Tags", default)]
        tags: Option<Vec<String>>,
    }
    serde_json::from_slice::<Whois>(raw)
        .is_ok_and(|w| w.node.user == user && w.node.tags.is_none_or(|tags| tags.is_empty()))
}

/// Decides whether a connecting address is one of this user's machines.
///
/// The address can be believed because of where the listener is: bound to the
/// tailnet address, it is only reached through Tailscale, which does not
/// deliver a packet whose source is not the node that encrypted it.
struct Gate {
    cli: String,
    user: i64,
    seen: Mutex<HashMap<IpAddr, (bool, Instant)>>,
}

impl Gate {
    fn allows(&self, peer: IpAddr) -> bool {
        if let Some(&(allowed, at)) = self.seen.lock().unwrap().get(&peer) {
            if at.elapsed() < REMEMBER {
                return allowed;
            }
        }
        // Anything short of Tailscale saying yes is a no: the CLI gone, the
        // daemon not answering, an address it has never heard of.
        let allowed = tailscale(&self.cli, &["whois", "--json", &peer.to_string()])
            .is_some_and(|raw| owned(&raw, self.user));
        // Remembered either way. An import is dozens of connections from one
        // address, and each would otherwise cost a process.
        self.seen.lock().unwrap().insert(peer, (allowed, Instant::now()));
        allowed
    }
}

/// 100.64.0.0/10 — the carrier-grade NAT range every tailnet is numbered from.
fn is_tailnet(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && b & 0xC0 == 64
}

/// This machine's Tailscale address, as the routing table has it.
///
/// Asking the CLI costs a process, and this is asked every few seconds for as
/// long as the app runs. Connecting a UDP socket sends nothing: it only makes
/// the kernel pick a route, and with the route the address it would send from.
/// Tailscale's own resolver sits at 100.100.100.100 and is only ever routed
/// through the tailnet, so the address chosen for it is this machine's own —
/// or, with Tailscale down, some other interface's, which the range check
/// throws out.
///
/// A hint and no more. A phone hotspot can hand out an address from the same
/// range, which is why [`open`] has Tailscale confirm it before listening.
fn routed_address() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(100, 100, 100, 100), 53)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if is_tailnet(ip) => Some(ip),
        _ => None,
    }
}

// ------------------------------------------------------------------ serving

/// What is on offer: the library file, and the directory its attachments are
/// in. Passed in rather than looked up so the tests can serve a scratch one.
#[cfg_attr(test, derive(Clone))]
struct Shelf {
    library: PathBuf,
    assets: PathBuf,
}

impl Shelf {
    fn ours() -> Self {
        Self {
            library: crate::store::library_path(),
            assets: crate::assets::assets_dir(),
        }
    }

    fn read(&self) -> Option<Library> {
        serde_json::from_slice(&std::fs::read(&self.library).ok()?).ok()
    }

    /// Replace the library on it — which only a sync does, and only to ours.
    fn write(&self, library: &Library) -> Result<(), String> {
        match self.library == crate::store::library_path() {
            // Through the store, which remembers what it wrote so that the
            // file watcher does not take it for a hand edit.
            true => crate::store::save(library).map(drop),
            false => crate::store::write(&self.library, library).map(drop),
        }
    }

    fn revision(&self) -> Option<String> {
        self.read().map(|library| crate::sync::revision(&library))
    }

    /// Whether an attachment's file is here.
    fn has(&self, attachment: &Attachment) -> bool {
        crate::assets::bare_name(&self.assets, &attachment.file).is_ok_and(|path| path.exists())
    }
}

/// What a machine says about itself when asked whether Reze is there.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol: u32,
    /// `macos` or `linux`. Decides whether hotkeys can come along.
    #[serde(default)]
    pub os: String,
    /// How much is there, so the list can say before anything is fetched.
    #[serde(default)]
    pub macros: usize,
    #[serde(default)]
    pub attachments: usize,
    /// Whether it keeps itself the same as the machines that ask this — which
    /// takes both saying so. Absent from a build that cannot, and so false.
    #[serde(default)]
    pub sync: bool,
    /// What its library holds — see [`crate::sync::revision`]. Two machines
    /// with the same one have nothing to fetch from each other.
    #[serde(default)]
    pub revision: String,
}

enum Reply {
    Json(Vec<u8>),
    File(std::fs::File, u64),
    Error(u16, &'static str),
}

/// Answer a request for `path`. The whole of what a connection can ask for.
fn answer(path: &str, shelf: &Shelf) -> Reply {
    const UNREADABLE: Reply = Reply::Error(500, "the library here could not be read");
    match path {
        "/reze" => {
            let Some(library) = shelf.read() else {
                return UNREADABLE;
            };
            let hello = Hello {
                protocol: PROTOCOL,
                os: std::env::consts::OS.to_string(),
                macros: library.macros.len(),
                attachments: library.macros.iter().map(|m| m.attachments.len()).sum(),
                sync: library.settings.syncs(),
                revision: crate::sync::revision(&library),
            };
            Reply::Json(serde_json::to_vec(&hello).unwrap_or_default())
        }
        // "There is something here for you." Nothing is taken on the word of
        // whoever says it: all it does is bring forward the look this machine
        // would have taken within the minute anyway, at the machines it would
        // have looked at anyway.
        "/poke" => {
            sync_soon();
            Reply::Json(b"{}".to_vec())
        }
        // The file as it is, rather than parsed and written out again: it is
        // the source of truth, and the other side reads it the way this one
        // does. Folder macros are never in it, so there is nothing to strip.
        "/library" => match std::fs::read(&shelf.library) {
            Ok(raw) => Reply::Json(raw),
            Err(_) => UNREADABLE,
        },
        _ => match path.strip_prefix("/assets/") {
            Some(name) => asset(&decode(name), shelf),
            None => Reply::Error(404, "nothing here by that name"),
        },
    }
}

fn asset(name: &str, shelf: &Shelf) -> Reply {
    // The boundary the library itself is held to: a bare file name or nothing.
    let Ok(path) = crate::assets::bare_name(&shelf.assets, name) else {
        return Reply::Error(400, "that is not a file name");
    };
    // And only a file a macro names. The directory also holds whatever is
    // waiting to be swept, which was removed from the library on purpose.
    let named = shelf.read().is_some_and(|library| {
        library
            .macros
            .iter()
            .flat_map(|m| &m.attachments)
            .any(|a| a.path.is_none() && a.file == name)
    });
    if !named {
        return Reply::Error(404, "no macro here has that file");
    }
    match std::fs::File::open(&path).and_then(|f| f.metadata().map(|m| (f, m))) {
        Ok((file, meta)) if meta.is_file() => Reply::File(file, meta.len()),
        _ => Reply::Error(404, "that file is gone"),
    }
}

/// The path a request asks for, or `None` if it is not one of ours.
fn read_request(stream: &TcpStream) -> Option<String> {
    let mut lines = BufReader::new(stream.take(MAX_HEAD)).lines();
    let first = lines.next()?.ok()?;
    // The rest is headers. None of them changes the answer, but they are read
    // to the end: closing a socket with part of a request still unread in it
    // resets the connection, and the other side sees that, not the reply.
    for line in lines {
        if line.ok()?.is_empty() {
            break;
        }
    }
    let mut words = first.split_whitespace();
    if words.next()? != "GET" {
        return None;
    }
    let target = words.next()?;
    Some(target.split(['?', '#']).next().unwrap_or(target).to_string())
}

fn send(stream: &mut TcpStream, reply: Reply) -> std::io::Result<()> {
    let (code, kind, length) = match &reply {
        Reply::Json(body) => (200, "application/json", body.len() as u64),
        Reply::File(_, length) => (200, "application/octet-stream", *length),
        Reply::Error(code, why) => (*code, "text/plain; charset=utf-8", why.len() as u64),
    };
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\n\
         Content-Type: {kind}\r\n\
         Content-Length: {length}\r\n\
         Connection: close\r\n\r\n"
    )?;
    match reply {
        Reply::Json(body) => stream.write_all(&body)?,
        // Exactly as many bytes as were promised, even if the file grew since.
        Reply::File(file, length) => {
            std::io::copy(&mut file.take(length), stream)?;
        }
        Reply::Error(_, why) => stream.write_all(why.as_bytes())?,
    }
    stream.flush()
}

/// Answer one connection, then close it.
///
/// `allowed` is the gate's verdict on who connected. It is decided before the
/// request is looked at and nothing in the request can change it: someone who
/// is not this user is told so whatever they asked for, including for things
/// that do not exist.
fn serve(mut stream: TcpStream, allowed: bool, shelf: &Shelf) {
    // An accepted socket inherits the listener's non-blocking mode on macOS
    // and does not on Linux, so say which is wanted rather than find out.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(REQUEST));
    let _ = stream.set_write_timeout(Some(STALL));

    let request = read_request(&stream);
    let reply = match (allowed, request) {
        (false, _) => Reply::Error(403, "this is not one of that user's devices"),
        (true, None) => Reply::Error(400, "not a request Reze understands"),
        (true, Some(path)) => answer(&path, shelf),
    };
    let _ = send(&mut stream, reply);
}

// ---------------------------------------------------------------- listening

/// Whether to answer at all. Mirrored from settings, the way the clipboard
/// history mirrors its own switch, so the loop below never reads the disk.
static ENABLED: AtomicBool = AtomicBool::new(true);

/// Whether something is actually listening right now, for Settings to show.
static LISTENING: AtomicBool = AtomicBool::new(false);

/// Why nothing is listening although something should be, if that is the
/// case. Kept apart from [`LISTENING`] because "not yet" and "tried and could
/// not" look the same there, and only one of them is worth telling anyone.
static PROBLEM: Mutex<Option<String>> = Mutex::new(None);

/// Set to have the loop look again now rather than at its next check.
static WAKE: AtomicBool = AtomicBool::new(false);

static SUPERVISOR: OnceLock<std::thread::Thread> = OnceLock::new();

/// Connections being answered at this moment.
static ANSWERING: AtomicUsize = AtomicUsize::new(0);

/// Point the listener, and the sync loop, at the current settings.
pub fn configure(settings: &crate::store::Settings) {
    let share = settings.share_over_tailscale;
    if ENABLED.swap(share, Ordering::SeqCst) != share {
        WAKE.store(true, Ordering::SeqCst);
        if let Some(thread) = SUPERVISOR.get() {
            thread.unpark();
        }
    }
    let sync = settings.syncs();
    if SYNCING.swap(sync, Ordering::SeqCst) != sync {
        sync_soon();
    }
}

/// Start listening whenever there is a tailnet to listen on, and syncing
/// whenever that is switched on, for as long as the app runs.
pub fn start(app: tauri::AppHandle) {
    let syncing = app.clone();
    let handle = std::thread::spawn(move || supervise(app));
    let _ = SUPERVISOR.set(handle.thread().clone());
    let handle = std::thread::spawn(move || keep_in_sync(syncing));
    let _ = SYNCER.set(handle.thread().clone());
}

struct Serving {
    address: Ipv4Addr,
    listener: TcpListener,
    gate: Arc<Gate>,
}

/// Begin answering on `address`, once Tailscale agrees it is this machine's.
///
/// The error is for Settings to show, so it is only worded for the failures a
/// person can do something about.
fn open(address: Ipv4Addr) -> Result<Serving, Option<String>> {
    // Tailscale not being there to ask is not a problem to report: the
    // address was only ever a hint, and Settings already says as much.
    let cli = cli().ok_or(None)?;
    let net = net_via(&cli).filter(|net| net.address == address).ok_or(None)?;

    let listener = TcpListener::bind((address, PORT)).map_err(|e| {
        Some(match e.kind() {
            std::io::ErrorKind::AddrInUse => format!(
                "Something else is already listening on port {PORT} — another copy of \
                 Reze, perhaps."
            ),
            _ => format!("Could not listen on {address}:{PORT} ({e})."),
        })
    })?;
    // Polled rather than blocked on, so the loop that owns it can also notice
    // Tailscale going away and the setting being switched off. A blocked
    // `accept` can only be woken by connecting to it, and an address that has
    // just vanished is not one that can be connected to.
    listener.set_nonblocking(true).map_err(|e| Some(e.to_string()))?;
    Ok(Serving {
        address,
        listener,
        gate: Arc::new(Gate {
            cli,
            user: net.user,
            seen: Mutex::default(),
        }),
    })
}

/// Keep a listener on the tailnet address for exactly as long as there is one
/// and sharing is on, and hand each connection to a thread of its own.
///
/// One loop does both jobs because they share the one thing that changes: the
/// listener. Tailscale starting after Reze, stopping under it, or coming back
/// signed in as someone else all end up here as "the address is not what it
/// was", and the answer to each is the same — drop the socket, open another.
fn supervise(app: tauri::AppHandle) {
    let shelf = Arc::new(Shelf::ours());
    let mut serving: Option<Serving> = None;
    let mut routed_before = None;
    let mut check_at = Instant::now();
    let mut retry_at = Instant::now();
    let mut busy_until = Instant::now();

    loop {
        let woken = WAKE.swap(false, Ordering::SeqCst);
        if woken || Instant::now() >= check_at {
            check_at = Instant::now() + CHECK;
            let routed = routed_address();
            let moved = routed != routed_before;
            routed_before = routed;

            let wanted = routed.filter(|_| ENABLED.load(Ordering::SeqCst));
            let mut problem_changed = false;
            if wanted != serving.as_ref().map(|s| s.address) {
                // Dropping it is what closes the socket.
                serving = None;
                let problem = match wanted {
                    Some(address) if woken || moved || Instant::now() >= retry_at => {
                        match open(address) {
                            Ok(opened) => {
                                serving = Some(opened);
                                Some(None)
                            }
                            Err(why) => {
                                retry_at = Instant::now() + RETRY;
                                Some(why)
                            }
                        }
                    }
                    // Still waiting out a failed attempt: whatever went wrong
                    // then is still what is wrong.
                    Some(_) => None,
                    None => Some(None),
                };
                if let Some(problem) = problem {
                    let mut current = PROBLEM.lock().unwrap();
                    problem_changed = *current != problem;
                    *current = problem;
                }
            }

            let listening = serving.is_some();
            let flipped = LISTENING.swap(listening, Ordering::SeqCst) != listening;
            if flipped && listening {
                // On a tailnet again: whatever was missed while off it.
                sync_soon();
            }
            if moved || flipped || problem_changed {
                // Settings shows all of this, and may be open.
                let _ = app.emit("tailnet-changed", ());
            }
        }

        let Some(current) = &serving else {
            // Nothing to accept from; sleep until the next look, or until
            // `configure` has something to say.
            std::thread::park_timeout(CHECK);
            continue;
        };
        match current.listener.accept() {
            Ok((stream, peer)) => {
                busy_until = Instant::now() + BUSY_FOR;
                if ANSWERING.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    ANSWERING.fetch_sub(1, Ordering::SeqCst);
                    continue; // dropped, which closes it
                }
                let gate = current.gate.clone();
                let shelf = shelf.clone();
                std::thread::spawn(move || {
                    // A guard, so a panic while answering cannot leak the slot.
                    struct Done;
                    impl Drop for Done {
                        fn drop(&mut self) {
                            ANSWERING.fetch_sub(1, Ordering::SeqCst);
                        }
                    }
                    let _done = Done;
                    serve(stream, gate.allows(peer.ip()), &shelf);
                });
            }
            Err(_) => std::thread::sleep(match Instant::now() < busy_until {
                true => BUSY_POLL,
                false => QUIET_POLL,
            }),
        }
    }
}

// ------------------------------------------------------------------- asking

/// Why a request to another machine came to nothing.
#[derive(Debug)]
pub enum Failed {
    /// It answered, with something other than what was asked for.
    Status(u16),
    /// It never answered, or stopped partway.
    Network(String),
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failed::Status(403) => f.write_str(
                "it does not count this machine as one of yours — both have to be \
                 signed in to Tailscale as the same user",
            ),
            Failed::Status(code) => write!(f, "it answered with an error ({code})"),
            Failed::Network(why) => f.write_str(why),
        }
    }
}

impl From<std::io::Error> for Failed {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind::*;
        Failed::Network(match e.kind() {
            // Nothing on the port: not running, a build from before this
            // existed, or sharing switched off. All the same from here.
            ConnectionRefused => "Reze is not answering there".into(),
            TimedOut | WouldBlock => "it did not answer in time".into(),
            UnexpectedEof | ConnectionReset | BrokenPipe => {
                "the connection dropped partway".into()
            }
            _ => e.to_string(),
        })
    }
}

fn garbled() -> Failed {
    Failed::Network("something other than Reze answered".into())
}

/// Ask `at` for `path`. Hands back the body, unread, and its length.
///
/// `patience` is how long to wait for the connection and then for each read —
/// short when probing a list of machines, long when fetching from one.
fn get(at: SocketAddr, path: &str, patience: Duration) -> Result<(BufReader<TcpStream>, u64), Failed> {
    let mut stream = TcpStream::connect_timeout(&at, patience)?;
    stream.set_read_timeout(Some(patience))?;
    stream.set_write_timeout(Some(patience))?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {at}\r\nConnection: close\r\n\r\n")?;

    let mut reader = BufReader::new(stream);
    let mut head = (&mut reader).take(MAX_HEAD);
    let mut line = String::new();
    head.read_line(&mut line)?;
    let code: u16 = line
        .strip_prefix("HTTP/")
        .and_then(|rest| rest.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(garbled)?;

    let mut length = None;
    loop {
        line.clear();
        if head.read_line(&mut line)? == 0 {
            return Err(garbled());
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok();
            }
        }
    }
    if code != 200 {
        return Err(Failed::Status(code));
    }
    Ok((reader, length.ok_or_else(garbled)?))
}

/// Ask for something small enough to hold in memory.
fn fetch(at: SocketAddr, path: &str, patience: Duration, most: u64) -> Result<Vec<u8>, Failed> {
    let (mut reader, length) = get(at, path, patience)?;
    if length > most {
        return Err(garbled());
    }
    let mut body = vec![0; length as usize];
    reader.read_exact(&mut body)?;
    Ok(body)
}

fn hello(at: SocketAddr, patience: Duration) -> Result<Hello, Failed> {
    serde_json::from_slice(&fetch(at, "/reze", patience, MAX_HEAD)?).map_err(|_| garbled())
}

/// Ask for a file and write it to `dest`, which only exists once it is whole.
fn download(at: SocketAddr, path: &str, dest: &Path) -> Result<(), Failed> {
    let (reader, length) = get(at, path, STALL)?;
    let mut partial = dest.as_os_str().to_owned();
    partial.push(".part");
    let partial = PathBuf::from(partial);

    let written = std::fs::File::create(&partial)
        .and_then(|mut file| std::io::copy(&mut reader.take(length), &mut file));
    match written {
        Ok(got) if got == length => Ok(std::fs::rename(&partial, dest)?),
        Ok(_) => {
            let _ = std::fs::remove_file(&partial);
            Err(Failed::Network("the connection dropped partway".into()))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            Err(e.into())
        }
    }
}

/// A file name as one segment of a URL path. Asset names are a UUID and an
/// extension, but the extension is whatever the original file happened to have.
fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let hex = |at: usize| bytes.get(at).and_then(|b| (*b as char).to_digit(16));
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                i += 3;
            }
            (byte, ..) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ------------------------------------------------------------- public API

/// What Settings shows: whether there is a tailnet, and who else is on it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Tailnet {
    /// No Tailscale on this machine at all.
    Missing,
    /// Installed, but stopped, signed out, or still starting.
    Stopped,
    Running {
        /// This machine's name on the tailnet.
        name: String,
        address: String,
        port: u16,
        /// Whether this machine is answering its own other devices right now.
        listening: bool,
        /// Why not, when it should be and tried.
        problem: Option<String>,
        /// What this machine's library holds, to hold against each device's:
        /// the same one means the two are in sync.
        revision: String,
        devices: Vec<Device>,
    },
}

/// One of this user's other machines.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub name: String,
    pub address: String,
    pub os: String,
    pub online: bool,
    /// What answered there, if a Reze did.
    pub reze: Option<Hello>,
    /// Why the last attempt to sync with it came to nothing, if it did.
    pub trouble: Option<String>,
}

/// Look at the tailnet and ask every machine on it whether Reze is there.
///
/// Blocking, for up to [`PROBE`]: the machines are asked all at once, and a
/// sleeping laptop takes the whole of that to not answer.
pub fn status() -> Tailnet {
    let Some(cli) = cli() else {
        return Tailnet::Missing;
    };
    let Some(net) = net_via(&cli) else {
        return Tailnet::Stopped;
    };

    let asked: Vec<_> = net
        .peers
        .into_iter()
        .map(|peer| {
            let at = SocketAddr::from((peer.address, PORT));
            // Tailscale already knows which machines are off; do not wait on them.
            let answer = peer.online.then(|| std::thread::spawn(move || hello(at, PROBE).ok()));
            (peer, answer)
        })
        .collect();
    let devices = asked
        .into_iter()
        .map(|(peer, answer)| Device {
            name: peer.name,
            address: peer.address.to_string(),
            os: peer.os,
            online: peer.online,
            reze: answer.and_then(|thread| thread.join().ok()).flatten(),
            trouble: TROUBLE.lock().unwrap().get(&peer.address).cloned(),
        })
        .collect();

    Tailnet::Running {
        name: net.name,
        address: net.address.to_string(),
        port: PORT,
        listening: LISTENING.load(Ordering::SeqCst),
        problem: PROBLEM.lock().unwrap().clone(),
        revision: Shelf::ours().revision().unwrap_or_default(),
        devices,
    }
}

/// Another machine's library, fetched, with the means to fetch its files.
pub struct Theirs {
    /// What the machine is called, for saying where things came from.
    pub name: String,
    pub hello: Hello,
    pub library: Library,
    at: SocketAddr,
}

/// What fetching an import's files came to.
#[derive(Debug, Default)]
pub struct Collected {
    /// How many files had to be fetched.
    pub fetched: usize,
    /// The names of attachments that machine's library has but its disk does
    /// not, for saying which were left off.
    pub missing: Vec<String>,
    /// The same attachments, by the file each would have been.
    pub gone: Vec<String>,
}

impl Collected {
    /// Take the attachments that could not be had off the macros that carry
    /// them. The macro is still worth having; a reference to nothing is not.
    pub fn strip(&self, library: &mut Library) {
        for m in &mut library.macros {
            m.attachments.retain(|a| !self.gone.contains(&a.file));
        }
    }
}

impl Theirs {
    /// Fetch into `dir` every one of `incoming` — the files a merged library
    /// needs from this machine.
    ///
    /// Run before the library is written, so that it never names a file that
    /// is not there. Until that write nothing has changed: a connection that
    /// drops halfway is an error with this machine exactly as it was, plus a
    /// few files the next sweep collects.
    pub fn collect(&self, incoming: &[Attachment], dir: &Path) -> Result<Collected, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {dir:?}: {e}"))?;
        let mut out = Collected::default();
        for attachment in incoming {
            let dest = crate::assets::bare_name(dir, &attachment.file)?;
            // Names are generated, never reused: the same name is the same file.
            if dest.exists() {
                continue;
            }
            let path = format!("/assets/{}", encode(&attachment.file));
            match download(self.at, &path, &dest) {
                Ok(()) => out.fetched += 1,
                // Their library names a file their disk no longer has.
                Err(Failed::Status(404)) => {
                    out.gone.push(attachment.file.clone());
                    out.missing.push(attachment.name.clone());
                }
                Err(e) => {
                    return Err(format!(
                        "Could not fetch {} from {}: {e}. Nothing here was changed.",
                        attachment.name, self.name
                    ))
                }
            }
        }
        Ok(out)
    }
}

/// Fetch the library of the machine at `address`.
pub fn library_of(address: &str) -> Result<Theirs, String> {
    let cli = cli().ok_or("Tailscale is not installed on this machine.")?;
    let net = net_via(&cli).ok_or("Tailscale is not connected.")?;
    // The address arrives over IPC, so it is checked rather than trusted: it
    // has to be one of the machines Tailscale says is this user's, right now.
    // Otherwise "import" would be a way to make Reze fetch from anywhere.
    let peer = net
        .peers
        .into_iter()
        .find(|p| p.address.to_string() == address)
        .ok_or("That is not one of your devices on this tailnet.")?;
    let at = SocketAddr::from((peer.address, PORT));
    let name = peer.name;

    let hello = hello(at, STALL).map_err(|e| format!("Could not reach {name}: {e}."))?;
    if hello.protocol != PROTOCOL {
        return Err(format!(
            "{name} runs a different version of Reze. Update both, then import."
        ));
    }
    let raw = fetch(at, "/library", STALL, MAX_LIBRARY)
        .map_err(|e| format!("Could not read {name}'s library: {e}."))?;
    let library: Library = serde_json::from_slice(&raw)
        .map_err(|e| format!("{name}'s library does not parse: {e}"))?;
    crate::merge::check(&library)?;

    Ok(Theirs {
        name,
        hello,
        library,
        at,
    })
}

// ------------------------------------------------------------------ syncing

/// Whether to keep this library the same as the other machines'. Mirrored
/// from settings, like [`ENABLED`].
static SYNCING: AtomicBool = AtomicBool::new(false);

static SYNCER: OnceLock<std::thread::Thread> = OnceLock::new();

/// Why the last attempt to sync with a machine came to nothing, by its
/// address, for Settings to show next to it.
static TROUBLE: Mutex<BTreeMap<Ipv4Addr, String>> = Mutex::new(BTreeMap::new());

/// Have the sync loop look now rather than at its next turn: something here
/// changed, or another machine says something there did.
pub fn sync_soon() {
    if let Some(thread) = SYNCER.get() {
        thread.unpark();
    }
}

/// Keep this library the same as the ones on this user's other machines, for
/// as long as that is switched on — here, and on each of them for itself.
///
/// Every round is the same and is all there is: ask each machine what it has,
/// fetch from the ones that have something different, and tell the ones that
/// are missing something of ours to come and do the same. Nothing depends on
/// a round having happened, or on a word having arrived; a machine that was
/// asleep through a week of changes is one round behind.
fn keep_in_sync(app: tauri::AppHandle) {
    let shelf = Shelf::ours();
    let mut told = HashMap::new();
    let mut stood = BTreeMap::new();
    loop {
        if !SYNCING.load(Ordering::SeqCst) {
            stood.clear();
            TROUBLE.lock().unwrap().clear();
            std::thread::park_timeout(SYNC_EVERY);
            continue;
        }
        std::thread::sleep(SETTLE);
        let round = round(&shelf, &mut told);

        if round.changed {
            if let Some(library) = shelf.read() {
                // Macros that were replaced or deleted let go of their files.
                crate::assets::sweep(&library);
            }
            crate::library_changed(&app);
        }
        *TROUBLE.lock().unwrap() = round
            .standing
            .iter()
            .filter_map(|(address, standing)| Some((*address, standing.clone().err()?)))
            .collect();
        if round.changed || round.standing != stood {
            // Settings says who is in sync with whom, and may be open.
            let _ = app.emit("tailnet-changed", ());
        }
        stood = round.standing;

        std::thread::park_timeout(match round.poked {
            true => FOLLOW_UP,
            false => SYNC_EVERY,
        });
    }
}

/// What one round came to.
#[derive(Default)]
struct Round {
    /// Whether the library here is different for it.
    changed: bool,
    /// Whether any machine was told to come and fetch.
    poked: bool,
    /// Where each machine that syncs now stands: holding the same as this
    /// one, not yet, or why it could not be synced with.
    standing: BTreeMap<Ipv4Addr, Result<bool, String>>,
}

fn round(shelf: &Shelf, told: &mut HashMap<SocketAddr, String>) -> Round {
    let mut out = Round::default();
    // The address first, which costs nothing to ask for: off the tailnet
    // there is nobody to sync with, and no call to start a process to hear so.
    let net = routed_address().and_then(|_| cli()).and_then(|cli| net_via(&cli));
    let Some(net) = net else {
        return out;
    };
    // Asked all at once, as the list in Settings is: one that is asleep
    // should cost the round a wait, not a wait for each.
    let asked: Vec<_> = net
        .peers
        .into_iter()
        .filter(|peer| peer.online)
        .map(|peer| {
            let at = SocketAddr::from((peer.address, PORT));
            (peer, at, std::thread::spawn(move || hello(at, PROBE).ok()))
        })
        .collect();
    for (peer, at, answer) in asked {
        // It takes both: a machine that has not asked to be kept in sync is
        // left alone, though it would hand its library to anyone who asked.
        let Some(theirs) = answer.join().ok().flatten().filter(|hello| hello.sync) else {
            continue;
        };
        let met = match theirs.protocol == PROTOCOL {
            true => meet(at, &theirs, shelf, told),
            false => Err("it runs a different version of Reze — update both".to_string()),
        };
        if let Ok(met) = &met {
            out.changed |= met.changed;
            out.poked |= met.poked;
        }
        out.standing.insert(peer.address, met.map(|met| met.level));
    }
    out
}

/// What a round came to with one machine.
#[derive(Debug)]
struct Met {
    changed: bool,
    poked: bool,
    /// Whether the two now hold the same thing.
    level: bool,
}

/// One machine's turn in a round: take what it has that this one lacks, and
/// if this one has something it lacks, say so.
fn meet(
    at: SocketAddr,
    theirs: &Hello,
    shelf: &Shelf,
    told: &mut HashMap<SocketAddr, String>,
) -> Result<Met, String> {
    let unreadable = || "the library here could not be read".to_string();
    let mut mine = shelf.revision().ok_or_else(unreadable)?;
    let mut changed = false;
    if mine != theirs.revision {
        changed = pull(at, shelf)?;
        mine = shelf.revision().ok_or_else(unreadable)?;
    }
    mend(at, shelf);

    // Still different, having taken everything it has: then the difference is
    // something of ours. It is told once for each state this library is in —
    // a machine that cannot fetch is not helped by hearing it again, and two
    // that could not agree would otherwise go on telling each other for ever.
    let level = mine == theirs.revision;
    let mut poked = false;
    if !level && told.get(&at) != Some(&mine) && fetch(at, "/poke", PROBE, MAX_HEAD).is_ok() {
        told.insert(at, mine);
        poked = true;
    }
    Ok(Met {
        changed,
        poked,
        level,
    })
}

/// Make the library on `shelf` hold everything the one at `at` does, and say
/// whether that changed it.
///
/// Files first, library last, as for an import: the library is only written
/// once every file it newly names is here. But the files are fetched with
/// nothing held, and the editor is free to save in the meantime — so what is
/// written is worked out afresh, under the lock, from the library as it is at
/// that moment, and if that turns out to need a file that has not been
/// fetched yet, round it goes again.
fn pull(at: SocketAddr, shelf: &Shelf) -> Result<bool, String> {
    // Worded to follow "Not syncing with tenet:", which is where they are read.
    let raw = fetch(at, "/library", STALL, MAX_LIBRARY)
        .map_err(|e| format!("its library could not be fetched ({e})"))?;
    let theirs: Library = serde_json::from_slice(&raw)
        .map_err(|e| format!("its library could not be read ({e})"))?;
    crate::merge::check(&theirs)
        .map_err(|_| "it runs a newer Reze than this one — update this one")?;

    // Files its library names and its disk does not have. Unlike an import,
    // the macro keeps the reference: taking it off would be a difference
    // between the two libraries that no amount of syncing could settle.
    let mut absent = HashSet::new();
    for _ in 0..ATTEMPTS {
        let wanted: Vec<Attachment> = {
            let _writing = crate::store::lock();
            let mine = shelf.read().ok_or("the library here could not be read")?;
            let out = crate::sync::reconcile(&mine, &theirs, crate::sync::now());
            if !out.changed {
                return Ok(false);
            }
            let wanted: Vec<Attachment> = out
                .incoming
                .into_iter()
                .filter(|a| !absent.contains(&a.file) && !shelf.has(a))
                .collect();
            if wanted.is_empty() {
                shelf.write(&out.library)?;
                return Ok(true);
            }
            wanted
        };
        std::fs::create_dir_all(&shelf.assets).map_err(|e| e.to_string())?;
        for attachment in wanted {
            let dest = crate::assets::bare_name(&shelf.assets, &attachment.file)?;
            match download(at, &format!("/assets/{}", encode(&attachment.file)), &dest) {
                Ok(()) => {}
                Err(Failed::Status(404)) => {
                    absent.insert(attachment.file);
                }
                Err(e) => return Err(format!("{} could not be fetched ({e})", attachment.name)),
            }
        }
    }
    Err("the library here kept changing while it was being synced".into())
}

/// Fetch from `at` any file the library on `shelf` names and does not have.
///
/// Usually there is none. When there is — a file deleted by hand, or swept
/// away in the minute a slow sync took — the machine it came from, or any
/// other that has since fetched it, is the place to get it back from. Tried
/// every round and with every machine, since which of them has it is not
/// known; one that does not says so in a few bytes.
fn mend(at: SocketAddr, shelf: &Shelf) {
    let Some(library) = shelf.read() else {
        return;
    };
    let lost = library
        .macros
        .iter()
        .flat_map(|m| &m.attachments)
        .filter(|a| a.path.is_none() && !shelf.has(a));
    for attachment in lost {
        let Ok(dest) = crate::assets::bare_name(&shelf.assets, &attachment.file) else {
            continue;
        };
        let path = format!("/assets/{}", encode(&attachment.file));
        // Not having it is an answer; not answering is the machine gone, and
        // every further file would wait on it just as long.
        if let Err(Failed::Network(_)) = download(at, &path, &dest) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A status the way Tailscale prints one, cut down to what is read, with a
    /// machine of every kind that must not be offered.
    const STATUS: &str = r#"{
        "BackendState": "Running",
        "Self": {
            "HostName": "Someone's Mac mini", "DNSName": "mac-mini.tail1234.ts.net.",
            "OS": "macOS", "UserID": 7, "Online": true, "Tags": null,
            "TailscaleIPs": ["100.71.90.33", "fd7a:115c:a1e0::1933:5a23"]
        },
        "Peer": {
            "nodekey:a": { "HostName": "zebra", "DNSName": "zebra.tail1234.ts.net.",
                "OS": "linux", "UserID": 7, "Online": true,
                "TailscaleIPs": ["100.97.46.114"] },
            "nodekey:b": { "HostName": "laptop", "DNSName": "laptop.tail1234.ts.net.",
                "OS": "macOS", "UserID": 7, "Online": false,
                "TailscaleIPs": ["100.98.21.63"] },
            "nodekey:c": { "HostName": "apple", "DNSName": "apple.tail1234.ts.net.",
                "OS": "linux", "UserID": 7, "Online": true,
                "TailscaleIPs": ["100.74.137.16"] },
            "nodekey:d": { "HostName": "localhost", "DNSName": "iphone.tail1234.ts.net.",
                "OS": "iOS", "UserID": 7, "Online": true,
                "TailscaleIPs": ["100.120.167.72"] },
            "nodekey:e": { "HostName": "colleague", "DNSName": "colleague.tail1234.ts.net.",
                "OS": "linux", "UserID": 8, "Online": true,
                "TailscaleIPs": ["100.64.0.9"] },
            "nodekey:f": { "HostName": "server", "DNSName": "server.tail1234.ts.net.",
                "OS": "linux", "UserID": 7, "Online": true, "Tags": ["tag:server"],
                "TailscaleIPs": ["100.64.0.10"] }
        }
    }"#;

    #[test]
    fn the_app_binary_is_told_to_be_a_cli() {
        // Without this the Tailscale app, asked anything by another app, tries
        // to launch its GUI instead of answering — and every machine that does
        // not set the variable globally shows up as having no Tailscale.
        let command = ask("/Applications/Tailscale.app/Contents/MacOS/Tailscale", &["status"]);
        let told = command
            .get_envs()
            .any(|(name, value)| name == "TAILSCALE_BE_CLI" && value == Some("1".as_ref()));
        assert!(told);
    }

    #[test]
    fn lists_this_users_own_computers_and_nothing_else() {
        let net = net_from(STATUS.as_bytes()).unwrap();

        assert_eq!(net.name, "mac-mini");
        assert_eq!(net.address, Ipv4Addr::new(100, 71, 90, 33));
        // Not the phone, not the colleague's machine, not the tagged server —
        // and the ones that are on come first, by name.
        let names: Vec<&str> = net.peers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["apple", "zebra", "laptop"]);
        assert!(!net.peers[2].online);
    }

    #[test]
    fn a_tailscale_that_is_not_connected_is_no_tailnet() {
        for state in ["Stopped", "NeedsLogin", "Starting", "NoState"] {
            let status = STATUS.replace("\"Running\"", &format!("\"{state}\""));
            assert!(net_from(status.as_bytes()).is_none(), "{state} was taken for up");
        }
        assert!(net_from(b"Tailscale is stopped.").is_none());
        // A machine with no peers yet prints `null` for them, not `{}`.
        let alone = r#"{"BackendState":"Running","Peer":null,
            "Self":{"DNSName":"only.ts.net.","UserID":7,"TailscaleIPs":["100.64.0.1"]}}"#;
        assert!(net_from(alone.as_bytes()).unwrap().peers.is_empty());
    }

    #[test]
    fn only_the_same_users_untagged_machines_are_let_in() {
        let whois = |user: i64, tags: &str| {
            format!(r#"{{"Node":{{"Name":"x.ts.net.","User":{user},"Tags":{tags}}},"UserProfile":{{"ID":{user}}}}}"#)
        };
        assert!(owned(whois(7, "null").as_bytes(), 7));
        assert!(owned(whois(7, "[]").as_bytes(), 7));
        assert!(!owned(whois(8, "null").as_bytes(), 7), "another user was let in");
        assert!(!owned(whois(7, r#"["tag:server"]"#).as_bytes(), 7), "a tagged node was let in");
        // Whatever Tailscale prints when it does not know the address.
        assert!(!owned(b"", 7));
        assert!(!owned(b"peer not found", 7));
    }

    #[test]
    fn knows_the_range_tailnets_are_numbered_from() {
        assert!(is_tailnet(Ipv4Addr::new(100, 64, 0, 1)));
        assert!(is_tailnet(Ipv4Addr::new(100, 127, 255, 254)));
        assert!(!is_tailnet(Ipv4Addr::new(100, 63, 0, 1)));
        assert!(!is_tailnet(Ipv4Addr::new(100, 128, 0, 1)));
        assert!(!is_tailnet(Ipv4Addr::new(192, 168, 1, 10)));
    }

    #[test]
    fn file_names_survive_the_trip_through_a_url() {
        for name in ["a1b2.png", "with space.tar gz", "naïve.jpé", "100%.png", "a%2Fb"] {
            assert_eq!(decode(&encode(name)), name);
        }
        // A separator can be smuggled in encoded; it comes out as one, and is
        // refused for being one — see the test below.
        assert_eq!(decode("..%2F..%2Fsecret"), "../../secret");
        // Truncated and nonsense escapes are left as they are, not a panic.
        assert_eq!(decode("50%"), "50%");
        assert_eq!(decode("%zz%aé"), "%zz%aé");
    }

    /// A library on disk with one attached file, one file nothing names, and a
    /// secret next door that nothing should ever reach.
    fn shelf() -> (Shelf, PathBuf) {
        let root = std::env::temp_dir().join(format!("reze-tailnet-{}", uuid::Uuid::new_v4()));
        let assets = root.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("named.png"), b"the picture").unwrap();
        std::fs::write(assets.join("orphan.png"), b"removed on purpose").unwrap();
        std::fs::write(root.join("secret"), b"not yours").unwrap();
        let library = root.join("macros.json");
        std::fs::write(
            &library,
            r#"{"version":1,
                "settings":{"hotkey":"A","pasteMode":"paste","restoreClipboard":true},
                "macros":[
                  {"id":"1","name":"with a file","body":"hello","attachments":[
                    {"id":"a","file":"named.png","name":"Shot.png","kind":"image",
                     "mime":"image/png","bytes":11}]},
                  {"id":"2","name":"plain","body":"text"},
                  {"id":"3","name":"lost its file","body":"still worth having","attachments":[
                    {"id":"b","file":"gone.png","name":"Gone.png","kind":"image",
                     "mime":"image/png","bytes":5}]}]}"#,
        )
        .unwrap();
        (Shelf { library, assets }, root)
    }

    fn code_of(reply: &Reply) -> u16 {
        match reply {
            Reply::Json(_) | Reply::File(..) => 200,
            Reply::Error(code, _) => *code,
        }
    }

    #[test]
    fn hands_out_the_library_and_only_the_files_it_names() {
        let (shelf, root) = shelf();

        let Reply::Json(raw) = answer("/reze", &shelf) else {
            panic!("no hello");
        };
        let hello: Hello = serde_json::from_slice(&raw).unwrap();
        assert_eq!((hello.protocol, hello.macros, hello.attachments), (PROTOCOL, 3, 2));
        // Sync is asked for, never assumed; and what a build from before it
        // existed says about itself still reads, as a machine that does not.
        assert!(!hello.sync);
        assert_eq!(hello.revision, shelf.revision().unwrap());
        let old: Hello = serde_json::from_str(r#"{"protocol":1,"os":"linux","macros":3}"#).unwrap();
        assert!(!old.sync && old.revision.is_empty());
        assert_eq!(code_of(&answer("/poke", &shelf)), 200);

        let Reply::Json(raw) = answer("/library", &shelf) else {
            panic!("no library");
        };
        assert_eq!(raw, std::fs::read(&shelf.library).unwrap(), "the file was not sent as it is");

        assert!(matches!(answer("/assets/named.png", &shelf), Reply::File(_, 11)));
        for (path, why) in [
            ("/assets/orphan.png", "a file no macro names"),
            ("/assets/gone.png", "a file that is named but not there"),
            ("/assets/../secret", "a path out of the directory"),
            ("/assets/..%2Fsecret", "an encoded path out of the directory"),
            ("/assets/%2Fetc%2Fpasswd", "an absolute path"),
            ("/assets/", "the directory itself"),
            ("/secret", "something that is not an asset"),
            ("/macros.json", "a file by its real name"),
            ("/", "nothing in particular"),
        ] {
            assert_ne!(code_of(&answer(path, &shelf)), 200, "{why} was handed out");
        }
        std::fs::remove_dir_all(root).ok();
    }

    /// Answer `connections` connections on loopback the way the real listener
    /// would, with the gate's verdict supplied.
    fn serving(shelf: Shelf, allowed: bool, connections: usize) -> SocketAddr {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let at = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..connections {
                let (stream, _) = listener.accept().unwrap();
                serve(stream, allowed, &shelf);
            }
        });
        at
    }

    #[test]
    fn one_reze_can_read_another_over_a_real_connection() {
        let (shelf, root) = shelf();
        let expected = std::fs::read(&shelf.library).unwrap();
        let at = serving(shelf, true, 4);
        let wait = Duration::from_secs(5);

        assert_eq!(hello(at, wait).unwrap().macros, 3);
        assert_eq!(fetch(at, "/library", wait, MAX_LIBRARY).unwrap(), expected);

        let dest = root.join("fetched.png");
        download(at, &format!("/assets/{}", encode("named.png")), &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"the picture");

        // A file that is not there is an answer, not a failure of the network
        // — the import tells the two apart — and leaves nothing half-written.
        let missing = root.join("missing.png");
        assert!(matches!(
            download(at, "/assets/orphan.png", &missing),
            Err(Failed::Status(404))
        ));
        assert!(!missing.exists() && !root.join("missing.png.part").exists());
        std::fs::remove_dir_all(root).ok();
    }

    /// The other machine's library as `library_of` would hand it over, served
    /// from `at`.
    fn theirs(shelf: &Shelf, at: SocketAddr) -> Theirs {
        Theirs {
            name: "there".into(),
            hello: Hello {
                protocol: PROTOCOL,
                os: "linux".into(),
                macros: 0,
                attachments: 0,
                sync: false,
                revision: String::new(),
            },
            library: shelf.read().unwrap(),
            at,
        }
    }

    fn empty() -> Library {
        serde_json::from_str(
            r#"{"settings":{"hotkey":"A","pasteMode":"paste","restoreClipboard":true}}"#,
        )
        .unwrap()
    }

    #[test]
    fn an_import_arrives_whole_with_its_files() {
        let (shelf, root) = shelf();
        let source = theirs(&shelf, SocketAddr::from(([127, 0, 0, 1], 0)));
        // Two files to ask for: the one that is there and the one that is not.
        let source = Theirs {
            at: serving(shelf, true, 2),
            ..source
        };
        let here = root.join("here");

        let mut merged = crate::merge::merge(&empty(), &source.library, Default::default());
        let got = source.collect(&merged.incoming, &here).unwrap();
        got.strip(&mut merged.library);

        assert_eq!(got.fetched, 1);
        assert_eq!(std::fs::read(here.join("named.png")).unwrap(), b"the picture");
        // The macro whose file is gone still came, without the dead reference.
        assert_eq!(got.missing, ["Gone.png"]);
        let lost = merged.library.macros.iter().find(|m| m.name == "lost its file").unwrap();
        assert!(lost.attachments.is_empty());
        let kept = merged.library.macros.iter().find(|m| m.name == "with a file").unwrap();
        assert_eq!(kept.attachments.len(), 1);

        // A second import asks for nothing it already has. The server above
        // has stopped answering, so this passing means nothing was asked.
        let mut again = crate::merge::merge(&empty(), &source.library, Default::default());
        again.incoming.retain(|a| a.file == "named.png");
        assert_eq!(source.collect(&again.incoming, &here).unwrap().fetched, 0);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_connection_lost_mid_import_changes_nothing_and_says_so() {
        let (shelf, root) = shelf();
        // Nothing is listening at all: every fetch fails as the network would.
        let dead = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap();
        let source = theirs(&shelf, dead);

        let merged = crate::merge::merge(&empty(), &source.library, Default::default());
        let failed = source.collect(&merged.incoming, &root.join("here")).unwrap_err();

        assert!(failed.contains("Nothing here was changed"), "{failed}");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn someone_who_is_not_this_user_is_refused_everything() {
        let (shelf, root) = shelf();
        let at = serving(shelf, false, 3);
        let wait = Duration::from_secs(5);

        assert!(matches!(hello(at, wait), Err(Failed::Status(403))));
        assert!(matches!(fetch(at, "/library", wait, MAX_LIBRARY), Err(Failed::Status(403))));
        // The same answer for something that does not exist: what is and is
        // not here is nobody else's to find out.
        assert!(matches!(fetch(at, "/nope", wait, MAX_HEAD), Err(Failed::Status(403))));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_machine_with_no_reze_on_it_is_just_not_answering() {
        // Bound and dropped: a port with nothing on it.
        let at = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap();
        let Err(Failed::Network(why)) = hello(at, Duration::from_secs(2)) else {
            panic!("something answered");
        };
        assert_eq!(why, "Reze is not answering there");
    }

    #[test]
    fn something_else_on_the_port_is_not_mistaken_for_reze() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let at = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&stream);
            let body = "<html>hello</html>";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
        });
        assert!(matches!(hello(at, Duration::from_secs(2)), Err(Failed::Network(_))));
    }

    /// A machine with sync on: a scratch library holding `macros` and
    /// remembering `deleted`, each written as it would be in the file.
    fn machine(root: &Path, name: &str, macros: &str, deleted: &str) -> Shelf {
        let assets = root.join(name).join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let library = root.join(name).join("macros.json");
        std::fs::write(
            &library,
            format!(
                r#"{{"version":1,
                    "settings":{{"hotkey":"A","pasteMode":"paste","restoreClipboard":true,"sync":true}},
                    "macros":[{macros}],"deleted":{{{deleted}}}}}"#
            ),
        )
        .unwrap();
        Shelf { library, assets }
    }

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!("reze-sync-{}", uuid::Uuid::new_v4()))
    }

    const WITH_A_FILE: &str = r#""attachments":[{"id":"f","file":"a.png","name":"A.png",
        "kind":"image","mime":"image/png","bytes":5}]"#;

    #[test]
    fn two_machines_that_sync_end_up_holding_the_same_thing() {
        let root = scratch();
        // Real times: a deletion from 1970 would be long forgotten.
        let t = crate::sync::now() - 10_000;
        let a = machine(
            &root,
            "a",
            &format!(
                r#"{{"id":"1","name":"only on a","body":"x","updatedAt":{t},{WITH_A_FILE}}},
                   {{"id":"2","name":"on both","body":"edited first","updatedAt":{t}}},
                   {{"id":"3","name":"deleted on b","body":"y","updatedAt":{t}}}"#
            ),
            "",
        );
        std::fs::write(a.assets.join("a.png"), b"a png").unwrap();
        let b = machine(
            &root,
            "b",
            &format!(
                r#"{{"id":"4","name":"only on b","body":"z","updatedAt":{}}},
                   {{"id":"2","name":"on both","body":"edited last","updatedAt":{}}}"#,
                t + 100,
                t + 200
            ),
            &format!(r#""3":{}"#, t + 300),
        );

        // B asks A: for its library, then for the one file that names.
        assert_eq!(pull(serving(a.clone(), true, 2), &b), Ok(true));
        assert_eq!(std::fs::read(b.assets.join("a.png")).unwrap(), b"a png");
        // A asks B, which has everything of A's by now. What B added, B's
        // later edit and B's deletion are the news.
        assert_eq!(pull(serving(b.clone(), true, 1), &a), Ok(true));

        for shelf in [&a, &b] {
            let library = shelf.read().unwrap();
            let mut holds: Vec<_> =
                library.macros.iter().map(|m| (m.name.as_str(), m.body.as_str())).collect();
            holds.sort();
            assert_eq!(
                holds,
                [("on both", "edited last"), ("only on a", "x"), ("only on b", "z")]
            );
        }
        assert_eq!(a.revision(), b.revision());
        // And having agreed, neither finds anything to do.
        assert_eq!(pull(serving(a.clone(), true, 1), &b), Ok(false));
        assert_eq!(pull(serving(b.clone(), true, 1), &a), Ok(false));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_machine_with_something_new_says_so_once() {
        let root = scratch();
        let t = crate::sync::now();
        let a = machine(
            &root,
            "a",
            &format!(r#"{{"id":"1","name":"new here","body":"x","updatedAt":{t}}}"#),
            "",
        );
        let b = machine(&root, "b", "", "");
        let hello = |shelf: &Shelf| -> Hello {
            let Reply::Json(raw) = answer("/reze", shelf) else {
                panic!("no hello");
            };
            serde_json::from_slice(&raw).unwrap()
        };
        assert!(hello(&b).sync);
        let mut told = HashMap::new();

        // More connections than it needs, so that a second telling would be
        // answered if it were sent.
        let at = serving(b.clone(), true, 4);
        let met = meet(at, &hello(&b), &a, &mut told).unwrap();
        assert!(!met.changed && met.poked && !met.level, "{met:?}");
        let met = meet(at, &hello(&b), &a, &mut told).unwrap();
        assert!(!met.poked, "said the same thing twice");

        // B comes and fetches, and then there is nothing to say either way.
        let at = serving(a.clone(), true, 2);
        let met = meet(at, &hello(&a), &b, &mut told).unwrap();
        assert!(met.changed && met.level && !met.poked, "{met:?}");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_sync_that_cannot_get_a_file_changes_nothing() {
        let root = scratch();
        let t = crate::sync::now();
        let a = machine(
            &root,
            "a",
            &format!(r#"{{"id":"1","name":"with a file","body":"x","updatedAt":{t},{WITH_A_FILE}}}"#),
            "",
        );
        std::fs::write(a.assets.join("a.png"), b"a png").unwrap();
        let b = machine(&root, "b", "", "");
        let before = std::fs::read(&b.library).unwrap();

        // It hands over its library and is gone before the file is asked for.
        let failed = pull(serving(a.clone(), true, 1), &b).unwrap_err();

        assert!(failed.contains("A.png"), "{failed}");
        assert_eq!(std::fs::read(&b.library).unwrap(), before);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_file_one_machine_lost_comes_back_from_another() {
        let root = scratch();
        let t = crate::sync::now();
        // A names a file it does not have.
        let a = machine(
            &root,
            "a",
            &format!(r#"{{"id":"1","name":"with a file","body":"x","updatedAt":{t},{WITH_A_FILE}}}"#),
            "",
        );
        let b = machine(&root, "b", "", "");

        // B takes the macro all the same, reference and all: a B without it
        // would differ from A for good.
        assert_eq!(pull(serving(a.clone(), true, 2), &b), Ok(true));
        assert_eq!(a.revision(), b.revision());

        // And should B have the file, A gets it back.
        std::fs::write(b.assets.join("a.png"), b"found").unwrap();
        mend(serving(b.clone(), true, 1), &a);
        assert_eq!(std::fs::read(a.assets.join("a.png")).unwrap(), b"found");
        std::fs::remove_dir_all(root).ok();
    }
}

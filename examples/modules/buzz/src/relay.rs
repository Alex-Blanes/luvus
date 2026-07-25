//! The relay layer: the NIP-29/NIP-42 wire protocol, identity, and the
//! background socket thread the TUI drives. Shared by the CLI commands and the
//! interactive pane so there is exactly one implementation of "talk to Buzz".

use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use nostr::prelude::*;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error as WsError, Message, WebSocket};

/// NIP-29 kinds we use.
pub const KIND_MESSAGE: u16 = 9;
pub const KIND_JOIN: u16 = 9021;
pub const KIND_GROUP_META: u16 = 39000;

pub type Sock = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

/// A discovered channel.
#[derive(Clone)]
pub struct Channel {
    pub uuid: String,
    pub name: String,
}

/// A chat message handed to the UI.
#[derive(Clone)]
pub struct ChatMsg {
    pub author: String, // npub (or hex fallback)
    pub content: String,
    pub ts: u64,
}

/// Commands the UI sends to the relay thread.
pub enum ToRelay {
    /// Switch the live subscription to this channel (joins it first, best-effort).
    Switch(String),
    /// Post a message to a channel.
    Send {
        channel: String,
        content: String,
    },
    Quit,
}

/// Events the relay thread sends back to the UI.
pub enum FromRelay {
    /// The discovered channel list (on connect).
    Channels(Vec<Channel>),
    /// A message arrived on the current subscription.
    Msg(ChatMsg),
    /// Reached the end of a channel's stored history.
    Eose,
    /// A status / error line for the UI to show.
    Info(String),
    /// The connection ended; the UI should show it and stop.
    Disconnected(String),
}

// ── identity ─────────────────────────────────────────────────────────────────

/// Resolve the client's identity. Priority: an explicit `nsec` (a module
/// setting), else a stored key, else a freshly generated one saved `0600`. The
/// second return is the npub, and the third is whether it was just created (so
/// the caller can tell the user to get it added to a gated relay).
pub fn identity(explicit_nsec: Option<&str>) -> Result<(Keys, String, bool)> {
    if let Some(nsec) = explicit_nsec.map(str::trim).filter(|s| !s.is_empty()) {
        if nsec.starts_with("npub") {
            bail!("that is a public key (npub); the setting needs your PRIVATE key, which starts nsec1…");
        }
        let keys = Keys::parse(nsec).context(
            "invalid private key (expected nsec1… or 64-char hex) — check the nsec setting",
        )?;
        let npub = keys.public_key().to_bech32()?;
        return Ok((keys, npub, false));
    }
    let path = identity_path();
    if let Ok(saved) = std::fs::read_to_string(&path) {
        let keys = Keys::parse(saved.trim()).context("stored identity is corrupt")?;
        let npub = keys.public_key().to_bech32()?;
        return Ok((keys, npub, false));
    }
    let keys = Keys::generate();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(&path, keys.secret_key().to_bech32()?).context("saving identity")?;
    set_owner_only(&path);
    let npub = keys.public_key().to_bech32()?;
    Ok((keys, npub, true))
}

/// Where the generated key lives. Prefers the module's own config dir (bohay
/// sets `BOHAY_MODULE_CONFIG_DIR`), so each user's key stays out of bohay's
/// shared state; falls back to `~/.config/bohay-buzz/` when run standalone.
fn identity_path() -> PathBuf {
    if let Ok(dir) = std::env::var("BOHAY_MODULE_CONFIG_DIR") {
        return PathBuf::from(dir).join("identity.nsec");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/bohay-buzz/identity.nsec")
}

/// The module's state dir (bohay sets `BOHAY_MODULE_STATE_DIR`), else a temp
/// dir when run standalone. Shared between the pane and the share-pane action.
fn state_dir() -> PathBuf {
    std::env::var_os("BOHAY_MODULE_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("bohay-buzz"))
}

/// The pane records the channel it is viewing here, so the `share` action (a
/// separate process) knows where to post. Best-effort — a stale value just
/// posts to the last-viewed channel.
pub fn write_current_channel(uuid: &str) {
    let dir = state_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("current-channel"), uuid);
}

pub fn read_current_channel() -> Option<String> {
    std::fs::read_to_string(state_dir().join("current-channel"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(unix)]
fn set_owner_only(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn set_owner_only(_path: &std::path::Path) {}

// ── connect + auth ───────────────────────────────────────────────────────────

/// Connect over WebSocket and complete the NIP-42 handshake. Buzz sends a
/// proactive `AUTH` challenge; we sign a `kind:22242` event and reply. Returns
/// the authenticated (still blocking) socket.
pub fn connect_authed(relay: &str, keys: &Keys) -> Result<Sock> {
    let relay_url = RelayUrl::parse(relay).context("invalid relay url")?;
    let (mut sock, _resp) = tungstenite::connect(relay).context("websocket connect failed")?;
    loop {
        match RelayMessage::from_json(&read_text(&mut sock)?)? {
            RelayMessage::Auth { challenge } => {
                let ev = EventBuilder::auth(challenge, relay_url.clone()).sign_with_keys(keys)?;
                send_blocking(&mut sock, ClientMessage::auth(ev).as_json())?;
                return Ok(sock);
            }
            RelayMessage::Notice(m) => eprintln!("notice: {m}"),
            RelayMessage::Closed { message, .. } => bail!("relay closed before auth: {message}"),
            _ => {}
        }
    }
}

/// One-shot channel discovery (NIP-29 group metadata is historical-only): REQ
/// `kind:39000`, collect until EOSE. Blocking; run right after connect.
pub fn discover_channels(sock: &mut Sock) -> Result<Vec<Channel>> {
    let filter = Filter::new().kind(Kind::from(KIND_GROUP_META));
    send_blocking(
        sock,
        ClientMessage::req(SubscriptionId::new("disc"), filter).as_json(),
    )?;
    let mut out = Vec::new();
    loop {
        match RelayMessage::from_json(&read_text(sock)?)? {
            RelayMessage::Event { event, .. } => out.push(Channel {
                uuid: event.tags.identifier().unwrap_or("?").to_string(),
                name: tag_value(&event, "name").unwrap_or_default(),
            }),
            RelayMessage::EndOfStoredEvents(_) => {
                // Close the discovery sub so its id is free.
                let _ = send_blocking(
                    sock,
                    ClientMessage::close(SubscriptionId::new("disc")).as_json(),
                );
                return Ok(out);
            }
            RelayMessage::Closed { message, .. } => bail!("discovery closed: {message}"),
            _ => {}
        }
    }
}

// ── the TUI's background socket thread ────────────────────────────────────────

/// Own the socket for the pane: connect, discover, then loop non-blocking —
/// forwarding incoming messages to the UI and applying the UI's commands
/// (switch channel, send). One thread does both directions, so there is no
/// shared-socket locking; a small sleep keeps it responsive without busy-spin.
pub fn run(relay: String, keys: Keys, tx: Sender<FromRelay>, rx: Receiver<ToRelay>) {
    let outcome = (|| -> Result<()> {
        let mut sock = connect_authed(&relay, &keys)?;
        let channels = discover_channels(&mut sock)?;
        tx.send(FromRelay::Channels(channels)).ok();
        set_nonblocking(&mut sock);

        let sub = SubscriptionId::new("chat");
        loop {
            // Drain everything the relay has for us right now.
            loop {
                match sock.read() {
                    Ok(Message::Text(t)) => forward(&t, &tx),
                    Ok(Message::Ping(p)) => {
                        let _ = sock.send(Message::Pong(p));
                    }
                    Ok(Message::Close(_)) => bail!("relay closed the connection"),
                    Ok(_) => {}
                    Err(WsError::Io(e))
                        if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                    {
                        break
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            // Apply pending UI commands.
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    ToRelay::Switch(ch) => {
                        // Join first (best-effort: open channels add us, private
                        // ones or already-members just no-op with an error we
                        // ignore), then (re)subscribe. Reusing the sub id makes
                        // the relay replace the previous subscription.
                        if let Ok(join) = EventBuilder::new(Kind::from(KIND_JOIN), "")
                            .tags([Tag::parse(["h", &ch])?])
                            .sign_with_keys(&keys)
                        {
                            let _ = write_msg(&mut sock, ClientMessage::event(join).as_json());
                        }
                        let filter = Filter::new()
                            .kind(Kind::from(KIND_MESSAGE))
                            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), ch.clone());
                        write_msg(&mut sock, ClientMessage::req(sub.clone(), filter).as_json())?;
                    }
                    ToRelay::Send { channel, content } => {
                        let ev = EventBuilder::new(Kind::from(KIND_MESSAGE), content)
                            .tags([Tag::parse(["h", &channel])?])
                            .sign_with_keys(&keys)?;
                        write_msg(&mut sock, ClientMessage::event(ev).as_json())?;
                    }
                    ToRelay::Quit => return Ok(()),
                }
            }
            std::thread::sleep(Duration::from_millis(60));
        }
    })();
    if let Err(e) = outcome {
        tx.send(FromRelay::Disconnected(format!("{e}"))).ok();
    }
}

/// Parse one relay frame and forward anything the UI cares about.
fn forward(raw: &str, tx: &Sender<FromRelay>) {
    let Ok(msg) = RelayMessage::from_json(raw) else {
        return;
    };
    match msg {
        RelayMessage::Event { event, .. } => {
            let author = event
                .pubkey
                .to_bech32()
                .unwrap_or_else(|_| event.pubkey.to_hex());
            tx.send(FromRelay::Msg(ChatMsg {
                author,
                content: event.content.clone(),
                ts: event.created_at.as_secs(),
            }))
            .ok();
        }
        RelayMessage::EndOfStoredEvents(_) => {
            tx.send(FromRelay::Eose).ok();
        }
        RelayMessage::Closed { message, .. } => {
            tx.send(FromRelay::Info(format!("subscription closed: {message}")))
                .ok();
        }
        RelayMessage::Ok {
            status, message, ..
        } if !status && !message.is_empty() => {
            tx.send(FromRelay::Info(format!("relay: {message}"))).ok();
        }
        RelayMessage::Notice(m) => {
            tx.send(FromRelay::Info(format!("notice: {m}"))).ok();
        }
        _ => {}
    }
}

// ── low-level helpers ────────────────────────────────────────────────────────

/// Read the next text frame (blocking), answering pings so the relay's 30s
/// heartbeat (3 missed pongs disconnects) never drops us.
pub fn read_text(sock: &mut Sock) -> Result<String> {
    loop {
        match sock.read().context("websocket read")? {
            Message::Text(t) => return Ok(t.to_string()),
            Message::Ping(p) => sock.send(Message::Pong(p)).context("pong")?,
            Message::Close(_) => bail!("relay closed the connection"),
            _ => {}
        }
    }
}

/// Blocking send (used during connect/auth/discovery, before non-blocking mode).
pub fn send_blocking(sock: &mut Sock, json: String) -> Result<()> {
    sock.send(Message::text(json)).context("websocket send")
}

/// Non-blocking send: queue the frame and flush, tolerating a full send buffer
/// (chat frames are tiny, so this rarely loops).
fn write_msg(sock: &mut Sock, json: String) -> Result<()> {
    sock.write(Message::text(json)).context("ws write")?;
    loop {
        match sock.flush() {
            Ok(()) => return Ok(()),
            Err(WsError::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn set_nonblocking(sock: &mut Sock) {
    match sock.get_mut() {
        MaybeTlsStream::Plain(s) => {
            let _ = s.set_nonblocking(true);
        }
        MaybeTlsStream::Rustls(s) => {
            let _ = s.get_mut().set_nonblocking(true);
        }
        _ => {}
    }
}

/// First value of the first tag named `key` (e.g. `name`).
pub fn tag_value(event: &Event, key: &str) -> Option<String> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.first().map(String::as_str) == Some(key))
            .then(|| s.get(1).cloned())
            .flatten()
    })
}

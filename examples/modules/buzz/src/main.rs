//! luvus-buzz — a NIP-29 nostr chat client for Buzz relays (docs/39).
//!
//! Buzz's relay speaks standard NIP-29 (relay-based groups) + NIP-42 (auth) over
//! a WebSocket, so this talks to it directly — no `buzz` CLI needed, no tokio.
//!
//!   luvus-buzz keygen
//!   luvus-buzz channels [--relay <url>] [--nsec <nsec>]
//!   luvus-buzz listen   [--relay <url>] [--nsec <nsec>] --channel <uuid>
//!   luvus-buzz send      [--relay <url>] [--nsec <nsec>] --channel <uuid> --content <text>
//!   luvus-buzz tui       [--relay <url>] [--nsec <nsec>]      # the interactive pane
//!
//! Each flag also reads an env var (the module-setting names):
//!   --relay → BUZZ_RELAY_URL   --nsec → BUZZ_PRIVATE_KEY   --channel → BUZZ_CHANNEL
//! With no --nsec, an identity is generated once and stored (see `relay::identity`).

mod relay;
mod tui;

use anyhow::{bail, Context, Result};
use nostr::prelude::*;
use relay::{connect_authed, discover_channels, identity, read_text, send_blocking, KIND_MESSAGE};

fn main() -> Result<()> {
    // rustls 0.23 needs an explicit process crypto provider or the first TLS
    // handshake panics; `ring` is the one we compiled in.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        Some("keygen") => keygen(),
        Some("channels") => channels(&args),
        Some("listen") => listen(&args),
        Some("send") => send(&args),
        Some("share") => share(&args),
        Some("tui") => {
            // Resolve startup into a Result the TUI can *display* — a bad nsec or
            // missing relay must show on-screen, not exit and take the pane's tab
            // down with it.
            let startup = (|| -> Result<(String, nostr::Keys, String, bool)> {
                let relay = relay_url(&args)?;
                let (keys, npub, created) = identity(explicit_nsec(&args).as_deref())?;
                Ok((relay, keys, npub, created))
            })();
            tui::run(startup)
        }
        _ => {
            eprintln!(
                "usage:\n  luvus-buzz keygen\n  luvus-buzz channels\n  luvus-buzz listen \
                 --channel <uuid>\n  luvus-buzz send --channel <uuid> --content <text>\n  \
                 luvus-buzz tui"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = &r {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
    r
}

fn keygen() -> Result<()> {
    let keys = Keys::generate();
    println!("nsec:  {}", keys.secret_key().to_bech32()?);
    println!("npub:  {}", keys.public_key().to_bech32()?);
    println!("hex:   {}", keys.public_key().to_hex());
    eprintln!("\nkeep the nsec secret. give the npub (or hex) to your relay operator.");
    Ok(())
}

fn channels(args: &[String]) -> Result<()> {
    let mut sock = connect_authed(&relay_url(args)?, &keys(args)?)?;
    eprintln!("discovering channels…");
    let chans = discover_channels(&mut sock)?;
    for c in &chans {
        println!("{}  {}", c.uuid, c.name);
    }
    eprintln!("— {} channel(s) —", chans.len());
    Ok(())
}

fn listen(args: &[String]) -> Result<()> {
    let channel = cfg(args, "--channel", "BUZZ_CHANNEL").context("--channel or BUZZ_CHANNEL")?;
    let mut sock = connect_authed(&relay_url(args)?, &keys(args)?)?;
    let filter = Filter::new()
        .kind(Kind::from(KIND_MESSAGE))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::H), channel.clone());
    send_blocking(
        &mut sock,
        ClientMessage::req(SubscriptionId::new("listen"), filter).as_json(),
    )?;
    eprintln!("subscribed to {channel}; waiting for messages (Ctrl+C to stop)…");
    loop {
        match RelayMessage::from_json(&read_text(&mut sock)?)? {
            RelayMessage::Event { event, .. } => {
                let who = event
                    .pubkey
                    .to_bech32()
                    .unwrap_or_else(|_| event.pubkey.to_hex());
                println!("[{}] {}", short(&who), event.content);
            }
            RelayMessage::EndOfStoredEvents(_) => eprintln!("— end of history; now live —"),
            RelayMessage::Closed { message, .. } => bail!("subscription closed: {message}"),
            _ => {}
        }
    }
}

fn send(args: &[String]) -> Result<()> {
    let channel = cfg(args, "--channel", "BUZZ_CHANNEL").context("--channel or BUZZ_CHANNEL")?;
    let content = cfg(args, "--content", "").context("--content <text> is required")?;
    let keys = keys(args)?;
    let mut sock = connect_authed(&relay_url(args)?, &keys)?;
    let event = EventBuilder::new(Kind::from(KIND_MESSAGE), content)
        .tags([Tag::parse(["h", &channel])?])
        .sign_with_keys(&keys)?;
    let id = event.id;
    send_blocking(&mut sock, ClientMessage::event(event).as_json())?;
    loop {
        match RelayMessage::from_json(&read_text(&mut sock)?)? {
            RelayMessage::Ok {
                event_id,
                status,
                message,
            } if event_id == id => {
                if status {
                    eprintln!("sent ✓");
                    return Ok(());
                }
                bail!("relay rejected: {message}");
            }
            RelayMessage::Closed { message, .. } => bail!("relay closed: {message}"),
            _ => {}
        }
    }
}

/// BUZZ-5: post a reference to the right-clicked pane into the channel the chat
/// pane is currently viewing. Runs as the `share-pane` module action, so it gets
/// the pane's context (agent/status/cwd + any text selection) in its env.
fn share(args: &[String]) -> Result<()> {
    let channel = relay::read_current_channel()
        .context("no active Buzz channel — open the chat pane and pick a channel first")?;
    let content = share_message();
    let keys = keys(args)?;
    let mut sock = connect_authed(&relay_url(args)?, &keys)?;
    let event = EventBuilder::new(Kind::from(KIND_MESSAGE), content)
        .tags([Tag::parse(["h", &channel])?])
        .sign_with_keys(&keys)?;
    let id = event.id;
    send_blocking(&mut sock, ClientMessage::event(event).as_json())?;
    loop {
        match RelayMessage::from_json(&read_text(&mut sock)?)? {
            RelayMessage::Ok {
                event_id,
                status,
                message,
            } if event_id == id => {
                if status {
                    eprintln!("shared to Buzz ✓");
                    return Ok(());
                }
                bail!("relay rejected: {message}");
            }
            RelayMessage::Closed { message, .. } => bail!("relay closed: {message}"),
            _ => {}
        }
    }
}

/// Build the chat message from the pane's context env (flat `LUVUS_PANE_*`) plus
/// any text selection (from `LUVUS_MODULE_CONTEXT_JSON`).
fn share_message() -> String {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let agent = env("LUVUS_PANE_AGENT");
    let status = env("LUVUS_PANE_STATUS");
    let dir = std::path::Path::new(&env("LUVUS_PANE_CWD"))
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let mut head = String::from("shared a pane");
    if !agent.is_empty() {
        head.push_str(&format!(" · {agent}"));
    }
    if !status.is_empty() {
        head.push_str(&format!(" ({status})"));
    }
    if !dir.is_empty() {
        head.push_str(&format!(" · {dir}"));
    }
    match selection_text() {
        Some(sel) => format!("{head}\n{sel}"),
        None => head,
    }
}

/// The mouse selection over the right-clicked pane, if any (it travels in the
/// JSON context, not a flat env var).
fn selection_text() -> Option<String> {
    let json = std::env::var("LUVUS_MODULE_CONTEXT_JSON").ok()?;
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    v.get("selection")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

// ── shared arg resolution ────────────────────────────────────────────────────

fn relay_url(args: &[String]) -> Result<String> {
    flag(args, "--relay")
        .or_else(|| env_nonempty("BUZZ_RELAY_URL"))
        .or_else(|| env_nonempty("LUVUS_SETTING_RELAY_URL")) // the module setting
        .context("set --relay, BUZZ_RELAY_URL, or the relay_url module setting")
}

/// The identity for a command: an explicit `--nsec`/env/setting, else the
/// stored/generated one. On first generation, note the npub on stderr.
fn keys(args: &[String]) -> Result<Keys> {
    let (keys, npub, created) = identity(explicit_nsec(args).as_deref())?;
    if created {
        eprintln!("generated a new identity: {npub}");
        eprintln!("(add this npub to your relay if it requires membership)");
    }
    Ok(keys)
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
fn cfg(args: &[String], name: &str, env: &str) -> Option<String> {
    flag(args, name).or_else(|| env_nonempty(env))
}
/// An env var, treating empty as unset (a blank `nsec` setting means auto-gen).
fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}
/// An explicitly supplied nsec (flag, env, or the secret module setting); `None`
/// means "use the stored/auto-generated identity".
fn explicit_nsec(args: &[String]) -> Option<String> {
    flag(args, "--nsec")
        .or_else(|| env_nonempty("BUZZ_PRIVATE_KEY"))
        .or_else(|| env_nonempty("LUVUS_SETTING_NSEC"))
}
fn short(npub: &str) -> String {
    if npub.len() > 16 {
        format!("{}…{}", &npub[..10], &npub[npub.len() - 4..])
    } else {
        npub.to_string()
    }
}

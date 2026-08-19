# buzz — chat on a Buzz relay, inside luvus

A native-Rust luvus module that talks to a [Buzz](https://github.com/block/buzz)
relay directly over its standard **NIP-29 (groups) + NIP-42 (auth)** WebSocket.
No `buzz` CLI needed, no tokio. See `docs/39-buzz-module.md` for the design.

Opens as a **chat pane** in luvus: a channel sidebar, live messages, and a
compose line. Discovers channels, self-joins open ones, and streams messages.

## Use it in luvus

```sh
luvus module link ./examples/modules/buzz     # builds the binary, registers it
```

Then set the relay in **Settings → Modules → Buzz → Relay URL** (defaults to the
example community), and open the **Buzz** pane. On first open the module
generates an identity, stores it `0600` in its config dir, and shows your `npub`.
If the relay requires membership, have that `npub` added, then reopen.

**In the pane:** type to compose, `Enter` to send, `Tab` / `Shift+Tab` to switch
channels, `PgUp`/`PgDn` to scroll, `Esc` to close.

## Settings

| Key | Meaning |
|---|---|
| `relay_url` | the community's relay (`wss://…`); one relay URL == one community |
| `nsec` | your key; **blank auto-generates and stores one**, or paste an existing nsec |

They reach the client as `LUVUS_SETTING_RELAY_URL` / `LUVUS_SETTING_NSEC`.

## CLI (the tested foundation the pane calls into)

```sh
cp Cargo.toml.example Cargo.toml   # the manifest ships as .example (see note below)
cargo build --release              # → target/release/luvus-buzz

luvus-buzz keygen                                   # make an identity
luvus-buzz channels --relay wss://… --nsec nsec1…   # list channels
luvus-buzz listen   --relay wss://… --nsec nsec1… --channel <uuid>
luvus-buzz send     --relay wss://… --nsec nsec1… --channel <uuid> --content "hi"
luvus-buzz tui      --relay wss://… [--nsec nsec1…] # the pane, standalone
```

Every flag also reads an env var: `BUZZ_RELAY_URL`, `BUZZ_PRIVATE_KEY`,
`BUZZ_CHANNEL`.

## How it works

Built on the `nostr` core crate (crypto + event/message codec) and blocking
`tungstenite` (WebSocket + rustls/`ring` TLS).

- **Connect + auth:** answers the relay's proactive NIP-42 `AUTH` challenge with
  a signed `kind:22242` event.
- **Discovery:** NIP-29 group metadata (`kind:39000`) is historical-only, so it
  queries once on connect and lists `uuid  name`.
- **Chat:** subscribes with `REQ kind:9 #h=<channel>`; sends signed `kind:9`
  events; **self-joins** open channels with a `kind:9021` request on switch.
- **Threading:** one background thread owns the socket (read + write via a small
  non-blocking loop); the UI thread only renders and reads input, so a slow relay
  never stalls the keyboard. WebSocket pings are answered so the relay's 30s
  heartbeat (3 missed pongs disconnects) never drops us.

## Why the manifest ships as `Cargo.toml.example`

`cargo install --git <luvus>` searches the whole repository for any file named
`Cargo.toml` and refuses to install when two packages have binaries (it would
find both `luvus` and `luvus-buzz`). Shipping this example's manifest as
`Cargo.toml.example` keeps the top-level install unambiguous. The module's build
step (and the snippet above) copy it to `Cargo.toml` before compiling; that
generated file is gitignored so building in place never reintroduces the clash.

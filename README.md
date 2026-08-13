# Terminal RPG

A dark-fantasy, turn-based dungeon crawler that runs in a terminal locally,
through an application-owned SSH session, or in a browser terminal viewport.
You play the Grave Knight, crossing one procedurally generated floor of the
Ossuary while fighting Skeletons, Ghouls, and Cultists.

This is a first playable prototype. A run is designed to last roughly 10–20
minutes and ends when you reach `>` or die.

## Requirements and build

- Rust 1.97 or newer on the stable channel
- A terminal at least 80 columns by 24 rows
- Python 3 and OpenSSH client tools only for the smoke-test scripts

```sh
cargo build --release
```

## Play locally

```sh
cargo run --release -- play
```

Use a known seed to reproduce a run:

```sh
cargo run --release -- play --seed 12345
```

The effective seed and generator version are shown in the game. An explicitly
seeded restart repeats that seed; an unseeded restart creates a fresh run.

The default presentation uses a restrained ANSI dark-fantasy palette. Gameplay
information is also carried by glyphs and labels, so color is never required.
Use `--no-color` to suppress all foreground and background colors.

For terminals without Unicode or color support:

```sh
cargo run --release -- play --ascii --no-color
```

## Controls

| Action | Keys |
| --- | --- |
| Start | `Enter` |
| Move or bump-attack | `q w e / a d / z x c` for eight directions; arrow keys for cardinal movement |
| Wait | `s` |
| Skill slot 1: Cleave | `1` |
| Skill slot 2: Grave Bolt | `2`, then move/cycle and confirm a target |
| Empty skill slots | `3` through `0` (reserved; no turn consumed) |
| Inspect | `i`; move the cursor with movement keys and exit with `i` or `Escape` |
| Pick up item | `g` while standing on it |
| Use item | `u`, then inventory slot `1` through `4`; cancel with `u` or `Escape` |
| Toggle help | `?` |
| Restart | `r` |
| Quit | `Shift+Q` |

The spatial movement layout is:

```text
q w e
a s d
z x c
```

Grave Bolt enters a turn-free targeting mode. Move its cursor with the movement
keys or arrows, cycle valid foes with `Tab`/`Shift+Tab`, confirm with `Enter`, and
cancel with `Escape` or `2`. It reaches six tiles through unobstructed line of
sight; only a successful cast consumes a turn and starts its three-turn cooldown.
The Grave Knight begins with 10 mana and regenerates 1 mana on every gameplay
turn. Cleave costs 3 mana and Grave Bolt costs 4; unaffordable abilities and
turn-free targeting or inspection commands neither spend nor regenerate mana.

Inspect mode is turn-free. Visible tiles report current terrain, creatures,
health, armor, activation, and combat omens. Previously explored tiles remember
terrain only, while unexplored tiles reveal no details.

The four inventory slots hold one item each. A carried Grave Torch passively
raises sight range from 8 to 12 without stacking; a Health Potion restores up to
12 health and is consumed. Successful pickup and potion use each consume one
turn. Item placement uses an independent seed-derived random stream, so it does
not perturb maps, enemies, combat rolls, or AI behavior.

## Content definitions

Static content is kept in authoritative definitions: item catalogs hold names, descriptions, glyphs, and effects; class catalogs hold stats, resources, and loadouts; monster definitions hold descriptions, glyphs, population order, and combat stats; tile definitions hold descriptions, glyph variants, and traversal/opacity semantics; and ability definitions hold targeting, costs, cooldowns, availability, and effects. Runtime state stores stable IDs or kinds and resolves static metadata through these definitions.

Movement and waiting consume a turn. Bumping a closed door opens it permanently,
consumes one turn, and leaves the player in place. Invalid terrain moves, empty
skill slots, help, and commands received while the terminal is below 80×24 do
not consume turns.

Successful attacks vary within a small bounded spread around the attacker's base
damage before armor is applied. Damage rolls are deterministic for a given run
seed and command sequence, so seeded runs remain reproducible.

Player classes and their ten-slot ability loadouts are backed by validated game
definitions. The prototype currently ships the Grave Knight with Cleave in slot
1 and Grave Bolt in slot 2; slots 3-0 are empty and consume no turn when pressed.

## Serve over SSH

Start the development server:

```sh
cargo run --release -- serve
```

Then connect from another terminal:

```sh
ssh -tt -p 2222 -o PreferredAuthentications=none player@127.0.0.1
```

Each connection receives an independent fresh run. The default listener is
`127.0.0.1:2222`; change it with `--listen`. The server creates and reuses an
Ed25519 key at `.terminal-rpg/host-key` by default, or at the path supplied with
`--host-key`. The key is created with mode `0600` on Unix.

The SSH service intentionally accepts `none` authentication for local prototype
testing. It is not suitable for exposure to an untrusted network. Keep the
loopback default unless authentication and deployment hardening are added. The
server accepts only PTY-backed game shells and rejects exec, subsystem, X11, and
environment requests; it never launches an operating-system shell. Slow output
consumers are disconnected when their bounded queue saturates so their terminal
diff stream cannot become corrupt or affect healthy sessions.

`--ascii` and `--no-color` are also supported by `serve` and apply to every
session served by that process.

## Play in a browser

Start the unauthenticated development web server:

```sh
cargo run --release -- web
```

Then open <http://127.0.0.1:8080/>. The page focuses the game on load; click the
viewport to restore keyboard focus if needed. It uses the same controls, colors,
minimum 80×24 behavior, renderer, and independent per-connection game sessions
as terminal play. Resizing the browser resizes the game viewport. A disconnected
session reconnects only when you select **Reconnect**, which starts a fresh run.

If the game runs inside an isolated VM, forward the loopback web port from your
local machine:

```sh
ssh -L 8080:127.0.0.1:8080 user@your-vm
```

Keep that SSH connection open and visit <http://127.0.0.1:8080/> locally. Change
the local side of the forwarding command if port 8080 is already occupied.

The default web listener is `127.0.0.1:8080`; change it with `--listen`.
`--ascii` and `--no-color` apply to all browser sessions served by that process.
The service has no authentication and is intended only for local development or
access through a trusted port forward. Do not bind it to an untrusted network.
It serves only the embedded game page and versioned game WebSocket protocol—no
operating-system shell, command execution, or filesystem interface is exposed.

The browser UI vendors [xterm.js 6.0.0](https://github.com/xtermjs/xterm.js) and
`@xterm/addon-fit` 0.11.0 under their MIT licenses. Pinned production assets,
license texts, and checksums are stored in `web/vendor/`; no CDN is used at
runtime.

## Architecture

- `src/world/`: map semantics, invariants, and deterministic versioned generation
- `src/game/`: synchronous serializable run state, combat, visibility, and enemy AI
- `src/session.rs`: transport-neutral input decoding and intent dispatch
- `src/ui/`: Ratatui presentation and the local Crossterm lifecycle adapter
- `src/server/`: Russh sessions, bounded SSH output, and host-key persistence
- `src/web/`: embedded HTTP assets, security policy, browser protocol, logical terminal backend, and WebSocket sessions
- `src/main.rs`: CLI and structured diagnostics

The game and world layers do not depend on Ratatui, Crossterm, Russh, Tokio, or
terminal I/O. Local, SSH, and browser adapters decode input into the same
semantic intents and invoke the same engine. See
[docs/architecture.md](docs/architecture.md) for dependency direction,

Static content is centralized in authoritative definitions: item catalogs include names, descriptions, glyphs, and effects; class catalogs include stats, resources, and loadouts; monster definitions include descriptions, glyphs, population order, and combat stats; tile definitions include descriptions, glyph variants, and traversal/opacity semantics; and ability definitions include targeting, costs, cooldowns, availability, and effects. Runtime state stores stable IDs or kinds and resolves static metadata through these definitions.
responsibilities, tradeoffs, and extension guidance.

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --all-features
python3 scripts/verify-local.py --expect-color -- target/debug/terminal-rpg play --seed 12345
python3 scripts/verify-local.py --expect-no-color -- target/debug/terminal-rpg play --seed 12345 --ascii --no-color
python3 scripts/verify-local.py --expect-color --exercise-items -- target/debug/terminal-rpg play --seed 25
python3 scripts/verify-local.py --expect-color --exercise-potion -- target/debug/terminal-rpg play --seed 4
bash scripts/verify-ssh.sh
bash scripts/verify-web-assets.sh
bash scripts/verify-web.sh
git diff --check
```

The local smoke uses a real pseudo-terminal and verifies title-to-game entry,
quit, terminal attributes, cursor restoration, and alternate-screen cleanup. The
SSH smoke exercises request rejection, independent concurrent games, restart,
malformed input, slow clients, resize transitions, disconnect/reconnect, and
stable host identity. The browser smoke uses live HTTP and WebSocket connections
to verify fragmented protocol input, independent sessions, keyboard and skill
input, resize transitions, malformed and slow clients, clean quit, reconnect,
and listener health.

## Prototype limitations

There are no accounts, saves, reconnectable runs, multiplayer, multiple floors,
inventory, loot, progression, scripting, database, or deployment automation.
Balance and content are intentionally compact, and SSH/browser authentication is
for development only.


For development-only combat debugging, pass `--debug-godmode` to `play`, `serve`, or `web`; uppercase `G` toggles per-session invulnerability. It does not grant mana, damage, items, or other cheats.

## Redeploy the web service

From the repository root, run `scripts/redeploy.sh --host your-vm --user user` (or use `--user`, `--port`, `--binary`, `--service`, and `--config-dir` to override the defaults). Preview the exact plan first with `scripts/redeploy.sh --host your-vm --user user --dry-run`. For a deployment on the current machine, use `scripts/redeploy.sh --local --sudo`. The script builds and checks the release, transfers only the binary and deployment templates, stages the replacement, restarts `terminal-rpg-web.service`, and checks the local HTTP endpoint. On a failed health check it attempts to restore the previous binary. SSH access, remote service permissions, and any Cloudflare credentials must be configured separately; the script never copies or manages secrets.

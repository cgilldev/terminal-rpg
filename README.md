# Terminal RPG

A dark-fantasy, turn-based dungeon crawler that runs entirely in a terminal,
either locally or through an application-owned SSH session. You play the Grave
Knight, crossing one procedurally generated floor of the Ossuary while fighting
Skeletons, Ghouls, and Cultists.

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
| Empty skill slots | `2` through `0` (reserved; no turn consumed) |
| Toggle help | `?` |
| Restart | `r` |
| Quit | `Shift+Q` |

The spatial movement layout is:

```text
q w e
a s d
z x c
```

Movement and waiting consume a turn. Bumping a closed door opens it permanently,
consumes one turn, and leaves the player in place. Invalid terrain moves, empty
skill slots, help, and commands received while the terminal is below 80×24 do
not consume turns.

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

## Architecture

- `src/world.rs`: deterministic versioned dungeon generation and map invariants
- `src/game.rs`: synchronous, serializable game state, combat, FOV, and enemy AI
- `src/ui.rs`: shared semantic input decoding, Ratatui rendering, and local PTY lifecycle
- `src/server.rs`: Russh transport, isolated session ownership, bounded I/O, and host keys
- `src/main.rs`: CLI and structured diagnostics

The game and world layers do not depend on Ratatui, Crossterm, Russh, Tokio, or
terminal I/O. Local and SSH transports decode input into the same semantic
intents and invoke the same engine.

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-features
python3 scripts/verify-local.py --expect-color -- target/debug/terminal-rpg play --seed 12345
python3 scripts/verify-local.py --expect-no-color -- target/debug/terminal-rpg play --seed 12345 --ascii --no-color
bash scripts/verify-ssh.sh
git diff --check
```

The local smoke uses a real pseudo-terminal and verifies title-to-game entry,
quit, terminal attributes, cursor restoration, and alternate-screen cleanup. The
SSH smoke exercises request rejection, independent concurrent games, restart,
malformed input, slow clients, resize transitions, disconnect/reconnect, and
stable host identity.

## Prototype limitations

There are no accounts, saves, reconnectable runs, multiplayer, multiple floors,
inventory, loot, progression, scripting, database, or deployment automation.
Balance and content are intentionally compact, and SSH authentication is for
development only.

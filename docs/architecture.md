# Architecture

This document records both the evidence behind the current organization and the
boundaries contributors should preserve as the prototype grows.

## Assessment before reorganization

The prototype began as six top-level modules. That was effective while features
were small, but the module sizes and imports showed several expansion hazards:

- `game.rs` (1,335 lines including tests) owned public domain types, run lifecycle,
  movement, combat, enemy AI, visibility, population, and all tests.
- `ui.rs` (1,296 lines including tests) owned the semantic palette, rendering,
  transport-neutral input decoding and intent dispatch, local terminal lifecycle,
  and snapshots.
- `server.rs` (560 lines) combined SSH configuration, host-key persistence,
  terminal buffering, session state, and Russh callbacks.
- `web.rs` (731 lines) combined embedded assets and HTTP policy, wire protocol,
  ANSI backend emulation, WebSocket lifecycle, and tests.
- `world.rs` combined the map model with procedural-generation policy and tests.
- The domain modules themselves did not import terminal or networking crates, but
  the shared input/session behavior lived inside the presentation module. That
  made transports depend on `ui` for both rendering and application semantics.

Browser assets in `web/`, verification utilities in `scripts/`, and service
definitions in `deploy/` were already explicit and should remain separate from
Rust module state.

## Chosen structure

The crate uses a layered dependency direction:

```text
CLI / app configuration
        |
        v
local UI      SSH transport      web transport
        \          |              /
         \         v             /
          presentation + session
                    |
                    v
               game domain
                    |
                    v
                world model
```

- `world/` owns map semantics and deterministic generation.
- `game/` owns actors, run state, game rules, combat, AI, and visibility.
- `session` owns transport-neutral input decoding and intent-to-game dispatch.
- `ui/` owns Ratatui presentation and the local Crossterm adapter.
- `server/` owns the SSH adapter, host-key persistence, and SSH backpressure.
- `web/` owns HTTP assets/security, browser protocol, its logical terminal
  backend, and WebSocket lifecycle.
- `app` and `main` own mode configuration and CLI composition only.

The current Rust source tree is:

```text
src/
├── app.rs
├── main.rs
├── session.rs
├── game/
│   ├── mod.rs          # public model and run lifecycle
│   ├── abilities.rs    # IDs, definitions, slots, and runtime cooldowns
│   ├── ai.rs           # activation, pursuit, and telegraphs
│   ├── combat.rs       # attacks, armor, and cooldowns
│   ├── inspection.rs   # fog-safe structured tile and entity facts
│   ├── items.rs        # definitions, placement, inventory, and effects
│   ├── classes.rs      # class definitions and validated content catalog
│   ├── population.rs   # stable IDs and seeded enemy population
│   ├── targeting.rs    # modal cursor, target validation, and confirmation
│   └── visibility.rs   # field of view and line of sight
├── world/
│   ├── mod.rs          # positions, tiles, map, and invariants
│   └── generation.rs   # room-and-corridor generation policy
├── ui/
│   ├── mod.rs          # semantic palette and Ratatui renderer
│   └── local.rs        # local Crossterm setup and cleanup
├── server/
│   ├── mod.rs          # Russh session lifecycle
│   ├── terminal.rs     # bounded SSH output
│   └── host_key.rs     # host-key persistence
└── web/
    ├── mod.rs          # WebSocket session lifecycle
    ├── assets.rs       # embedded assets and HTTP security headers
    ├── protocol.rs     # versioned wire messages
    └── terminal.rs     # logical ANSI backend and output queue
```

The compatibility module names and documented entry points remain available, but
implementation submodules are private by default. Tests live beside the boundary
they exercise, with cross-boundary and end-to-end checks in `tests/` and
`scripts/`.

## Alternatives rejected

- **Separate workspace crates now:** this would enforce domain boundaries, but it
  adds publishing and dependency-management overhead before the prototype has a
  second consumer. Source-level boundary tests provide a lighter guard for now.
- **One generic transport trait:** SSH callback lifecycles and WebSocket
  backpressure differ substantially. Sharing game-session semantics is useful;
  hiding all I/O behind a speculative trait would obscure important behavior.
- **Entity-component system:** current actors and rules do not require an ECS.
  Cohesive domain modules leave room to revisit storage if items, effects, and
  actor variety make the benefit concrete.
- **Move browser assets into Rust modules:** keeping authored and vendored assets
  under `web/` makes auditing and checksum verification clearer; compile-time
  embedding remains an adapter detail.

## Extension points

- Add generation algorithms or multi-floor policy behind `world` without adding
  presentation or transport imports.
- Add actors, abilities, items, and effects as game-domain modules; expose only
  state required by presentation and session commands.
- Add persistence as an application boundary that serializes domain state rather
  than teaching domain types about files or databases.
- Add transports by translating their input into `session::Intent` and rendering
  through the shared presentation, while retaining transport-specific security,
  flow control, and lifecycle code in the adapter.

### Adding a class or ability

Player identity is represented by `ClassId` and `ClassDefinition`; hostile
species remain separate in `ActorKind`. To add a class, assign a stable ID, add
its display name and starting combat stats, and bind ability IDs to unique slots
1 through 10. `GameCatalog::new` rejects duplicate IDs or slots, out-of-range
slots, unknown abilities, and abilities unavailable to that class.

An ability definition owns its stable ID, name, cooldown, targeting metadata,
availability rule, and typed effect. Add effect execution to the focused dispatch
in `game/combat.rs`; transports continue to send only `UseSkill(AbilitySlot)` and
never learn content IDs or cooldown rules. Runtime cooldowns live in the run's
ten-slot `AbilityState` array, while definitions remain immutable in its validated
catalog. A future class-selection screen should call `new_with_class` (or the
validated `new_with_catalog` extension seam) and need not change SSH or browser
protocols.

Targeted abilities declare hostile-single targeting, range, and line-of-sight
requirements in their immutable definitions. `TargetingState` serializes only a
semantic ability slot, map cursor, and optional actor ID. Session adapters decode
movement, cycling, confirmation, and cancellation into shared intents; target
ordering, validation, damage, cooldowns, and turn costs remain in the game
domain. Entering, moving, cycling, invalid confirmation, help, and cancellation
are free. Only successful confirmation clears the mode and advances one turn.

Inspection follows the same semantic modal boundary. `session` sends only
toggle, cursor movement, and cancellation commands; `game::inspection` returns
structured, renderer-neutral facts. New terrain, actor, class, and telegraph
variants add typed names and descriptions there. The query enforces fog of war,
so adapters cannot disclose live entities from remembered or unknown tiles.

Items use stable definition and instance IDs in a validated in-code catalog.
`game::items` owns deterministic placement, four-slot inventory rules, passive
visibility effects, and consumable execution. Its dedicated seed-derived stream
is separate from generation, enemy population, combat, and AI. Presentation
looks up typed item metadata and `session` sends only pickup/use intents, so a
future effect can be added to the catalog and domain dispatch without teaching
SSH or browser transports about item IDs or healing values. Inspection exposes
ground-item descriptions only for currently visible tiles.

## Repository map

```text
src/       Rust domain, session, presentation, adapters, and CLI
tests/     crate-boundary and deterministic-generation integration tests
web/       authored browser shell and pinned xterm.js assets
scripts/   local, SSH, browser, and asset verification
deploy/    systemd and Cloudflare Tunnel configuration templates
docs/      architecture and contributor guidance
```

The authoritative verification commands are listed in the README. A release is
not considered healthy until unit/integration tests and all three live session
smokes (local PTY, SSH, and browser) pass.

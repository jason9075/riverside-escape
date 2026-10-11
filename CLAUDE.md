# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Riverside Escape is a 2-player co-op browser game (Rust + Bevy): one player rides a bicycle along a Taipei riverside path, the other sits on the back and shoots at an AI-controlled helicopter. The repo is currently only a scaffold. The client and server are empty Bevy shells, and `riverside-common` holds no code yet. No gameplay, networking, assets, lobby, or tests exist so far.

The design source of truth is `riverside-escape-architecture-guide.md` (in Traditional Chinese). Read it before adding any gameplay, networking, or crate-structure work. Its code snippets are conceptual sketches, not drop-in code.

## Commands

```sh
cargo run -p riverside-server          # headless native server (60 Hz ScheduleRunner loop)
cargo run -p riverside-client          # native windowed client
NO_COLOR=true trunk serve              # browser (WASM) client via web/index.html
trunk build --release                  # WASM build into dist/
cargo build -p riverside-client --target wasm32-unknown-unknown   # quick WASM compile check
cargo clippy --workspace --all-targets
cargo fmt --all
cargo test --workspace                 # single test: cargo test -p <crate> <test_name>
```

`rust-toolchain.toml` pins stable and adds the `wasm32-unknown-unknown` target plus clippy/rustfmt. Trunk (0.21.14) is a separate CLI that you install yourself. `Trunk.toml` points at `web/index.html`, which builds the `riverside-client` bin through a `data-trunk rel="rust"` link.

## Architecture

- **Workspace crates**: `common` (shared lib), `client` (Bevy app, native + WASM), `server` (native dedicated server). Bevy is pinned exactly (`=0.19.0`, with the `webgl2` feature) in `[workspace.dependencies]`. Upgrade Bevy and any networking or physics crates (Lightyear and Avian are the candidates) together, and only after checking their compatibility matrices. The lockfile is the source of truth.
- **Server stays headless**: it depends only on `bevy_app` and drives a fixed 60 Hz loop with `ScheduleRunnerPlugin`. Never pull the full `bevy` crate, renderer, audio, or browser APIs into the server.
- **Authority model** (planned): the server is authoritative for all world state, hits, damage, and win/lose. Clients send inputs only, and the server clamps and validates them, including seat permissions, tick range, cooldowns, and ammo. The Rider client uses prediction and reconciliation. Other entities are interpolated. The Gunner's aim/camera must respond locally and immediately. Planned rates: 60 Hz sim, 20 Hz snapshots. Plan for WebSocket transport first.
- **Bicycle + Seats** (planned): a single `Bicycle` entity owns the physics transform. Rider and Gunner are `Seat`s (`SeatKind`) on that entity, not two independently simulated players. Don't hard-code `player_count == 2`; extend through new seat kinds.
- **Code sharing** (planned): `common` holds the protocol (inputs, snapshots, events), shared components, and game constants. The guide also proposes `simulation` and `networking` crates for deterministic-enough movement shared between client prediction and the server. Everything shared must also compile for `wasm32-unknown-unknown`, so avoid native-only crates there.
- Milestones P0–P6 are listed in §13 of the guide. The next planned step is P0/P1: cube vehicles, two browsers in one room, input sequencing, and correction metrics, with Gunner aim stability as the priority.

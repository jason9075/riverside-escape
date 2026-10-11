# Riverside Escape

Rust + Bevy workspace for the Riverside Escape browser game.

The current prototype covers P0 riding and camera controls plus the first P1 room flow: one authoritative bike, Rider/Gunner seats, Ready, and a server-timed five-second countdown. Combat and the full invite/lobby flow are not part of this build.

## Prerequisites

- Rust stable. `rust-toolchain.toml` selects the `wasm32-unknown-unknown` target and the `rustfmt` / `clippy` components.
- Trunk 0.21.14 for serving and building the browser client. See the [Trunk installation guide](https://trunkrs.dev/#install).
- Chromium or Firefox for browser testing.

The current Linux development machine already has the Bevy audio, Wayland, X11, and Vulkan loader libraries installed.

## Setup

Install [Rustup](https://rustup.rs/) if Rust is not installed. From the project directory, Rustup will install the stable compiler and components selected in `rust-toolchain.toml`.

The Rust toolchain is pinned by `rust-toolchain.toml`; Trunk is a separate CLI tool.

## Run

Start the headless game server:

```sh
cargo run -p riverside-server
```

For one-player development testing, enable the solo seat on the server:

```sh
RIVERSIDE_DEV_SOLO=1 cargo run -p riverside-server
```

Then serve the browser client:

```sh
NO_COLOR=true trunk serve
```

Open the Trunk URL in two browser tabs for the normal co-op flow. The server assigns Rider to the first connection and Gunner to the second. Both players press **R** after connecting; the server starts the five-second countdown only when both seats are connected and Ready. The server listens on `0.0.0.0:3001` by default; override it with `RIVERSIDE_BIND`.

For solo development, open one tab with `?solo=1` appended to the Trunk URL. Press **V** to switch between Rider and Gunner, then press **R** to mark both seats Ready and start the same five-second countdown. The server exposes this path only when `RIVERSIDE_DEV_SOLO=1` is set; normal two-player rooms keep their existing Ready flow. With a custom server, combine the options, for example `?server=ws%3A%2F%2F192.168.1.20%3A3001%2Fws&solo=1`.

The browser connects to port `3001` on the host used by the Trunk page. For a different server, pass a WebSocket URL in the page query, for example `?server=ws://192.168.1.20:3001/ws`. An HTTPS page needs a `wss://` endpoint.

The dev profile omits debug symbols to keep Bevy's WASM build manageable. Set `CARGO_PROFILE_DEV_DEBUG=2` for native debugging sessions that need full symbols.

For the native local preview, run:

```sh
BEVY_ASSET_ROOT="$PWD" cargo run -p riverside-client
```

When the browser server is unavailable, the client still opens as a local Rider preview. **V** switches between the Rider and Gunner camera locally. The native preview currently stays offline.

### Prototype controls

| Role | Input | Behavior |
|---|---|---|
| Rider | Hold left mouse button | Pedal |
| Rider | Space | Brake; releasing the pedal coasts instead |
| Rider | A / D | Steer; temporary bindings pending product review |
| Rider | Hold right mouse button and move | Free-look; releasing returns to chase view |
| Gunner | Move mouse | Rotate the rear seat and first-person view |
| Gunner | Hold right mouse button | Aim state for the later lock-on milestone |
| Either | R | Toggle Ready in co-op before the match starts |
| Solo development | V | Switch Rider/Gunner role while connected |
| Solo development | R | Force Ready both seats and start the countdown |
| Local preview | V | Switch Rider/Gunner view while offline |

Leaving the game window clears held input. Mouse capture is released with **Esc** in the Gunner view.

## Models

The first stylized low-poly model kit is in `assets/models/` (15 standalone GLB files).
Open `art/riverside_models.blend` to edit the source or inspect the assembled bicycle crew.
See [the model guide](art/README.md) for previews, pivots, seat sockets, scale, and rebuild commands.
The models are exported and validated, and the P0 scene now loads the bicycle, two seated riders, road segments, riverside dressing, and prototype obstacles through Bevy's glTF loader.

Trunk copies `assets/` into the browser bundle. For native development, the command above
points Bevy at this workspace's asset root rather than the client crate directory.

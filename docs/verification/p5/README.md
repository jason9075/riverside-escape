# P5 room and role selection verification

The first P5 slice replaces the implicit shared public lobby with explicit room creation and invite-code joining. Each public room has isolated simulation state and snapshot broadcasts, supports at most two members, and lets either member choose an open Rider or Gunner seat. New members remain unassigned and cannot Ready until they choose. The room directory allows four active public rooms and removes empty rooms after ten minutes. Development Solo rooms remain separate.

`room-smoke-report.json` records a WebSocket integration run against a temporary headless Server. It created four public rooms and verified that an unassigned member's Ready request is ignored; a first member can choose Gunner and a later member Rider; concurrent claims for Rider have exactly one winner; a third member is rejected; and a fifth active public room is refused. The run also rejected a proposed role swap without changing seats, ignored a repeated response, accepted a second proposal and atomically exchanged both seats, cleared Ready after the swap, cancelled a Countdown when a player withdrew Ready, and restarted a full Countdown. Room A advanced through its Countdown while room B remained in ReadyCheck and advanced its own tick.

The Server assigns each swap a room-local request ID and a 15-second expiry. Only the requested teammate can accept or decline. A pending request blocks the Countdown; rejection, expiry, or departure cancels it for both players. A successful swap clears both Ready states and all held role input. Requests are accepted before a match and after it finishes, and rejected during Countdown or Running.

The reconnect slice gives every public-room member a random, rotating token that remains in that browser tab's `sessionStorage`; the invite URL contains only the room code. An unexpected disconnect during Running pauses that room, clears held controls, aim lock, attack warnings, and projectiles, and freezes gameplay cooldowns while retaining the current match resources and route progress. The player has 30 seconds to reclaim the same seat. After reconnect, both members Ready again and the Server runs a fresh five-second resume countdown. Expiry or an explicit Leave during a match marks the result Aborted; a normal disconnect during the initial countdown cancels it and clears both Ready states.

`reconnect-smoke-report.json` records a direct WebSocket integration run covering paused-state preservation, reconnect seat restoration, token rotation, cross-room and stale-token rejection, duplicate live-token rejection, Ready-to-resume, and explicit Leave abort. The additional browser run in `browser-reconnect-report.json` refreshes the actual WASM page, then drops its socket through a test-only loopback TCP proxy. It verifies that both refresh and short-drop retries restore the original seat, rotate and store tokens in `sessionStorage`, keep tokens out of invite URLs, and resume the paused match after both members Ready. The browser report includes WebSocket URLs with reconnect token values redacted. The proxy control endpoint binds only to `127.0.0.1` and drops the test connection; it does not change host network settings.

Run the checks from the project root after building the Server and WASM client:

```sh
node tools/p5_reconnect_smoke.cjs
node tools/p5_browser_reconnect.cjs
```

The first script starts a temporary Server on port 3113. The browser script expects the built page at `http://127.0.0.1:8080/`, starts another temporary Server on port 3114, and launches headless Chrome / SwiftShader plus `ws_disconnect_proxy.cjs` on loopback ports 3115/3116. Set `RIVERSIDE_SERVER_BIN`, `RECONNECT_SMOKE_PORT`, `RECONNECT_SMOKE_REPORT`, `BROWSER_SITE_URL`, `BROWSER_RECONNECT_PORT`, `BROWSER_RECONNECT_PROXY_PORT`, `BROWSER_RECONNECT_CONTROL_PORT`, `BROWSER_DEBUG_PORT`, `BROWSER_RECONNECT_REPORT`, or `CHROME_BIN` to override these defaults.

Run the smoke check from the project root after building the Server:

```sh
node tools/p5_room_smoke.cjs
```

The script starts an isolated Server on port 3112, drives the WebSocket protocol directly, and writes `room-smoke-report.json`. Override `RIVERSIDE_SERVER_BIN`, `ROOM_SMOKE_PORT`, or `ROOM_SMOKE_REPORT` to use another binary, port, or report path. The `ws` Node package must be available to the script.

The remote headless Chromium / SwiftShader P3 regression also passed after adding explicit create/join URLs and role cards. It verified a displayed, copyable invite URL, joining by room code, role selection, a Rider-initiated swap accepted with the Gunner's visible Lobby button, the three-shot authoritative helicopter defeat, rematch, and Solo `R` / `V` behavior at fixed 100 ms RTT; see [`P3 verification`](../p3/README.md). The WebSocket integration and Server tests cover rejection, duplicate replies, timeout, departure, phase limits, Ready clearing, countdown cancellation, reconnect reservations, paused-state restoration, and aborted matches.

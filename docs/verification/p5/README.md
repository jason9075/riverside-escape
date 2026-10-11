# P5 room and role selection verification

The first P5 slice replaces the implicit shared public lobby with explicit room creation and invite-code joining. Each public room has isolated simulation state and snapshot broadcasts, supports at most two members, and lets either member choose an open Rider or Gunner seat. New members remain unassigned and cannot Ready until they choose. The room directory allows four active public rooms and removes empty rooms after ten minutes. Development Solo rooms remain separate.

`room-smoke-report.json` records a WebSocket integration run against a temporary headless Server. It created four public rooms and verified that an unassigned member's Ready request is ignored; a first member can choose Gunner and a later member Rider; concurrent claims for Rider have exactly one winner; a third member is rejected; and a fifth active public room is refused. The run also rejected a proposed role swap without changing seats, ignored a repeated response, accepted a second proposal and atomically exchanged both seats, cleared Ready after the swap, cancelled a Countdown when a player withdrew Ready, and restarted a full Countdown. Room A advanced through its Countdown while room B remained in ReadyCheck and advanced its own tick.

The Server assigns each swap a room-local request ID and a 15-second expiry. Only the requested teammate can accept or decline. A pending request blocks the Countdown; rejection, expiry, or departure cancels it for both players. A successful swap clears both Ready states and all held role input. Requests are accepted before a match and after it finishes, and rejected during Countdown or Running.

Run the smoke check from the project root after building the Server:

```sh
node tools/p5_room_smoke.cjs
```

The script starts an isolated Server on port 3112, drives the WebSocket protocol directly, and writes `room-smoke-report.json`. Override `RIVERSIDE_SERVER_BIN`, `ROOM_SMOKE_PORT`, or `ROOM_SMOKE_REPORT` to use another binary, port, or report path. The `ws` Node package must be available to the script.

The remote headless Chromium / SwiftShader P3 regression also passed after adding explicit create/join URLs and role cards. It verified a displayed, copyable invite URL, joining by room code, role selection, a Rider-initiated swap accepted with the Gunner's visible Lobby button, the three-shot authoritative helicopter defeat, rematch, and Solo `R` / `V` behavior at fixed 100 ms RTT; see [`P3 verification`](../p3/README.md). The WebSocket integration and Server tests cover rejection, duplicate replies, timeout, departure, phase limits, Ready clearing, and countdown cancellation. Reconnect reservations and packet loss/jitter remain later P5/P6 work.

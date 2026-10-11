# P4 route slice verification

This slice adds the shared 1,200.7 m course and its road-boundary collision data. The Rider is free to steer; the route projection only decides whether the next body position remains on the authored road.

The route contains 401.9 m of open riverside, 400.0 m of bridge-underpass road and 398.8 m of narrow bends. It uses 137 straight GLB pieces and 20 radius-10 m, 30-degree curve pieces. A reverse placement of the existing curve GLB supplies left turns. The headless Server reads the same course and collision code as Client prediction without loading rendering assets.

`cargo test -p riverside-common fixed_step_rider_can_follow_the_full_route_and_avoid_the_early_barrier -- --nocapture` traverses 1,197.9 m in 110.6 seconds at 6,633 fixed ticks. The controlled Rider follows the centerline through each bend and offsets around the existing concrete barrier. It stops within the test's 3 m completion threshold; ordered checkpoints and an authoritative escape finish are part of a later slice, so this is not the final escape completion time.

`route-start.png` is a 1280 × 657 remote Chromium / SwiftShader WebGL2 capture of the actual WASM Client, showing the course ahead from the starting area. A fixed 100 ms RTT two-client browser regression is recorded in `combat-compat-100ms.json`: both seats saw matching ticks and combat events, three shots defeated the helicopter, the rematch reset, and Solo Ready / seat switching still worked. The Rider stayed at the start during this combat check; the earlier continuously-pedaling P3 script reached the concrete barrier and lost its second lock, so that run is not treated as a combat acceptance result.

Both browser checks used an application-layer TCP WebSocket delay proxy with 50 ms each way; jitter and packet loss were not injected. SwiftShader results do not establish physical-GPU performance or final balance.

## Bridge cover slice

The second slice derives collision volumes from the shared 400 m bridge section. It contains 13 deck spans and 26 piers. The deck is 13 m wide, with its underside at 4.9 m and top at 5.7 m; the Rider lane remains below the deck. Client scene placement uses those same span and support positions.

Server and common tests verify that the deck blocks Gunner line of sight and the player missile path, clears lock and rejects fire while obstructed, cancels an enemy rocket warning when cover intervenes, and stops both projectile types at the deck without damaging either vehicle. Helicopter movement is also rejected when a proposed movement segment intersects a cover volume. `cargo test --workspace` passes 35 tests (Client 3, common 8, reconciliation 6, Server 18). Formatting, the WASM target check, Clippy, Server build, and Trunk build pass on the remote Rust toolchain.

The fixed-100 ms two-browser report above is the open-course P3 compatibility regression from slice one; it did not exercise the bridge. Actual bridge traversal and final escape integration are recorded below. SwiftShader does not establish physical-GPU performance or gameplay balance.

## Checkpoints, escape, and full-route integration

The shared course defines two ordered checkpoint gates near 394 m and 794 m, followed by a finish gate near 1,200 m. The Server accepts a checkpoint only when the actual previous-to-current bike segment crosses its road-width gate in the forward direction. Skipping a gate, crossing it backward or outside the road, or reaching the finish before both checkpoints cannot win. A bike reaching zero health on the same tick as the finish loses first. Escaping and destroying the helicopter both use `riders_victory`, with distinct `escaped` and `helicopter_destroyed` reasons. A replay resets route progress, health, ammunition, and victory state.

The Client renders cyan checkpoint arches and a gold finish arch. Its route HUD shows the current section, rounded meters remaining, and checkpoint count. Finished screens distinguish escape from helicopter destruction. The latest 1280 × 657 WebGL2 capture was visually checked: the Gunner reticle and lock ring are concentric at screen center, and the HUD uses ASCII separators supported by the bundled font.

The controlled no-network full-route test reaches 1,199.8 m. A separate Server test sends 20 Hz Rider inputs with synthetic 100 ms input/feedback delay and escapes at **1,199.8 m in 121.7 seconds (7,304 ticks)**. These are controlled engineering-controller measurements, not player balance results.

`escape-100ms.json` records a two-browser WASM run through a TCP WebSocket proxy set to 100 ms RTT (50 ms each way), using headless Chromium, SwiftShader, and WebGL2. Both clients observed all three sections and both ordered checkpoints. The bridge fire request was rejected as `obstructed` in the same combat event on both clients (Event ID 333, Tick 169,026); ammunition remained at 6. The escape event also matched across both clients (Event ID 352, server Tick 173,884) at 1,200.7 m, after 208.5 simulated seconds, with two checkpoints passed and 75 bike health. One receive-time section-boundary comparison shows different latest client ticks under the injected RTT; the final event and outcome snapshots match. No client JavaScript exceptions or console errors occurred.

Run `node tools/p4_escape_smoke.cjs` while the remote development Server and Trunk page are available to repeat the dual-client route, cover, escape, replay, and Solo seat-switch sequence.

Pressing `R` on both seats replayed the match: the route returned to `open_riverside` at 0 m, checkpoints returned to zero, both health values returned to 100, ammunition returned to 6, and the victory reason cleared. In the Solo run, `R` forced Ready and started the round; `V` switched the server-confirmed seat to Gunner while keeping the match Running. Solo health and ammunition stayed at 100 and 6, with no runtime errors.

Browser captures from this run: [`bridge-underpass-100ms.png`](bridge-underpass-100ms.png), [`gunner-under-bridge-100ms.png`](gunner-under-bridge-100ms.png), [`rider-escape-victory-100ms.png`](rider-escape-victory-100ms.png), [`gunner-escape-victory-100ms.png`](gunner-escape-victory-100ms.png), [`rider-escape-replay-100ms.png`](rider-escape-replay-100ms.png), [`gunner-escape-replay-100ms.png`](gunner-escape-replay-100ms.png), and [`solo-gunner-after-switch-p4.png`](solo-gunner-after-switch-p4.png). The JSON report includes event IDs, ticks, route transitions, checkpoint progress, replay state, WebGL2 details, and browser errors.

The final workspace run passed 39 tests (Client 3, common 9, reconciliation 6, Server 21), formatting check, the WASM target check, workspace Clippy, Server build, and Trunk build. The proxy did not inject jitter or packet loss. SwiftShader does not establish physical-GPU performance, final gameplay balance, or human playability.

# P4 route slice verification

This slice adds the shared 1,200.7 m course and its road-boundary collision data. The Rider is free to steer; the route projection only decides whether the next body position remains on the authored road.

The route contains 401.9 m of open riverside, 400.0 m of bridge-underpass road and 398.8 m of narrow bends. It uses 137 straight GLB pieces and 20 radius-10 m, 30-degree curve pieces. A reverse placement of the existing curve GLB supplies left turns. The headless Server reads the same course and collision code as Client prediction without loading rendering assets.

`cargo test -p riverside-common fixed_step_rider_can_follow_the_full_route_and_avoid_the_early_barrier -- --nocapture` traverses 1,197.9 m in 110.6 seconds at 6,633 fixed ticks. The controlled Rider follows the centerline through each bend and offsets around the existing concrete barrier. It stops within the test's 3 m completion threshold; ordered checkpoints and an authoritative escape finish are part of a later slice, so this is not the final escape completion time.

`route-start.png` is a 1280 × 657 remote Chromium / SwiftShader WebGL2 capture of the actual WASM Client, showing the course ahead from the starting area. A fixed 100 ms RTT two-client browser regression is recorded in `combat-compat-100ms.json`: both seats saw matching ticks and combat events, three shots defeated the helicopter, the rematch reset, and Solo Ready / seat switching still worked. The Rider stayed at the start during this combat check; the earlier continuously-pedaling P3 script reached the concrete barrier and lost its second lock, so that run is not treated as a combat acceptance result.

Both browser checks used an application-layer TCP WebSocket delay proxy with 50 ms each way; jitter and packet loss were not injected. SwiftShader results do not establish physical-GPU performance or final balance.

## Bridge cover slice

The second slice derives collision volumes from the shared 400 m bridge section. It contains 13 deck spans and 26 piers. The deck is 13 m wide, with its underside at 4.9 m and top at 5.7 m; the Rider lane remains below the deck. Client scene placement uses those same span and support positions.

Server and common tests verify that the deck blocks Gunner line of sight and the player missile path, clears lock and rejects fire while obstructed, cancels an enemy rocket warning when cover intervenes, and stops both projectile types at the deck without damaging either vehicle. Helicopter movement is also rejected when a proposed movement segment intersects a cover volume. `cargo test --workspace` passes 35 tests (Client 3, common 8, reconciliation 6, Server 18). Formatting, the WASM target check, Clippy, Server build, and Trunk build pass on the remote Rust toolchain.

The fixed-100 ms two-browser report above is the open-course P3 compatibility regression from slice one; it did not exercise the bridge. Actual bridge traversal and the final two-client section/escape result comparison remain part of the final P4 verification. SwiftShader does not establish physical-GPU performance or gameplay balance.

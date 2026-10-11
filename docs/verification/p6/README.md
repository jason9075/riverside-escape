# P6 network and browser verification

This folder contains reproducible browser/network evidence and screenshots for the P6 prototype checks. The tested interaction includes room join/ready, Rider steering and braking, Gunner aim/fire and hit flow, pointer lock, rematch, and the development-only one-player seat switch. All six Chrome network runs passed their recorded checks.

## Network matrix

The RTT values below are measured by the test client through the ordered TCP delay proxy. Jitter is independently applied in each direction with a fixed seed. The baseline run has scheduler outliers in its p95/p99 measurements, so its median is the more representative value for the zero-delay setup.

| Injected RTT | Per-direction jitter | Seed | Measured RTT p50 / p95 / p99 | Max Rider input backlog | Same-tick bike position / yaw error | Result |
|---:|---:|---:|---:|---:|---:|---|
| 0 ms | 0 ms | 137 | 2.7 / 44.8 / 44.8 ms | 0 | 0 m / 0° | Pass |
| 50 ms | 0 ms | 137 | 49.1 / 50.2 / 50.5 ms | 0 | 0 m / 0° | Pass |
| 100 ms | 0 ms | 137 | 98.3 / 99.4 / 100.3 ms | 0 | 0 m / 0° | Pass |
| 200 ms | 0 ms | 137 | 199.6 / 199.7 / 200.0 ms | 1 | 0 m / 0° | Pass |
| 100 ms | ±20 ms | 137 | 102.1 / 126.1 / 128.0 ms | 1 | 0 m / 0° | Pass |
| 200 ms | ±20 ms | 237 | 200.7 / 229.7 / 236.8 ms | 1 | 0 m / 0° | Pass |

Every report asserts that the Rider input acknowledgement backlog drains, no reconciliation history misses occur, same-tick state stays within the 2 cm/yaw consistency checks, the solo seat switch keeps the match running, and the browser records no runtime errors. The no-delay p95/p99 reflect periodic host scheduling spikes rather than injected network delay; see the raw samples in [rtt-0.json](rtt-0.json).

The delay proxy preserves TCP byte order and delivery. It does not model packet loss. The P5 short disconnect/reconnect path is documented in the [reconnect smoke report](../p5/reconnect-smoke-report.json) and [browser reconnect report](../p5/browser-reconnect-report.json), but 1% TCP loss remains unverified because an isolated network namespace/netem could not be enabled in this environment.

## Browser and rendering environment

- Network and gameplay smoke: headless Chrome 155.0.8059.39, Linux, WebGL2 through ANGLE/SwiftShader.
- Firefox 157.0.1 starts through WebDriver, but its headless session returns `null` for a standalone WebGL2 canvas. Game input and rendering therefore could not be tested there; the result is recorded in [firefox-smoke.json](firefox-smoke.json).
- SwiftShader is not physical GPU evidence. Frame-time, real GPU FPS, and Gunner camera feel on hardware remain unverified.
- The tests use scripted input and controlled routes. They validate the prototype networking path; they do not establish final player-facing balance or subjective feel.

## Reproduction

Run the server with `RIVERSIDE_DEV_SOLO=1` and serve the built WebAssembly client. For example, reproduce the 100 ms / ±20 ms jitter case with:

```sh
P3_RTT_MS=100 P3_JITTER_MS=20 P3_SEED=137 \
P3_SITE_URL=http://127.0.0.1:8081/ \
P3_ARTIFACT_DIR=docs/verification/p6/rtt-100-jitter-20-seed-137 \
P3_REPORT=docs/verification/p6/rtt-100-jitter-20-seed-137.json \
node tools/p3_smoke.cjs
```

Use the other matrix row values for `P3_RTT_MS`, `P3_JITTER_MS`, and `P3_SEED`. The load test is `tools/p6_server_load.cjs` and runs four rooms/eight WebSocket clients against its own temporary server port.

## Ten-minute server load

The Linux load run completed 600.091 seconds with four rooms and eight WebSocket clients sending 20 Hz Rider/Gunner inputs. All scripted gates passed. The server tick rate stayed near 60 Hz (p50 59.8 Hz across 10-second samples); final tick duration was 47 / 72 / 88 μs at p50 / p95 / p99, against the 8,000 / 16,700 μs prototype limits. Each client received about 19.93 snapshots/s; aggregate delivery was 159.32 snapshots/s and 201,817 payload bytes/s. RSS drift across samples was 24.85 MB. No server or client errors were recorded.

After all eight clients left, connected sessions, reconnect reservations, and projectiles were all zero. The four empty rooms were retained under the existing 10-minute reconnect-retention policy; this run confirms the room count stayed bounded at four, but does not wait another 10 minutes to observe expiration. The complete time series and per-client counts are in [server-load-10m.json](server-load-10m.json).

Reproduce from the repository root on Linux with:

```sh
P6_LOAD_SECONDS=600 \
P6_LOAD_REPORT=docs/verification/p6/server-load-10m.json \
RIVERSIDE_SERVER_BIN=target/debug/riverside-server \
node tools/p6_server_load.cjs
```

# P3 browser verification

`smoke-report.json` records a complete dual-client and solo-seat run against the remote WASM client and authoritative server. The test used two isolated Headless Chromium 154 / SwiftShader clients through a loopback WebSocket proxy set to 100 ms RTT (50 ms each direction, no jitter or packet loss).

The first client created room `R00002`; its lobby displayed an invite URL containing that code, and the second client joined through it. Both selected their roles from the Rider/Gunner cards before Ready. With Gunner pointer lock active, the shooter acquired the server lock and fired three single shots. The server reported helicopter health `100 → 66 → 32 → 0`, with matching fire and impact event IDs on Rider and Gunner at the same snapshot ticks. Both clients received `riders_victory` at tick 2,076, both Ready flags cleared, and bike health was 75 after one rocket hit. Both seats readied again; the rematch countdown started at five seconds and the next Running snapshot had bike and helicopter health 100 and six missiles. The solo client forced Ready with `R`, switched to Gunner with `V`, and retained its health and ammo. All three browser clients had no JavaScript exceptions or console errors.

This regression was rerun after aligning gameplay yaw, rendered bike/chase orientation, and Gunner aim coordinates. It confirms room creation/join, explicit role cards, the corrected Gunner world-space aim, rematch, and Solo controls at fixed 100 ms RTT; it does not measure a moving Rider's tracking stability.

Run the check from the project root after starting the server with `RIVERSIDE_DEV_SOLO=1` and serving the WASM page at `http://127.0.0.1:8080/`:

```sh
node tools/p3_smoke.cjs
```

The script starts its own 100 ms proxy and isolated Chrome profiles, drives the actual canvas mouse/key input, and writes the JSON report and screenshots under this directory. Set `CHROME_BIN` if Chromium is installed at a different path. The server and Trunk must be reachable on their documented ports; browser control ports 9232–9234 and proxy port 3102 must be free.

Screenshots:

- [Rider running](rider-running.png)
- [Gunner sees the helicopter](gunner-helicopter-visible.png)
- [Gunner missile hit](gunner-missile-hit.png)
- [Rider victory](rider-victory.png)
- [Gunner victory](gunner-victory.png)
- [Solo Gunner after switching seats](solo-gunner-after-switch.png)

These results validate the prototype on remote SwiftShader with fixed 100 ms RTT only. They do not establish physical-GPU performance, other browser behavior, jitter/loss tolerance, or final gameplay balance.

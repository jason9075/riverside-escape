const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const WebSocket = require('ws');

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor(check, label, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await sleep(50);
  }
  throw new Error(`Timed out waiting for ${label}`);
}

class DevTools {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 0;
    this.pending = new Map();
    this.onEvent = () => {};
    socket.on('message', (data) => {
      const message = JSON.parse(data.toString());
      if (message.id) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        clearTimeout(pending.timer);
        if (message.error) pending.reject(new Error(JSON.stringify(message.error)));
        else pending.resolve(message.result || {});
      } else {
        this.onEvent(message.method, message.params || {});
      }
    });
  }

  static async connect(url) {
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      socket.once('open', resolve);
      socket.once('error', reject);
    });
    return new DevTools(socket);
  }

  command(method, params = {}) {
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP timeout: ${method}`));
      }, 8000);
      this.pending.set(id, { resolve, reject, timer });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  close() {
    if (this.socket.readyState === WebSocket.OPEN) this.socket.close();
  }
}

async function fetchJson(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return response.json();
}

async function launchBrowser({ port, url, profile }) {
  const browser = spawn(process.env.CHROME_BIN || '/usr/bin/google-chrome', [
    '--headless=new', '--no-sandbox', '--disable-dev-shm-usage',
    '--enable-webgl', '--ignore-gpu-blocklist', '--use-gl=angle',
    '--use-angle=swiftshader', '--enable-unsafe-swiftshader',
    '--hide-scrollbars', '--window-size=1280,800', '--force-device-scale-factor=1',
    `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, 'about:blank',
  ], { stdio: 'ignore' });
  const endpoint = `http://127.0.0.1:${port}`;
  let target;
  await waitFor(async () => {
    try {
      target = (await fetchJson(`${endpoint}/json/list`)).find((item) => item.type === 'page');
      return target;
    } catch {
      return false;
    }
  }, `Chrome DevTools ${port}`);
  const cdp = await DevTools.connect(target.webSocketDebuggerUrl);
  const client = {
    port,
    browser,
    cdp,
    welcome: null,
    seat: null,
    latest: null,
    byTick: new Map(),
    exceptions: [],
    consoleErrors: [],
  };
  cdp.onEvent = (method, params) => {
    if (method === 'Runtime.exceptionThrown') {
      client.exceptions.push(params.exceptionDetails?.exception?.description
        || params.exceptionDetails?.text || 'JavaScript exception');
    }
    if (method === 'Log.entryAdded' && params.entry?.level === 'error') {
      client.consoleErrors.push(params.entry.text);
    }
    if (method !== 'Network.webSocketFrameReceived') return;
    try {
      const message = JSON.parse(params.response.payloadData);
      if (message.type === 'welcome') {
        client.welcome = message.payload;
        client.seat = message.payload.seat;
      }
      if (message.type === 'seat_changed') client.seat = message.payload.seat;
      if (message.type === 'snapshot') {
        client.latest = message.payload;
        client.byTick.set(message.payload.tick, message.payload);
        while (client.byTick.size > 1200) {
          client.byTick.delete(client.byTick.keys().next().value);
        }
      }
    } catch { /* Ignore protocol frames not carrying JSON messages. */ }
  };
  await cdp.command('Page.enable');
  await cdp.command('Runtime.enable');
  await cdp.command('Log.enable');
  await cdp.command('Network.enable');
  await cdp.command('Page.addScriptToEvaluateOnNewDocument', {
    source: `
      Object.defineProperty(Document.prototype, 'hasFocus', { configurable: true, value: () => true });
      (() => {
        const NativeWebSocket = window.WebSocket;
        function InstrumentedWebSocket(url, protocols) {
          const socket = protocols === undefined
            ? new NativeWebSocket(url)
            : new NativeWebSocket(url, protocols);
          window.__riversideSocket = socket;
          socket.addEventListener('message', (event) => {
            try {
              const message = JSON.parse(event.data);
              if (message.type === 'snapshot') window.__p4Latest = message.payload;
            } catch {}
          });
          return socket;
        }
        InstrumentedWebSocket.prototype = NativeWebSocket.prototype;
        Object.setPrototypeOf(InstrumentedWebSocket, NativeWebSocket);
        window.WebSocket = InstrumentedWebSocket;
        window.__p4Send = (message) => {
          const socket = window.__riversideSocket;
          if (!socket || socket.readyState !== NativeWebSocket.OPEN) return false;
          socket.send(JSON.stringify(message));
          return true;
        };
      })();
    `,
  });
  await cdp.command('Page.navigate', { url });
  await waitFor(() => client.welcome, `Welcome on ${port}`);
  await cdp.command('Page.bringToFront');
  await cdp.command('Runtime.evaluate', { expression: 'document.querySelector("canvas")?.focus()' });
  return client;
}

async function sendKey(client, character) {
  await client.cdp.command('Page.bringToFront');
  const upper = character.toUpperCase();
  const common = {
    key: character,
    code: `Key${upper}`,
    windowsVirtualKeyCode: upper.charCodeAt(0),
    nativeVirtualKeyCode: upper.charCodeAt(0),
  };
  await client.cdp.command('Input.dispatchKeyEvent', { type: 'keyDown', ...common });
  await sleep(200);
  await client.cdp.command('Input.dispatchKeyEvent', { type: 'char', text: character, ...common });
  await sleep(40);
  await client.cdp.command('Input.dispatchKeyEvent', { type: 'keyUp', ...common });
}

async function sendProtocol(client, message) {
  const expression = `window.__p4Send(${JSON.stringify(message)})`;
  const result = await client.cdp.command('Runtime.evaluate', {
    expression,
    returnByValue: true,
  });
  if (result.result?.value !== true) throw new Error(`WebSocket input injection failed: ${message.type}`);
}

async function capture(client, filename) {
  await sleep(100);
  const screenshot = await client.cdp.command('Page.captureScreenshot', {
    format: 'png', captureBeyondViewport: false,
  });
  fs.writeFileSync(filename, Buffer.from(screenshot.data, 'base64'));
}

async function dispose(client) {
  if (!client) return;
  try { client.cdp.close(); } catch { /* The browser may already be gone. */ }
  client.browser.kill('SIGTERM');
}

function wrapAngle(angle) {
  return ((angle + Math.PI) % (2 * Math.PI) + 2 * Math.PI) % (2 * Math.PI) - Math.PI;
}

function angleDelta(from, to) {
  return wrapAngle(to - from);
}

function buildCourse() {
  const curveLength = 10 * Math.PI / 6;
  const points = [{ x: 0, z: 0, yaw: 0, distance: 0 }];
  const curves = [];
  let x = 0;
  let z = 2;
  let yaw = 0;
  let distance = 0;
  const localToWorld = (localX, localZ, angle) => ({
    x: Math.cos(angle) * localX - Math.sin(angle) * localZ,
    z: Math.sin(angle) * localX + Math.cos(angle) * localZ,
  });
  const straight = (count) => {
    for (let index = 0; index < count; index += 1) {
      for (let step = 1; step <= 8; step += 1) {
        const offset = localToWorld(0, -step, yaw);
        points.push({ x: x + offset.x, z: z + offset.z, yaw, distance: distance + step });
      }
      z -= 8 * Math.cos(yaw);
      x += 8 * Math.sin(yaw);
      distance += 8;
    }
  };
  const curve = (direction) => {
    const start = distance;
    curves.push({ start, end: start + curveLength, direction });
    for (let along = 0.5; along < curveLength; along += 0.5) {
      const angle = along / 10;
      const localX = direction * 10 * (1 - Math.cos(angle));
      const localZ = -10 * Math.sin(angle);
      const offset = localToWorld(localX, localZ, yaw);
      points.push({
        x: x + offset.x,
        z: z + offset.z,
        yaw: wrapAngle(yaw + direction * angle),
        distance: start + along,
      });
    }
    const offset = localToWorld(
      direction * 10 * (1 - Math.cos(Math.PI / 6)),
      -10 * Math.sin(Math.PI / 6),
      yaw,
    );
    x += offset.x;
    z += offset.z;
    distance += curveLength;
    yaw = wrapAngle(yaw + direction * Math.PI / 6);
    points.push({ x, z, yaw, distance });
  };
  let openStart;
  for (let index = 0; index < 4; index += 1) {
    const count = [11, 11, 11, 12][index];
    const lead = Math.floor(count / 2);
    straight(lead);
    const direction = index % 2 === 0 ? 1 : -1;
    curve(direction);
    straight(count - lead);
    curve(-direction);
  }
  openStart = distance;
  straight(50);
  const bridgeEnd = distance;
  for (let index = 0; index < 6; index += 1) {
    const direction = index % 2 === 0 ? 1 : -1;
    straight(5);
    curve(direction);
    straight(2);
    curve(-direction);
  }
  const pointAt = (targetDistance) => {
    const target = Math.max(0, Math.min(distance, targetDistance));
    let low = 0;
    let high = points.length - 1;
    while (low < high) {
      const middle = Math.floor((low + high) / 2);
      if (points[middle].distance < target) low = middle + 1;
      else high = middle;
    }
    const b = points[low];
    const a = points[Math.max(0, low - 1)];
    const span = b.distance - a.distance;
    const alpha = span > 1e-7 ? Math.max(0, Math.min(1, (target - a.distance) / span)) : 0;
    return {
      x: a.x + (b.x - a.x) * alpha,
      z: a.z + (b.z - a.z) * alpha,
      yaw: wrapAngle(a.yaw + angleDelta(a.yaw, b.yaw) * alpha),
    };
  };
  const curvatureAt = (atDistance) => {
    const piece = curves.find((item) => atDistance >= item.start && atDistance < item.end);
    return piece ? piece.direction / 10 : 0;
  };
  return { length: distance, openEnd: openStart, bridgeEnd, pointAt, curvatureAt };
}

function routeInput(course, snapshot) {
  const progress = snapshot.route_distance_m;
  const lookahead = course.pointAt(Math.min(course.length, progress + 6));
  // Clear the barrier before the first bend; carrying the lateral offset into
  // the curve compounds the road's own rightward turn under delayed feedback.
  const detour = progress >= 30 && progress < 40 ? 0.75 : 0;
  const targetX = lookahead.x + Math.cos(lookahead.yaw) * detour;
  const targetZ = lookahead.z + Math.sin(lookahead.yaw) * detour;
  const desiredYaw = Math.atan2(targetX - snapshot.bike.x, snapshot.bike.z - targetZ);
  const headingError = angleDelta(snapshot.bike.yaw, desiredYaw);
  const curvature = course.curvatureAt(progress + 2);
  const routeCenter = course.pointAt(progress);
  const lateral = (snapshot.bike.x - routeCenter.x) * Math.cos(routeCenter.yaw)
    + (snapshot.bike.z - routeCenter.z) * Math.sin(routeCenter.yaw);
  let steer = Math.max(-1, Math.min(1, (curvature + headingError * 0.65) / 0.148));
  if (lateral > 1.25) steer = Math.min(steer, -0.5);
  if (lateral < -1.25) steer = Math.max(steer, 0.5);
  return {
    pedal_held: true,
    brake_held: false,
    steer,
  };
}

function gunnerInput(snapshot) {
  const bike = snapshot.bike;
  const helicopter = snapshot.helicopter;
  if (!helicopter) return { yaw: 0, pitch: 0, aim_held: false };
  const rearYaw = bike.yaw + Math.PI;
  const origin = {
    x: bike.x - Math.sin(rearYaw) * 1.03,
    y: 1.84,
    z: bike.z - Math.cos(rearYaw) * 1.03,
  };
  const dx = helicopter.position.x - origin.x;
  const dy = helicopter.position.y - origin.y;
  const dz = helicopter.position.z - origin.z;
  return {
    yaw: wrapAngle(Math.atan2(-dx, -dz) - bike.yaw - Math.PI),
    pitch: Math.atan2(dy, Math.hypot(dx, dz)),
    aim_held: true,
  };
}

function distanceToHelicopter(snapshot) {
  if (!snapshot.helicopter) return Infinity;
  const bike = snapshot.bike;
  const rearYaw = bike.yaw + Math.PI;
  const dx = snapshot.helicopter.position.x - (bike.x - Math.sin(rearYaw) * 1.03);
  const dy = snapshot.helicopter.position.y - 1.84;
  const dz = snapshot.helicopter.position.z - (bike.z - Math.cos(rearYaw) * 1.03);
  return Math.hypot(dx, dy, dz);
}

function sharedEvent(rider, gunner, type, predicate) {
  for (const [tick, left] of rider.byTick) {
    const right = gunner.byTick.get(tick);
    if (!right) continue;
    const event = left.combat_events?.find((item) => item.kind?.type === type && predicate(item));
    const other = event && right.combat_events?.find((item) => item.event_id === event.event_id);
    if (event && other && other.kind?.type === type) return { tick, left, right, event, other };
  }
  return false;
}

(async () => {
  const root = path.resolve(__dirname, '..');
  const artifactDir = path.join(root, 'docs/verification/p4');
  fs.mkdirSync(artifactDir, { recursive: true });
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'riverside-p4-'));
  const proxy = spawn('node', [path.join(root, 'tools/ws_latency_proxy.js'),
    '--rtt-ms', '100', '--listen-port', '3104'], { stdio: 'ignore' });
  let rider, gunner, solo;
  const course = buildCourse();
  const report = {
    fixed_rtt_ms: 100,
    one_way_delay_ms: 50,
    route_length_m: Number(course.length.toFixed(1)),
    section_boundaries_m: {
      open_riverside_end: Number(course.openEnd.toFixed(1)),
      bridge_underpass_end: Number(course.bridgeEnd.toFixed(1)),
    },
    section_transitions: [],
    checkpoints: [],
    bridge_fire_attempts: [],
  };
  try {
    await sleep(500);
    const dualUrl = 'http://127.0.0.1:8080/?server=ws%3A%2F%2F127.0.0.1%3A3104%2Fws';
    rider = await launchBrowser({ port: 9242, url: dualUrl, profile: path.join(tempRoot, 'rider') });
    gunner = await launchBrowser({ port: 9243, url: dualUrl, profile: path.join(tempRoot, 'gunner') });
    if (rider.seat !== 'rider' || gunner.seat !== 'gunner') throw new Error('Dual seats were not assigned');
    await waitFor(() => rider.latest?.phase === 'ready_check' && gunner.latest?.phase === 'ready_check', 'dual Ready Check');
    await sendKey(rider, 'r');
    await sendKey(gunner, 'r');
    await waitFor(() => rider.latest?.phase === 'running' && gunner.latest?.phase === 'running', 'dual Running', 10000);
    report.started = { rider_tick: rider.latest.tick, gunner_tick: gunner.latest.tick };
    const startedAt = Date.now();
    let riderSequence = 1_000_000;
    let gunnerSequence = 1_000_000;
    let previousSection = null;
    let previousCheckpoints = 0;
    let coverFireRequested = false;
    let coverFireResponseObserved = false;
    let coverFireEvent = false;
    let bridgeCaptured = false;
    let lastSentTick = 0;

    while (Date.now() - startedAt < 300_000) {
      const snapshot = rider.latest;
      if (!snapshot) {
        await sleep(50);
        continue;
      }
      if (snapshot.phase === 'finished') break;
      if (snapshot.phase === 'running' && snapshot.tick !== lastSentTick) {
        lastSentTick = snapshot.tick;
        const input = routeInput(course, snapshot);
        await sendProtocol(rider, {
          type: 'rider_input',
          payload: { sequence: riderSequence++, tick: snapshot.tick, input },
        });
        if (gunner.latest?.phase === 'running') {
          await sendProtocol(gunner, {
            type: 'gunner_input',
            payload: {
              sequence: gunnerSequence++,
              tick: gunner.latest.tick,
              input: gunnerInput(gunner.latest),
            },
          });
        }
        if (snapshot.route_section !== previousSection) {
          previousSection = snapshot.route_section;
          report.section_transitions.push({
            tick: snapshot.tick,
            section: snapshot.route_section,
            distance_m: Number(snapshot.route_distance_m.toFixed(1)),
            rider_and_gunner_same_tick: rider.latest.tick === gunner.latest?.tick,
          });
          console.log(`[p4] rider entered ${snapshot.route_section} at ${snapshot.route_distance_m.toFixed(1)} m`);
        }
        if (snapshot.route_checkpoints_passed !== previousCheckpoints) {
          previousCheckpoints = snapshot.route_checkpoints_passed;
          report.checkpoints.push({
            tick: snapshot.tick,
            passed: snapshot.route_checkpoints_passed,
            distance_m: Number(snapshot.route_distance_m.toFixed(1)),
          });
          console.log(`[p4] server confirmed checkpoint ${snapshot.route_checkpoints_passed}`);
        }
        if (!bridgeCaptured
          && snapshot.route_section === 'bridge_underpass'
          && snapshot.route_distance_m > course.openEnd + 12) {
          await capture(rider, path.join(artifactDir, 'bridge-underpass-100ms.png'));
          await capture(gunner, path.join(artifactDir, 'gunner-under-bridge-100ms.png'));
          bridgeCaptured = true;
          console.log('[p4] captured both WASM clients beneath bridge cover');
        }
        if (!coverFireRequested
          && snapshot.route_section === 'bridge_underpass'
          && snapshot.route_distance_m > course.openEnd + 80
          && distanceToHelicopter(gunner.latest) < 120) {
          await sendProtocol(gunner, {
            type: 'fire_request',
            payload: { request_id: 1, tick: gunner.latest.tick },
          });
          coverFireRequested = true;
          console.log('[p4] requested one Gunner shot from deep beneath the bridge');
        }
      }
      if (coverFireRequested && !coverFireResponseObserved) {
        const rejection = sharedEvent(
          rider,
          gunner,
          'gunner_fire_rejected',
          (event) => event.kind.request_id === 1,
        );
        if (rejection) {
          coverFireResponseObserved = true;
          report.bridge_fire_attempts.push({
            request_id: rejection.event.kind.request_id,
            reason: rejection.event.kind.reason,
            tick: rejection.tick,
          });
          if (rejection.event.kind.reason === 'obstructed') {
            coverFireEvent = rejection;
            report.bridge_fire_rejection = {
              tick: rejection.tick,
              event_id: rejection.event.event_id,
              reason: rejection.event.kind.reason,
              same_event_on_both_clients: true,
              gunner_ammo: rejection.left.gunner_ammo,
              lock_ticks: rejection.left.gunner_lock_progress_ticks,
            };
            console.log('[p4] matching Gunner fire rejection confirmed by both clients');
          }
        }
      }
      await sleep(50);
    }

    if (!bridgeCaptured) throw new Error('The Rider did not reach the bridge capture point');
    if (!report.bridge_fire_rejection) throw new Error('Gunner fire was not rejected by bridge cover');
    if (rider.latest.phase !== 'finished' || gunner.latest?.phase !== 'finished') {
      throw new Error(`Route did not finish in time: ${JSON.stringify({ rider: rider.latest, gunner: gunner.latest })}`);
    }
    const escape = await waitFor(() => sharedEvent(
      rider,
      gunner,
      'match_finished',
      (event) => event.kind.result === 'riders_victory' && event.kind.victory_reason === 'escaped',
    ), 'matching dual-client escape victory', 5000);
    report.escape = {
      event_id: escape.event.event_id,
      tick: escape.tick,
      event_tick: escape.event.tick,
      reason: escape.event.kind.victory_reason,
      same_event_on_both_clients: true,
      route_length_m: escape.left.route_length_m,
      rider: {
        result: escape.left.match_result,
        reason: escape.left.match_victory_reason,
        checkpoints_passed: escape.left.route_checkpoints_passed,
        bike_health: escape.left.bike_health,
      },
      gunner: {
        result: escape.right.match_result,
        reason: escape.right.match_victory_reason,
        checkpoints_passed: escape.right.route_checkpoints_passed,
        bike_health: escape.right.bike_health,
      },
      elapsed_seconds_from_running: Number(((escape.event.tick - report.started.rider_tick) / 60).toFixed(1)),
      browser_runtime_errors: { rider: rider.exceptions, gunner: gunner.exceptions },
    };
    if (escape.left.route_checkpoints_passed !== 2 || escape.right.route_checkpoints_passed !== 2
      || escape.left.match_victory_reason !== 'escaped' || escape.right.match_victory_reason !== 'escaped') {
      throw new Error(`Escape snapshots disagree: ${JSON.stringify(report.escape)}`);
    }
    report.elapsed_seconds_from_server_tick = report.escape.elapsed_seconds_from_running;
    console.log(`[p4] both clients confirmed escape at tick ${escape.event.tick}`);
    await Promise.all([
      capture(rider, path.join(artifactDir, 'rider-escape-victory-100ms.png')),
      capture(gunner, path.join(artifactDir, 'gunner-escape-victory-100ms.png')),
    ]);

    await sendKey(rider, 'r');
    await sendKey(gunner, 'r');
    const reset = await waitFor(() => rider.latest?.phase === 'countdown'
      && gunner.latest?.phase === 'countdown', 'both clients reset for replay', 5000);
    report.replay_reset = {
      phase: reset.phase,
      rider_tick: rider.latest.tick,
      gunner_tick: gunner.latest.tick,
      route_section: rider.latest.route_section,
      route_distance_m: rider.latest.route_distance_m,
      checkpoints_passed: rider.latest.route_checkpoints_passed,
      match_result: rider.latest.match_result,
      victory_reason: rider.latest.match_victory_reason,
      bike_health: rider.latest.bike_health,
      helicopter_health: rider.latest.helicopter_health,
      ammo: rider.latest.gunner_ammo,
    };
    if (report.replay_reset.route_section !== 'open_riverside'
      || report.replay_reset.route_distance_m !== 0
      || report.replay_reset.checkpoints_passed !== 0
      || report.replay_reset.match_result || report.replay_reset.victory_reason
      || report.replay_reset.bike_health !== 100
      || report.replay_reset.helicopter_health !== 100
      || report.replay_reset.ammo !== 6) throw new Error(`Replay did not reset: ${JSON.stringify(report.replay_reset)}`);
    await waitFor(() => rider.latest?.phase === 'running' && gunner.latest?.phase === 'running', 'replay countdown completed', 10000);
    report.replay_running = { rider_tick: rider.latest.tick, gunner_tick: gunner.latest.tick };
    await Promise.all([
      capture(rider, path.join(artifactDir, 'rider-escape-replay-100ms.png')),
      capture(gunner, path.join(artifactDir, 'gunner-escape-replay-100ms.png')),
    ]);
    const dualClientErrors = {
      rider: { exceptions: rider.exceptions, console_errors: rider.consoleErrors },
      gunner: { exceptions: gunner.exceptions, console_errors: gunner.consoleErrors },
    };
    if (dualClientErrors.rider.exceptions.length || dualClientErrors.rider.console_errors.length
      || dualClientErrors.gunner.exceptions.length || dualClientErrors.gunner.console_errors.length) {
      throw new Error(`Dual browser errors: ${JSON.stringify(dualClientErrors)}`);
    }
    await dispose(rider); rider = null;
    await dispose(gunner); gunner = null;
    await sleep(400);

    solo = await launchBrowser({
      port: 9244,
      url: 'http://127.0.0.1:8080/?solo=1&server=ws%3A%2F%2F127.0.0.1%3A3104%2Fws',
      profile: path.join(tempRoot, 'solo'),
    });
    await waitFor(() => solo.latest?.phase === 'ready_check', 'solo Ready Check');
    await sendKey(solo, 'r');
    await waitFor(() => solo.latest?.phase === 'running', 'solo force-ready countdown', 10000);
    await sendKey(solo, 'v');
    await waitFor(() => solo.seat === 'gunner', 'server-confirmed solo seat switch');
    report.solo = {
      seat: solo.seat,
      phase: solo.latest.phase,
      route_section: solo.latest.route_section,
      route_distance_m: solo.latest.route_distance_m,
      checkpoints_passed: solo.latest.route_checkpoints_passed,
      bike_health: solo.latest.bike_health,
      ammo: solo.latest.gunner_ammo,
      runtime_errors: solo.exceptions,
    };
    await capture(solo, path.join(artifactDir, 'solo-gunner-after-switch-p4.png'));
    if (solo.exceptions.length || solo.consoleErrors.length) {
      throw new Error(`Solo browser errors: ${JSON.stringify({ exceptions: solo.exceptions, console: solo.consoleErrors })}`);
    }
    report.client_errors = {
      dual: dualClientErrors,
      solo_exceptions: solo.exceptions,
      solo_console_errors: solo.consoleErrors,
    };
    report.canvas_debug = await solo.cdp.command('Runtime.evaluate', {
      expression: `(() => { const canvas = document.querySelector('canvas'); const gl = canvas?.getContext('webgl2'); return { width: canvas?.width, height: canvas?.height, webgl2: !!gl, version: gl?.getParameter(gl.VERSION) }; })()`,
      returnByValue: true,
    }).then((value) => value.result?.value);
    fs.writeFileSync(path.join(artifactDir, 'escape-100ms.json'), `${JSON.stringify(report, null, 2)}\n`);
    console.log(JSON.stringify(report, null, 2));
  } catch (error) {
    report.error = error.stack || String(error);
    report.live_state = {
      rider: rider?.latest,
      gunner: gunner?.latest,
      solo: solo?.latest,
      runtime_errors: {
        rider: rider?.exceptions,
        gunner: gunner?.exceptions,
        solo: solo?.exceptions,
      },
    };
    fs.writeFileSync(path.join(artifactDir, 'escape-100ms-failure.json'), `${JSON.stringify(report, null, 2)}\n`);
    console.error(error.stack || String(error));
    console.error(JSON.stringify(report, null, 2));
    process.exitCode = 1;
  } finally {
    await dispose(rider);
    await dispose(gunner);
    await dispose(solo);
    proxy.kill('SIGTERM');
  }
})();

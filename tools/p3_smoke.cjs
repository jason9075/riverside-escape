const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const WebSocket = require('ws');

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function fetchJson(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return response.json();
}

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

async function launchBrowser({ port, url, profile }) {
  const browser = spawn(process.env.CHROME_BIN || '/usr/bin/google-chrome', [
    '--headless=new', '--no-sandbox', '--disable-dev-shm-usage',
    '--enable-webgl', '--ignore-gpu-blocklist', '--use-gl=angle',
    '--use-angle=swiftshader', '--enable-unsafe-swiftshader',
    '--hide-scrollbars', '--window-size=1280,800', '--force-device-scale-factor=1',
    `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, 'about:blank',
  ], { stdio: 'ignore' });
  const endpoint = `http://127.0.0.1:${port}`;
  const browserVersion = await waitFor(async () => {
    try {
      return await fetchJson(`${endpoint}/json/version`);
    } catch {
      return false;
    }
  }, `Chrome DevTools version ${port}`);
  let target;
  await waitFor(async () => {
    try {
      const targets = await fetchJson(`${endpoint}/json/list`);
      target = targets.find((item) => item.type === 'page');
      return target;
    } catch {
      return false;
    }
  }, `Chrome DevTools ${port}`);
  const cdp = await DevTools.connect(target.webSocketDebuggerUrl);
  const client = {
    port,
    browser,
    browserVersion,
    cdp,
    seat: null,
    welcome: null,
    seatSwapProposals: [],
    latest: null,
    byTick: new Map(),
    exceptions: [],
    consoleErrors: [],
    cursor: { x: 0, y: 0 },
    mouseEventCount: 0,
    mouseDeltaX: 0,
    mouseDeltaY: 0,
    rightDown: false,
  };
  cdp.onEvent = (method, params) => {
    if (method === 'Runtime.exceptionThrown') {
      client.exceptions.push({
        text: params.exceptionDetails?.text,
        description: params.exceptionDetails?.exception?.description,
        url: params.exceptionDetails?.url,
        line: params.exceptionDetails?.lineNumber,
        column: params.exceptionDetails?.columnNumber,
      });
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
      if (message.type === 'seat_assigned') client.seat = message.payload.seat;
      if (message.type === 'seat_changed') client.seat = message.payload.seat;
      if (message.type === 'seat_swap_proposed') client.seatSwapProposals.push(message.payload);
      if (message.type === 'seat_swap_completed') client.seat = message.payload.seat;
      if (message.type === 'snapshot') {
        client.latest = message.payload;
        client.byTick.set(message.payload.tick, message.payload);
        if (client.byTick.size > 1000) client.byTick.delete(client.byTick.keys().next().value);
      }
    } catch { /* Ignore non-protocol WebSocket frames. */ }
  };
  await cdp.command('Page.enable');
  await cdp.command('Runtime.enable');
  await cdp.command('Log.enable');
  await cdp.command('Network.enable');
  await cdp.command('Page.addScriptToEvaluateOnNewDocument', {
    source: `
      Object.defineProperty(Document.prototype, 'hasFocus', { configurable: true, value: () => true });
      window.__p3MouseMoves = [];
      document.addEventListener('mousemove', (event) => window.__p3MouseMoves.push({
        movementX: event.movementX, movementY: event.movementY,
        clientX: event.clientX, clientY: event.clientY,
        locked: document.pointerLockElement !== null,
      }), true);
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
  const code = `Key${upper}`;
  const keyCode = upper.charCodeAt(0);
  const common = { key: character, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode };
  await client.cdp.command('Input.dispatchKeyEvent', { type: 'keyDown', ...common });
  await sleep(200);
  await client.cdp.command('Input.dispatchKeyEvent', { type: 'char', text: character, ...common });
  await sleep(40);
  await client.cdp.command('Input.dispatchKeyEvent', { type: 'keyUp', ...common });
}

async function holdKey(client, key, durationMs) {
  await client.cdp.command('Page.bringToFront');
  const code = key === ' ' ? 'Space' : `Key${key.toUpperCase()}`;
  const keyCode = key === ' ' ? 32 : key.toUpperCase().charCodeAt(0);
  const common = {
    key: key === ' ' ? ' ' : key.toLowerCase(),
    code,
    windowsVirtualKeyCode: keyCode,
    nativeVirtualKeyCode: keyCode,
  };
  await client.cdp.command('Input.dispatchKeyEvent', { type: 'keyDown', ...common });
  await sleep(durationMs);
  await client.cdp.command('Input.dispatchKeyEvent', { type: 'keyUp', ...common });
}

async function sendFunctionKey(client, key, virtualKey) {
  await client.cdp.command('Page.bringToFront');
  const common = { key, code: key, windowsVirtualKeyCode: virtualKey, nativeVirtualKeyCode: virtualKey };
  await client.cdp.command('Input.dispatchKeyEvent', { type: 'keyDown', ...common });
  await sleep(40);
  await client.cdp.command('Input.dispatchKeyEvent', { type: 'keyUp', ...common });
}

async function browserDiagnostics(client) {
  const result = await client.cdp.command('Runtime.evaluate', {
    expression: 'document.getElementById("network-diagnostics")?.textContent',
    returnByValue: true,
  });
  const text = result.result?.value;
  if (!text) return null;
  try { return JSON.parse(text); } catch { return null; }
}

async function measureProxyRtt(address, sampleCount = 30) {
  const socket = new WebSocket(address);
  await new Promise((resolve, reject) => {
    socket.once('open', resolve);
    socket.once('error', reject);
  });
  await waitFor(() => socket.readyState === WebSocket.OPEN, 'RTT probe connection');
  const samples = [];
  for (let index = 0; index < sampleCount; index += 1) {
    const start = performance.now();
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('RTT ping timed out')), 5000);
      socket.once('pong', () => {
        clearTimeout(timer);
        samples.push(performance.now() - start);
        resolve();
      });
      socket.ping(Buffer.from(String(index)));
    });
  }
  socket.close();
  const sorted = [...samples].sort((left, right) => left - right);
  const percentile = (rank) => sorted[Math.min(sorted.length - 1,
    Math.ceil(sorted.length * rank) - 1)];
  return {
    samples: samples.length,
    min_ms: sorted[0],
    p50_ms: percentile(0.50),
    p95_ms: percentile(0.95),
    p99_ms: percentile(0.99),
    max_ms: sorted.at(-1),
    sample_ms: samples,
  };
}

async function chooseSeat(client, seat) {
  const buttonId = seat === 'rider' ? 'choose-rider' : 'choose-gunner';
  await client.cdp.command('Runtime.evaluate', {
    expression: `document.getElementById('${buttonId}')?.click(); document.querySelector('canvas')?.focus()`,
  });
  await waitFor(() => client.seat === seat, `${seat} seat selection`);
}

async function updateMouseTrace(client) {
  const result = await client.cdp.command('Runtime.evaluate', {
    expression: 'window.__p3MouseMoves || []',
    returnByValue: true,
  });
  const events = result.result?.value || [];
  for (const event of events.slice(client.mouseEventCount)) {
    client.mouseDeltaX += event.movementX;
    client.mouseDeltaY += event.movementY;
  }
  client.mouseEventCount = events.length;
  return events;
}

async function moveMouse(client, dx, dy) {
  await client.cdp.command('Page.bringToFront');
  client.cursor.x += dx;
  client.cursor.y += dy;
  await client.cdp.command('Input.dispatchMouseEvent', {
    type: 'mouseMoved',
    x: client.cursor.x,
    y: client.cursor.y,
    button: client.rightDown ? 'right' : 'none',
  });
  await sleep(35);
  await updateMouseTrace(client);
}

async function mouseDown(client, button) {
  await client.cdp.command('Page.bringToFront');
  await client.cdp.command('Runtime.evaluate', { expression: 'document.querySelector("canvas")?.focus()' });
  await client.cdp.command('Input.dispatchMouseEvent', {
    type: 'mousePressed', x: client.cursor.x, y: client.cursor.y, button, clickCount: 1,
  });
  if (button === 'right') client.rightDown = true;
  await sleep(80);
}

async function mouseUp(client, button) {
  await client.cdp.command('Input.dispatchMouseEvent', {
    type: 'mouseReleased', x: client.cursor.x, y: client.cursor.y, button, clickCount: 1,
  });
  if (button === 'right') client.rightDown = false;
  await sleep(60);
}

async function fireClick(client) {
  await client.cdp.command('Input.dispatchMouseEvent', {
    type: 'mousePressed', x: client.cursor.x, y: client.cursor.y, button: 'left', clickCount: 1,
  });
  await sleep(25);
  await client.cdp.command('Input.dispatchMouseEvent', {
    type: 'mouseReleased', x: client.cursor.x, y: client.cursor.y, button: 'left', clickCount: 1,
  });
}

async function pageDebug(client) {
  const result = await client.cdp.command('Runtime.evaluate', {
    expression: `(() => {
      const canvas = document.querySelector('canvas');
      const rect = canvas?.getBoundingClientRect();
      const gl = canvas?.getContext('webgl2');
      return {
        href: location.href,
        focused: document.hasFocus(),
        pointer_locked: document.pointerLockElement !== null,
        canvas: canvas && { width: canvas.width, height: canvas.height,
          rect: rect && { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
          webgl2: !!gl, version: gl?.getParameter(gl.VERSION),
          renderer: (() => {
            const ext = gl?.getExtension('WEBGL_debug_renderer_info');
            return ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : null;
          })() },
      };
    })()`,
    returnByValue: true,
  });
  return result.result?.value;
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
  try { client.cdp.close(); } catch { /* Browser may already have disconnected. */ }
  client.browser.kill('SIGTERM');
}

function wrapAngle(angle) {
  return ((angle + Math.PI) % (Math.PI * 2) + Math.PI * 2) % (Math.PI * 2) - Math.PI;
}

function desiredGunnerAngles(snapshot) {
  const bike = snapshot.bike;
  const helicopter = snapshot.helicopter;
  const origin = {
    x: bike.x - Math.sin(bike.yaw) * 1.03,
    y: 1.84,
    z: bike.z + Math.cos(bike.yaw) * 1.03,
  };
  const dx = helicopter.position.x - origin.x;
  const dy = helicopter.position.y - origin.y;
  const dz = helicopter.position.z - origin.z;
  return {
    yaw: wrapAngle(Math.atan2(dx, -dz) - bike.yaw - Math.PI),
    pitch: Math.atan2(dy, Math.hypot(dx, dz)),
  };
}

async function aimAtHelicopter(client) {
  const snapshot = client.latest;
  if (!snapshot?.helicopter) return null;
  await updateMouseTrace(client);
  const currentYaw = wrapAngle(client.mouseDeltaX * 0.004);
  const currentPitch = Math.max(-Math.PI / 4, Math.min(75 * Math.PI / 180, -client.mouseDeltaY * 0.004));
  const desired = desiredGunnerAngles(snapshot);
  const yawError = wrapAngle(desired.yaw - currentYaw);
  const pitchError = desired.pitch - currentPitch;
  const deltaX = Math.abs(yawError) > 0.0015 ? Math.round(yawError / 0.004) : 0;
  const deltaY = Math.abs(pitchError) > 0.0015 ? Math.round(-pitchError / 0.004) : 0;
  if (deltaX || deltaY) await moveMouse(client, deltaX, deltaY);
  return {
    tick: snapshot.tick,
    desired,
    current: { yaw: currentYaw, pitch: currentPitch },
    error: { yaw: yawError, pitch: pitchError },
    mouse_delta: { x: deltaX, y: deltaY },
    lock_progress_ticks: client.latest.gunner_lock_progress_ticks,
  };
}

async function ensureLocked(client, report, timeoutMs = 14000) {
  const deadline = Date.now() + timeoutMs;
  const adjustments = [];
  while (Date.now() < deadline) {
    if (client.latest?.phase !== 'running') {
      await sleep(100);
      continue;
    }
    if (client.latest.gunner_locked && client.latest.fire_cooldown_ticks_remaining === 0) {
      report.lock_attempts.push({
        adjustment_count: adjustments.length,
        final_adjustments: adjustments.slice(-8),
        locked_tick: client.latest.tick,
      });
      return client.latest;
    }
    adjustments.push(await aimAtHelicopter(client));
    await sleep(120);
  }
  report.lock_attempts.push({
    adjustment_count: adjustments.length,
    final_adjustments: adjustments.slice(-8),
    timed_out: true,
  });
  throw new Error(`Failed to get server lock; latest snapshot ${JSON.stringify({
    tick: client.latest?.tick,
    heli: client.latest?.helicopter,
    lock: client.latest?.gunner_lock_progress_ticks,
    locked: client.latest?.gunner_locked,
    cooldown: client.latest?.fire_cooldown_ticks_remaining,
    adjustments: adjustments.slice(-10),
    mouse: { x: client.mouseDeltaX, y: client.mouseDeltaY },
  })}`);
}

function matchingEvent(rider, gunner, type, predicate = () => true) {
  for (const tick of rider.byTick.keys()) {
    const left = rider.byTick.get(tick);
    const right = gunner.byTick.get(tick);
    if (!right) continue;
    const leftEvents = left.combat_events?.filter((event) => event.kind?.type === type && predicate(event)) || [];
    for (const event of leftEvents) {
      const other = right.combat_events?.find((candidate) => candidate.event_id === event.event_id);
      if (other && other.kind?.type === type) return { tick, rider: left, gunner: right, event, other };
    }
  }
  return false;
}

(async () => {
  const root = path.resolve(__dirname, '..');
  const artifactDir = process.env.P3_ARTIFACT_DIR || path.join(root, 'docs/verification/p3');
  const reportPath = process.env.P3_REPORT || path.join(artifactDir, 'smoke-report.json');
  const rttMs = Number(process.env.P3_RTT_MS || 100);
  const jitterMs = Number(process.env.P3_JITTER_MS || 0);
  const seed = Number(process.env.P3_SEED || 1);
  const proxyPort = Number(process.env.P3_PROXY_PORT || 3102);
  const siteUrl = process.env.P3_SITE_URL || 'http://127.0.0.1:8080/';
  fs.mkdirSync(artifactDir, { recursive: true });
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'riverside-p3-'));
  const proxy = spawn(process.execPath, [path.join(root, 'tools/ws_latency_proxy.js'),
    '--rtt-ms', String(rttMs), '--jitter-ms', String(jitterMs), '--seed', String(seed),
    '--listen-port', String(proxyPort)], { stdio: 'ignore' });
  let rider, gunner, solo, dualRider, dualGunner;
  const report = {
    fixed_rtt_ms: rttMs,
    base_one_way_delay_ms: rttMs / 2,
    per_direction_jitter_ms: jitterMs,
    jitter_seed: seed,
    lock_attempts: [],
    shots: [],
  };
  try {
    await sleep(500);
    report.measured_proxy_rtt = await measureProxyRtt(
      `ws://127.0.0.1:${proxyPort}/ws?solo=1`,
    );
    const serverUrl = `server=${encodeURIComponent(`ws://127.0.0.1:${proxyPort}/ws`)}`;
    const createUrl = `${siteUrl}?${serverUrl}&create=1`;
    const firstPlayer = await launchBrowser({ port: 9232, url: createUrl, profile: path.join(tempRoot, 'first-player') });
    const displayedInviteUrl = await waitFor(async () => firstPlayer.cdp.command('Runtime.evaluate', {
      expression: 'document.getElementById("invite-url")?.value',
      returnByValue: true,
    }).then((result) => result.result?.value), 'copyable room invitation');
    if (!displayedInviteUrl?.includes(`room=${firstPlayer.welcome.room_code}`)) {
      throw new Error(`Invite URL did not include the room code: ${displayedInviteUrl}`);
    }
    const joinUrl = `${siteUrl}?${serverUrl}&room=${firstPlayer.welcome.room_code}`;
    const secondPlayer = await launchBrowser({ port: 9233, url: joinUrl, profile: path.join(tempRoot, 'second-player') });
    rider = firstPlayer;
    gunner = secondPlayer;
    await chooseSeat(firstPlayer, 'rider');
    await chooseSeat(secondPlayer, 'gunner');
    const firstRoleLabel = await waitFor(async () => firstPlayer.cdp.command('Runtime.evaluate', {
      expression: 'document.getElementById("rider-status")?.textContent',
      returnByValue: true,
    }).then((result) => result.result?.value).then((text) => text?.includes('你已選擇') ? text : false),
    'first member role card update');
    if (!firstRoleLabel?.includes('你已選擇')) {
      throw new Error(`The first member did not see their selected role: ${firstRoleLabel}`);
    }
    await firstPlayer.cdp.command('Runtime.evaluate', {
      expression: 'document.getElementById("request-seat-swap")?.click()',
    });
    const seatSwap = await waitFor(
      () => firstPlayer.seatSwapProposals.find((proposal) => proposal.requester_id === firstPlayer.welcome.player_id),
      'visible role swap proposal',
    );
    await waitFor(async () => secondPlayer.cdp.command('Runtime.evaluate', {
      expression: 'document.getElementById("seat-swap-response")?.hidden === false',
      returnByValue: true,
    }).then((result) => result.result?.value), 'recipient accept control');
    await secondPlayer.cdp.command('Runtime.evaluate', {
      expression: 'document.getElementById("accept-seat-swap")?.click()',
    });
    await waitFor(
      () => firstPlayer.seat === 'gunner' && secondPlayer.seat === 'rider',
      'browser-confirmed mutual seat swap',
    );
    report.seat_swap = {
      request_id: seatSwap.request_id,
      requester_new_seat: firstPlayer.seat,
      target_new_seat: secondPlayer.seat,
    };
    rider = secondPlayer;
    gunner = firstPlayer;
    dualRider = rider;
    dualGunner = gunner;
    report.dual_welcome = { rider: rider.welcome, gunner: gunner.welcome };
    report.join_order = ['Rider', 'Gunner'];
    report.displayed_invite_url = displayedInviteUrl;
    if (rider.seat !== 'rider' || gunner.seat !== 'gunner') throw new Error('Rider/Gunner seats were not assigned');
    await waitFor(() => rider.latest?.phase === 'ready_check' && gunner.latest?.phase === 'ready_check', 'dual Ready Check');

    await moveMouse(gunner, 640, 328);
    await sendKey(rider, 'r');
    await sendKey(gunner, 'r');
    await waitFor(() => rider.latest?.seats?.rider?.ready && rider.latest?.seats?.gunner?.ready
      && gunner.latest?.seats?.rider?.ready && gunner.latest?.seats?.gunner?.ready, 'both seats ready');
    await waitFor(() => rider.latest?.phase === 'running' && gunner.latest?.phase === 'running', 'five-second countdown', 10000);
    report.dual_running_tick = { rider: rider.latest.tick, gunner: gunner.latest.tick };
    await Promise.all([capture(rider, path.join(artifactDir, 'rider-running.png')),
      capture(gunner, path.join(artifactDir, 'gunner-helicopter-visible.png'))]);

    await mouseDown(rider, 'left');
    await mouseDown(gunner, 'right');
    report.pointer_lock = await waitFor(async () => {
      const state = await pageDebug(gunner);
      return state?.pointer_locked ? state : false;
    }, 'Gunner pointer lock', 5000);

    let nextRequestId = 1;
    report.rejected_attempts = [];
    for (let index = 0; index < 3; index += 1) {
      const previousHealth = gunner.latest.helicopter_health;
      let accepted = false;
      for (let attempt = 0; attempt < 5 && !accepted; attempt += 1) {
        const locked = await ensureLocked(gunner, report);
        if (index === 0 && attempt === 0) {
          report.first_server_lock = {
            tick: locked.tick,
            progress_ticks: locked.gunner_lock_progress_ticks,
            locked: locked.gunner_locked,
            can_fire: locked.gunner_can_fire,
            target: locked.helicopter.position,
          };
        }
        const requestId = nextRequestId;
        await fireClick(gunner);
        const outcome = await waitFor(() => matchingEvent(rider, gunner, 'gunner_fire_accepted',
          (event) => event.kind.request_id === requestId)
          || matchingEvent(rider, gunner, 'gunner_fire_rejected',
            (event) => event.kind.request_id === requestId),
        `shot ${index + 1} attempt ${attempt + 1} response`, 5000);
        nextRequestId += 1;
        if (outcome.event.kind.type === 'gunner_fire_accepted') {
          accepted = outcome;
        } else {
          report.rejected_attempts.push({
            shot: index + 1,
            request_id: requestId,
            reason: outcome.event.kind.reason,
            tick: outcome.event.tick,
          });
        }
      }
      if (!accepted) throw new Error(`Shot ${index + 1} was rejected five times`);
      const impact = await waitFor(() => matchingEvent(rider, gunner, 'player_missile_impacted',
        (event) => event.attack_id === accepted.event.attack_id && event.kind.hit), `shot ${index + 1} hit`, 5000);
      if (!impact.event.kind.hit || impact.event.kind.helicopter_health !== Math.max(0, previousHealth - 34)) {
        throw new Error(`Unexpected missile damage: ${JSON.stringify({ previousHealth, impact: impact.event })}`);
      }
      report.shots.push({
        index: index + 1,
        request_id: accepted.event.kind.request_id,
        attack_id: accepted.event.attack_id,
        fire_event_id: accepted.event.event_id,
        fire_tick: accepted.event.tick,
        impact_tick: impact.event.tick,
        impact_event_id: impact.event.event_id,
        helicopter_health: impact.event.kind.helicopter_health,
        ammo_remaining: accepted.event.kind.ammo_remaining,
        same_tick_event_on_both_clients: true,
      });
      if (index === 0) await capture(gunner, path.join(artifactDir, 'gunner-missile-hit.png'));
    }

    const victory = await waitFor(() => matchingEvent(rider, gunner, 'match_finished',
      (event) => event.kind.result === 'riders_victory'), 'matching dual-client victory', 5000);
    report.finished = {
      tick: victory.tick,
      rider_result: victory.rider.match_result,
      gunner_result: victory.gunner.match_result,
      rider_bike_health: victory.rider.bike_health,
      gunner_bike_health: victory.gunner.bike_health,
      rider_helicopter_health: victory.rider.helicopter_health,
      gunner_helicopter_health: victory.gunner.helicopter_health,
      event_id: victory.event.event_id,
      event_tick: victory.event.tick,
      both_ready_cleared: !victory.rider.seats.rider.ready && !victory.rider.seats.gunner.ready
        && !victory.gunner.seats.rider.ready && !victory.gunner.seats.gunner.ready,
    };
    if (report.finished.rider_result !== 'riders_victory'
      || report.finished.gunner_result !== 'riders_victory'
      || report.finished.rider_helicopter_health !== 0
      || report.finished.gunner_helicopter_health !== 0
      || !report.finished.both_ready_cleared) throw new Error(`Victory snapshots differ: ${JSON.stringify(report.finished)}`);
    await Promise.all([capture(rider, path.join(artifactDir, 'rider-victory.png')),
      capture(gunner, path.join(artifactDir, 'gunner-victory.png'))]);

    await sendKey(rider, 'r');
    await sendKey(gunner, 'r');
    await waitFor(() => rider.latest?.phase === 'countdown' && gunner.latest?.phase === 'countdown', 'both players ready for rematch', 5000);
    report.rematch_countdown = { rider: rider.latest.countdown_seconds, gunner: gunner.latest.countdown_seconds };
    if (rider.latest.bike_health !== 100 || gunner.latest.bike_health !== 100
      || rider.latest.helicopter_health !== 100 || gunner.latest.helicopter_health !== 100
      || rider.latest.gunner_ammo !== 6 || gunner.latest.gunner_ammo !== 6
      || rider.latest.player_missile || gunner.latest.player_missile
      || rider.latest.match_result || gunner.latest.match_result) throw new Error('Rematch retained prior combat state');
    await waitFor(() => rider.latest?.phase === 'running' && gunner.latest?.phase === 'running', 'rematch countdown completed', 10000);
    report.rematch_running = {
      rider_tick: rider.latest.tick,
      gunner_tick: gunner.latest.tick,
      bike_health: rider.latest.bike_health,
      helicopter_health: rider.latest.helicopter_health,
      ammo: rider.latest.gunner_ammo,
    };
    report.rider_controls = [];
    for (const key of ['a', 'd', ' ']) {
      const startedAt = performance.now();
      await holdKey(rider, key, 650);
      report.rider_controls.push({ key: key === ' ' ? 'space_brake' : key, held_ms: 650,
        resulting_tick: rider.latest?.tick, elapsed_ms: performance.now() - startedAt });
    }
    await mouseUp(rider, 'left');
    await mouseUp(gunner, 'right');
    await sendFunctionKey(rider, 'F3', 114);
    await sendFunctionKey(gunner, 'F3', 114);
    await rider.cdp.command('Page.bringToFront');
    report.network_diagnostics = await waitFor(async () => {
      const riderMetrics = await browserDiagnostics(rider);
      const gunnerMetrics = await browserDiagnostics(gunner);
      if (riderMetrics && gunnerMetrics) return { rider: riderMetrics, gunner: gunnerMetrics };
      return false;
    }, 'diagnostic panel on both browsers');
    report.ack_backlog_samples = [];
    let consecutiveEmptyAckSamples = 0;
    const ackDeadline = Date.now() + 10000;
    while (Date.now() < ackDeadline && consecutiveEmptyAckSamples < 3) {
      const metrics = await browserDiagnostics(rider);
      if (metrics) {
        report.network_diagnostics.rider = metrics;
        report.ack_backlog_samples.push({
          elapsed_ms: 10000 - (ackDeadline - Date.now()),
          pending_inputs: metrics.pending_rider_inputs,
          last_applied_sequence: metrics.last_applied_rider_sequence,
          server_tick: metrics.server_tick,
          client_tick: metrics.client_tick,
        });
        consecutiveEmptyAckSamples = metrics.pending_rider_inputs === 0
          ? consecutiveEmptyAckSamples + 1 : 0;
      }
      if (consecutiveEmptyAckSamples < 3) await sleep(500);
    }
    report.ack_backlog = {
      drained: consecutiveEmptyAckSamples >= 3,
      min_pending_inputs: Math.min(...report.ack_backlog_samples.map((item) => item.pending_inputs)),
      max_pending_inputs: Math.max(...report.ack_backlog_samples.map((item) => item.pending_inputs)),
      consecutive_empty_samples: consecutiveEmptyAckSamples,
    };
    const wrapAngle = (angle) => ((angle + Math.PI) % (2 * Math.PI) + 2 * Math.PI)
      % (2 * Math.PI) - Math.PI;
    report.shared_tick_consistency = (() => {
      const errors = [];
      for (const [tick, riderSnapshot] of rider.byTick) {
        const gunnerSnapshot = gunner.byTick.get(tick);
        if (!gunnerSnapshot) continue;
        errors.push({
          tick,
          position_error_m: Math.hypot(riderSnapshot.bike.x - gunnerSnapshot.bike.x,
            riderSnapshot.bike.z - gunnerSnapshot.bike.z),
          yaw_error_deg: Math.abs(wrapAngle(riderSnapshot.bike.yaw - gunnerSnapshot.bike.yaw))
            * 180 / Math.PI,
        });
      }
      return {
        compared_ticks: errors.length,
        max_position_error_m: Math.max(0, ...errors.map((item) => item.position_error_m)),
        max_yaw_error_deg: Math.max(0, ...errors.map((item) => item.yaw_error_deg)),
        samples: errors.slice(-12),
      };
    })();
    await dispose(rider); rider = null;
    await dispose(gunner); gunner = null;
    await sleep(400);

    solo = await launchBrowser({
      port: 9234,
      url: `${siteUrl}?solo=1&${serverUrl}`,
      profile: path.join(tempRoot, 'solo'),
    });
    if (!solo.welcome.solo_mode || solo.seat !== 'rider') throw new Error('Solo mode did not claim both seats');
    await waitFor(() => solo.latest?.phase === 'ready_check', 'solo Ready Check');
    await sendKey(solo, 'r');
    await waitFor(() => solo.latest?.phase === 'running', 'solo force-ready countdown', 10000);
    await sendKey(solo, 'v');
    await waitFor(() => solo.seat === 'gunner', 'server confirmed solo role switch');
    report.solo = {
      welcome: solo.welcome,
      seat_after_switch: solo.seat,
      phase: solo.latest.phase,
      bike_health: solo.latest.bike_health,
      helicopter_health: solo.latest.helicopter_health,
      ammo: solo.latest.gunner_ammo,
      runtime_errors: solo.exceptions,
    };
    if (solo.exceptions.length || solo.latest.bike_health !== 100 || solo.latest.gunner_ammo !== 6) {
      throw new Error('Solo role switch failed or refilled the match incorrectly');
    }
    await capture(solo, path.join(artifactDir, 'solo-gunner-after-switch.png'));
    for (const [name, client] of [['rider', dualRider], ['gunner', dualGunner], ['solo', solo]]) {
      if (client.exceptions.length || client.consoleErrors.length) {
        throw new Error(`${name} browser reported runtime errors: ${JSON.stringify({
          exceptions: client.exceptions,
          console: client.consoleErrors,
        })}`);
      }
    }
    report.client_errors = {
      rider: dualRider.exceptions, gunner: dualGunner.exceptions, solo: solo.exceptions,
      rider_console_errors: dualRider.consoleErrors, gunner_console_errors: dualGunner.consoleErrors,
      solo_console_errors: solo.consoleErrors,
    };
    report.canvas_debug = {
      solo: await pageDebug(solo),
    };
    report.environment = {
      browser: dualRider.browserVersion.Browser,
      user_agent: dualRider.browserVersion['User-Agent'],
      platform: process.platform,
      injected_rtt_ms: rttMs,
      injected_jitter_per_direction_ms: jitterMs,
      jitter_seed: seed,
      packet_loss: 'not injected; TCP stream proxy preserves byte delivery',
      webgl_renderer: report.canvas_debug.solo?.canvas?.renderer,
    };
    report.checks = {
      rider_ack_backlog_drained: report.ack_backlog.drained,
      no_history_misses: report.network_diagnostics.rider.history_miss_count === 0,
      same_tick_position_below_2cm: report.shared_tick_consistency.max_position_error_m < 0.02,
      same_tick_yaw_consistent: report.shared_tick_consistency.max_yaw_error_deg < 1,
      solo_role_switch_kept_match_running: report.solo.phase === 'running'
        && report.solo.seat_after_switch === 'gunner',
      no_browser_errors: Object.values(report.client_errors).flat().length === 0,
    };
    fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
    console.log(JSON.stringify(report, null, 2));
    if (Object.values(report.checks).some((passed) => !passed)) process.exitCode = 1;
  } catch (error) {
    report.runtime_errors = {
      rider: rider?.exceptions,
      gunner: gunner?.exceptions,
      solo: solo?.exceptions,
      rider_console: rider?.consoleErrors,
      gunner_console: gunner?.consoleErrors,
      solo_console: solo?.consoleErrors,
    };
    report.debug = {
      rider: rider && { latest: rider.latest, canvas: await pageDebug(rider).catch(String) },
      gunner: gunner && { latest: gunner.latest, canvas: await pageDebug(gunner).catch(String),
        mouse: { x: gunner.mouseDeltaX, y: gunner.mouseDeltaY } },
      solo: solo && { latest: solo.latest, canvas: await pageDebug(solo).catch(String) },
    };
    fs.writeFileSync('/tmp/riverside_p3_failure.json', `${JSON.stringify(report, null, 2)}\n`);
    console.error(error.stack || String(error));
    console.error(JSON.stringify(report, null, 2));
    process.exitCode = 1;
  } finally {
    await mouseUp(rider, 'left').catch(() => {});
    await mouseUp(gunner, 'right').catch(() => {});
    await dispose(rider);
    await dispose(gunner);
    await dispose(solo);
    proxy.kill('SIGTERM');
  }
})();

const { spawn } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { reportJson } = require('./report_json.cjs');
const WebSocket = require('ws');

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor(check, label, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const result = await check();
    if (result) return result;
    await sleep(100);
  }
  throw new Error(`Timed out waiting for ${label}`);
}

async function main() {
  const root = path.resolve(__dirname, '..');
  const reportPath = process.env.P6_FIREFOX_REPORT
    || path.join(root, 'docs/verification/p6/firefox-smoke.json');
  const siteUrl = process.env.P3_SITE_URL || 'http://127.0.0.1:8081/';
  const serverAddress = process.env.P6_FIREFOX_SERVER || 'ws://127.0.0.1:3001/ws';
  const driverPort = Number(process.env.P6_GECKODRIVER_PORT || 4445);
  const driverBinary = process.env.GECKODRIVER_BIN || '/snap/bin/geckodriver';
  const driverProcess = spawn(driverBinary, [
    '--host', '127.0.0.1', '--port', String(driverPort), '--log', 'fatal',
  ], { stdio: 'ignore', env: { ...process.env, MOZ_WEBRENDER: '1', MOZ_HEADLESS: '1' } });
  let sessionId;
  let bidi;
  let bidiId = 0;
  const bidiPending = new Map();
  const consoleErrors = [];
  const protocolErrors = [];
  const report = {
    generated_at: new Date().toISOString(),
    browser: 'Firefox WebDriver BiDi',
    checks: {},
  };

  async function driver(method, endpoint, body) {
    const response = await fetch(`http://127.0.0.1:${driverPort}${endpoint}`, {
      method,
      headers: body === undefined ? undefined : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await response.text();
    let result;
    try { result = text ? JSON.parse(text) : {}; } catch { result = { raw: text }; }
    if (!response.ok || result.value?.error) {
      throw new Error(`${method} ${endpoint}: ${response.status} ${JSON.stringify(result)}`);
    }
    return result.value;
  }

  function bidiCommand(method, params) {
    const id = ++bidiId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        bidiPending.delete(id);
        reject(new Error(`BiDi timeout: ${method}`));
      }, 8000);
      bidiPending.set(id, { resolve, reject, timer });
      bidi.send(JSON.stringify({ id, method, params }));
    });
  }

  async function execute(script, args = []) {
    return driver('POST', `/session/${sessionId}/execute/sync`, { script, args });
  }

  async function key(key) {
    await driver('POST', `/session/${sessionId}/actions`, {
      actions: [{
        type: 'key',
        id: 'p6-keyboard',
        actions: [{ type: 'keyDown', value: key }, { type: 'keyUp', value: key }],
      }],
    });
  }

  async function holdKey(key, durationMs) {
    await driver('POST', `/session/${sessionId}/actions`, {
      actions: [{ type: 'key', id: 'p6-hold-key', actions: [{ type: 'keyDown', value: key }] }],
    });
    await sleep(durationMs);
    await driver('POST', `/session/${sessionId}/actions`, {
      actions: [{ type: 'key', id: 'p6-hold-key', actions: [{ type: 'keyUp', value: key }] }],
    });
  }

  async function click(button) {
    await driver('POST', `/session/${sessionId}/actions`, {
      actions: [{
        type: 'pointer',
        id: 'p6-pointer',
        parameters: { pointerType: 'mouse' },
        actions: [
          { type: 'pointerMove', duration: 0, x: 640, y: 320 },
          { type: 'pointerDown', button },
          { type: 'pointerUp', button },
        ],
      }],
    });
  }

  async function holdPointer(button, durationMs, release = true) {
    await driver('POST', `/session/${sessionId}/actions`, {
      actions: [{
        type: 'pointer', id: 'p6-hold-pointer', parameters: { pointerType: 'mouse' },
        actions: [
          { type: 'pointerMove', duration: 0, x: 640, y: 320 },
          { type: 'pointerDown', button },
        ],
      }],
    });
    await sleep(durationMs);
    const state = await execute(`return ({locked: document.pointerLockElement === document.getElementById('game-canvas')})`);
    if (release) {
      await driver('POST', `/session/${sessionId}/actions`, {
        actions: [{ type: 'pointer', id: 'p6-hold-pointer', parameters: { pointerType: 'mouse' },
          actions: [{ type: 'pointerUp', button }] }],
      });
    }
    return state;
  }

  try {
    await waitFor(async () => {
      try { return (await fetch(`http://127.0.0.1:${driverPort}/status`)).ok; } catch { return false; }
    }, 'geckodriver ready');
    const created = await driver('POST', '/session', {
      capabilities: {
        alwaysMatch: {
          browserName: 'firefox',
          webSocketUrl: true,
          'moz:firefoxOptions': {
            args: ['-headless', '--width=1280', '--height=800'],
            prefs: {
              'webgl.force-enabled': true,
              'webgl.disabled': false,
              'webgl.disable-fail-if-major-performance-caveat': true,
              'gfx.webrender.all': true,
              'gfx.webrender.software': true,
              'layers.acceleration.force-enabled': true,
            },
          },
        },
      },
    });
    sessionId = created.sessionId;
    const capabilities = created.capabilities;
    report.environment = {
      browser_version: capabilities.browserVersion,
      platform: capabilities.platformName,
      user_agent: null,
      webgl_renderer: null,
      site_url: siteUrl,
      injected_rtt_ms: 0,
      injected_jitter_per_direction_ms: 0,
      packet_loss: 'not injected',
    };
    if (!capabilities.webSocketUrl) throw new Error('Firefox did not expose a WebDriver BiDi URL');

    bidi = new WebSocket(capabilities.webSocketUrl);
    bidi.on('message', (data) => {
      const message = JSON.parse(data.toString());
      if (message.id) {
        const pending = bidiPending.get(message.id);
        if (!pending) return;
        bidiPending.delete(message.id);
        clearTimeout(pending.timer);
        if (message.type === 'error') pending.reject(new Error(JSON.stringify(message))); else pending.resolve(message.result);
      } else if (message.method === 'log.entryAdded' && message.params?.level === 'error') {
        consoleErrors.push(message.params.text || message.params.message || 'Firefox console error');
      }
    });
    await new Promise((resolve, reject) => {
      bidi.once('open', resolve);
      bidi.once('error', reject);
    });
    await bidiCommand('session.subscribe', { events: ['log.entryAdded'] });
    await bidiCommand('script.addPreloadScript', {
      functionDeclaration: `() => {
        const nativeWebSocket = window.WebSocket;
        const state = { incoming: [], outgoing: [], socket: null };
        Object.defineProperty(window, '__p6Protocol', { value: state, configurable: false });
        window.WebSocket = new Proxy(nativeWebSocket, {
          construct(target, args) {
            const socket = Reflect.construct(target, args, target);
            if (!String(args[0]).includes('/ws')) return socket;
            state.socket = socket;
            socket.addEventListener('message', event => {
              if (typeof event.data === 'string') state.incoming.push(event.data);
            });
            const originalSend = socket.send.bind(socket);
            socket.send = data => {
              if (typeof data === 'string') state.outgoing.push(data);
              return originalSend(data);
            };
            return socket;
          }
        });
      }`,
    });

    const standaloneWebGl = await execute(`return (() => {
      const canvas = document.createElement('canvas');
      const gl = canvas.getContext('webgl2');
      const supported = !!gl;
      const renderer = gl && (() => { const ext=gl.getExtension('WEBGL_debug_renderer_info');
        return ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : null; })();
      gl?.getExtension('WEBGL_lose_context')?.loseContext();
      return { supported, renderer };
    })()`);
    report.environment.webgl_renderer = standaloneWebGl.renderer;
    report.checks.webgl2_available = standaloneWebGl.supported;
    if (!standaloneWebGl.supported) {
      report.unverified_reason = 'Firefox headless returned null from a standalone WebGL2 canvas; game interaction cannot run in this environment.';
      fs.mkdirSync(path.dirname(reportPath), { recursive: true });
      fs.writeFileSync(reportPath, `${reportJson(report)}\n`);
      console.log(reportJson(report));
      return;
    }

    const pageUrl = `${siteUrl}?solo=1&server=${encodeURIComponent(serverAddress)}`;
    await driver('POST', `/session/${sessionId}/url`, { url: pageUrl });
    const initialState = await waitFor(async () => {
      const state = await execute(`return (() => {
        const solo = document.getElementById('solo-mode');
        return { loaded: document.readyState === 'complete',
          solo_visible: !!solo && !solo.hidden,
          incoming: window.__p6Protocol?.incoming?.length || 0,
          user_agent: navigator.userAgent };
      })()`);
      return state?.solo_visible && state.incoming > 0 ? state : false;
    }, 'Firefox Solo page and WebSocket welcome');
    report.environment.user_agent = initialState.user_agent;
    report.checks.solo_page_loaded = initialState.loaded;
    report.checks.solo_websocket_connected = initialState.incoming > 0;

    await key('r');
    const running = await waitFor(async () => {
      const state = await execute(`return (() => {
        const messages = window.__p6Protocol?.incoming || [];
        const snapshots = messages.flatMap(raw => { try { const message=JSON.parse(raw);
          return message.type === 'snapshot' ? [message.payload] : []; } catch { return []; } });
        return snapshots.filter(snapshot => snapshot.phase === 'running').length;
      })()`);
      return state > 0 ? state : false;
    }, 'R forced both solo seats Ready and started the match');
    report.checks.force_ready_running = running > 0;

    await key('v');
    const gunnerSwitch = await waitFor(async () => execute(`return (() => {
      const messages = window.__p6Protocol?.incoming || [];
      return messages.some(raw => { try { const m=JSON.parse(raw);
        return m.type === 'seat_changed' && m.payload.seat === 'gunner'; } catch { return false; } });
    })()`), 'V server acknowledgement to Gunner');
    report.checks.v_switch_to_gunner_acknowledged = !!gunnerSwitch;
    await key('v');
    const riderSwitch = await waitFor(async () => execute(`return (() => {
      const messages = window.__p6Protocol?.incoming || [];
      return messages.filter(raw => { try { const m=JSON.parse(raw);
        return m.type === 'seat_changed' && m.payload.seat === 'rider'; } catch { return false; } }).length > 0;
    })()`), 'V server acknowledgement back to Rider');
    report.checks.v_switch_to_rider_acknowledged = !!riderSwitch;

    const mouseDown = await driver('POST', `/session/${sessionId}/actions`, {
      actions: [{
        type: 'pointer', id: 'p6-hold', parameters: { pointerType: 'mouse' },
        actions: [
          { type: 'pointerMove', duration: 0, x: 640, y: 320 },
          { type: 'pointerDown', button: 0 },
        ],
      }],
    });
    for (const keyValue of ['a', 'd', ' ']) {
      await holdKey(keyValue, 300);
      await sleep(120);
    }
    await driver('DELETE', `/session/${sessionId}/actions`);
    await sleep(500);
    const inputState = await execute(`return (() => {
      const messages = (window.__p6Protocol?.outgoing || []).flatMap(raw => {
        try { return [JSON.parse(raw)]; } catch { return []; }
      });
      const rider = messages.filter(m => m.type === 'rider_input').map(m => m.payload?.input);
      return { count: rider.length,
        steer_left: rider.some(input => input?.steer < -0.5),
        steer_right: rider.some(input => input?.steer > 0.5),
        brake: rider.some(input => input?.brake_held),
        pedal: rider.some(input => input?.pedal_held) };
    })()`);
    report.rider_inputs = inputState;
    report.checks.rider_steer_and_brake_inputs_sent = inputState.count > 0
      && inputState.steer_left && inputState.steer_right && inputState.brake;
    if (!mouseDown) throw new Error('Firefox pointer action was not acknowledged');

    await key('v');
    const gunner = await waitFor(async () => execute(`return (() => {
      const messages = (window.__p6Protocol?.incoming || []).flatMap(raw => {
        try { return [JSON.parse(raw)]; } catch { return []; }
      });
      return messages.some(m => m.type === 'seat_changed' && m.payload.seat === 'gunner');
    })()`), 'V switch to Gunner after Rider input test');
    report.checks.switched_back_to_gunner = !!gunner;
    const pointerState = await holdPointer(2, 450, false);
    report.pointer_lock = pointerState;
    report.checks.pointer_lock = pointerState?.locked === true;
    await click(0);
    await sleep(300);
    await driver('POST', `/session/${sessionId}/actions`, {
      actions: [{ type: 'pointer', id: 'p6-hold-pointer', parameters: { pointerType: 'mouse' },
        actions: [{ type: 'pointerUp', button: 2 }] }],
    });
    const outTypes = await execute(`return (window.__p6Protocol?.outgoing || []).flatMap(raw => {
      try { return [JSON.parse(raw).type]; } catch { return []; }
    })`);
    report.checks.gunner_input_sent = outTypes.includes('gunner_input');
    report.checks.fire_request_sent = outTypes.includes('fire_request');

    const metrics = await execute(`return (() => {
      const messages = (window.__p6Protocol?.incoming || []).flatMap(raw => {
        try { return [JSON.parse(raw)]; } catch { return []; }
      });
      return { snapshots: messages.filter(m => m.type === 'snapshot').length,
        seat_changes: messages.filter(m => m.type === 'seat_changed').map(m => m.payload.seat),
        fire_events: messages.filter(m => m.type === 'snapshot').flatMap(m => m.payload.combat_events || [])
          .filter(event => String(event.kind?.type || '').startsWith('gunner_fire_'))
          .map(event => ({type:event.kind.type,tick:event.tick,event_id:event.event_id})) };
    })()`);
    report.protocol_observations = metrics;
    report.browser_console_errors = consoleErrors;
    report.protocol_errors = protocolErrors;
    report.checks.no_browser_console_errors = consoleErrors.length === 0;
    const screenshot = await driver('GET', `/session/${sessionId}/screenshot`);
    report.screenshot_sha256 = crypto.createHash('sha256')
      .update(Buffer.from(screenshot, 'base64')).digest('hex');
    fs.mkdirSync(path.dirname(reportPath), { recursive: true });
    fs.writeFileSync(reportPath, `${reportJson(report)}\n`);
    console.log(reportJson(report));
    if (Object.values(report.checks).some((passed) => !passed)) process.exitCode = 1;
  } catch (error) {
    report.error = error.stack || String(error);
    report.browser_console_errors = consoleErrors;
    fs.mkdirSync(path.dirname(reportPath), { recursive: true });
    fs.writeFileSync(reportPath, `${reportJson(report)}\n`);
    console.error(report.error);
    process.exitCode = 1;
  } finally {
    if (bidi?.readyState === WebSocket.OPEN) bidi.close();
    if (sessionId) {
      try { await driver('DELETE', `/session/${sessionId}`); } catch { /* best effort cleanup */ }
    }
    driverProcess.kill('SIGTERM');
  }
}

main();

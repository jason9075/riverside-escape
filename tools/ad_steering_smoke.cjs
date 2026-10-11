const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { reportJson } = require('./report_json.cjs');
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

async function launchBrowser() {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'riverside-ad-'));
  const port = 9252;
  const browser = spawn(process.env.CHROME_BIN || '/usr/bin/google-chrome', [
    '--headless=new', '--no-sandbox', '--disable-dev-shm-usage',
    '--enable-webgl', '--ignore-gpu-blocklist', '--use-gl=angle',
    '--use-angle=swiftshader', '--enable-unsafe-swiftshader',
    '--disable-frame-rate-limit', '--disable-gpu-vsync',
    '--hide-scrollbars', '--window-size=640,480', '--force-device-scale-factor=1',
    `--remote-debugging-port=${port}`, `--user-data-dir=${path.join(tempDir, 'profile')}`, 'about:blank',
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
  }, 'Chrome DevTools endpoint');
  const cdp = await DevTools.connect(target.webSocketDebuggerUrl);
  const client = {
    browser, cdp, tempDir,
    welcome: null, latest: null, snapshots: [], exceptions: [], consoleErrors: [],
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
      if (message.type === 'welcome') client.welcome = message.payload;
      if (message.type === 'snapshot') {
        client.latest = message.payload;
        client.snapshots.push(message.payload);
        if (client.snapshots.length > 2000) client.snapshots.shift();
      }
    } catch { /* Ignore non-protocol WebSocket frames. */ }
  };
  await cdp.command('Page.enable');
  await cdp.command('Runtime.enable');
  await cdp.command('Log.enable');
  await cdp.command('Network.enable');
  await cdp.command('Page.addScriptToEvaluateOnNewDocument', {
    source: `Object.defineProperty(Document.prototype, 'hasFocus', {
      configurable: true, value: () => true,
    });`,
  });
  await cdp.command('Page.navigate', {
    url: 'http://127.0.0.1:8080/?solo=1',
  });
  await waitFor(() => client.welcome && client.latest?.phase === 'ready_check', 'Solo Ready Check');
  await cdp.command('Page.bringToFront');
  client.mouseCenter = await cdp.command('Runtime.evaluate', {
    expression: `(() => {
      const canvas = document.querySelector('canvas');
      canvas?.focus();
      const rect = canvas?.getBoundingClientRect();
      return rect ? { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 } : null;
    })()`,
    returnByValue: true,
  }).then((result) => result.result?.value);
  if (!client.mouseCenter) throw new Error('WASM canvas was not found');
  return client;
}

async function sendKey(client, key, type) {
  const upper = key.toUpperCase();
  await client.cdp.command('Input.dispatchKeyEvent', {
    type,
    key,
    code: key === ' ' ? 'Space' : `Key${upper}`,
    windowsVirtualKeyCode: upper.charCodeAt(0),
    nativeVirtualKeyCode: upper.charCodeAt(0),
  });
}

async function mouseButton(client, type) {
  await client.cdp.command('Input.dispatchMouseEvent', {
    type,
    x: client.mouseCenter.x,
    y: client.mouseCenter.y,
    button: 'left',
    buttons: type === 'mousePressed' ? 1 : 0,
    clickCount: 1,
  });
}

async function capture(client, filename) {
  await sleep(100);
  const screenshot = await client.cdp.command('Page.captureScreenshot', {
    format: 'png', captureBeyondViewport: false,
  });
  fs.writeFileSync(filename, Buffer.from(screenshot.data, 'base64'));
}

function angleDelta(from, to) {
  return (to - from + Math.PI) % (Math.PI * 2) - Math.PI;
}

function lateralDelta(from, to) {
  const yaw = from.bike.yaw;
  const rightX = Math.cos(yaw);
  const rightZ = Math.sin(yaw);
  return (to.bike.x - from.bike.x) * rightX + (to.bike.z - from.bike.z) * rightZ;
}

async function dispose(client) {
  if (!client) return;
  try { client.cdp.close(); } catch { /* Browser may already have exited. */ }
  await new Promise((resolve) => {
    const timeout = setTimeout(resolve, 2000);
    client.browser.once('exit', () => { clearTimeout(timeout); resolve(); });
    client.browser.kill('SIGTERM');
  });
  try {
    fs.rmSync(client.tempDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  } catch { /* Chrome may leave a transient profile lock while the process exits. */ }
}

(async () => {
  const reportPath = path.resolve(__dirname, '../docs/verification/p4/ad-steering-wasm.json');
  const artifactDir = path.dirname(reportPath);
  fs.mkdirSync(artifactDir, { recursive: true });
  let client;
  const report = { browser: 'Chrome headless + SwiftShader / WebGL2', actions: [] };
  try {
    const runAction = async (key, expected, sign, screenshotName) => {
      client = await launchBrowser();
      if (!client.welcome?.solo_mode || client.welcome.seat !== 'rider') {
        throw new Error(`Unexpected Solo welcome: ${JSON.stringify(client.welcome)}`);
      }
      await sendKey(client, 'r', 'keyDown');
      await sendKey(client, 'r', 'keyUp');
      await waitFor(() => client.latest?.phase === 'running', 'Solo countdown');
      await waitFor(() => client.latest?.bike?.speed === 0, 'starting bike at rest');
      await mouseButton(client, 'mousePressed');
      const start = client.latest;
      await sendKey(client, key.toLowerCase(), 'keyDown');
      await waitFor(() => client.latest?.bike?.steering_angle * sign > 0.015,
        `${key} ${expected} front fork`, 7000);
      const atSteeringResponse = client.latest;
      await capture(client, path.join(artifactDir, screenshotName));
      await sleep(1800);
      const end = client.latest;
      await sendKey(client, key.toLowerCase(), 'keyUp');
      await mouseButton(client, 'mouseReleased');
      const samples = client.snapshots.filter((snapshot) => snapshot.tick >= start.tick && snapshot.tick <= end.tick);
      const action = {
        key, expected,
        start: { x: start.bike.x, z: start.bike.z, yaw: start.bike.yaw },
        end: { x: end.bike.x, z: end.bike.z, yaw: end.bike.yaw, steer: end.bike.steering_angle },
        ticks: { start: start.tick, end: end.tick },
        steering_response: {
          tick: atSteeringResponse.tick,
          angle: atSteeringResponse.bike.steering_angle,
          peak_angle: sign > 0
            ? Math.max(...samples.map((snapshot) => snapshot.bike.steering_angle))
            : Math.min(...samples.map((snapshot) => snapshot.bike.steering_angle)),
        },
        yaw_delta: angleDelta(start.bike.yaw, end.bike.yaw),
        lateral_delta_m: lateralDelta(start, end),
      };
      const errors = { exceptions: client.exceptions, console_errors: client.consoleErrors };
      if (errors.exceptions.length || errors.console_errors.length) {
        throw new Error(`${key} browser errors: ${JSON.stringify(errors)}`);
      }
      await dispose(client);
      client = null;
      await sleep(300);
      return { action, errors };
    };

    const webglProbeClient = await launchBrowser();
    report.webgl2 = await webglProbeClient.cdp.command('Runtime.evaluate', {
      expression: '(() => !!document.querySelector("canvas")?.getContext("webgl2"))()',
      returnByValue: true,
    }).then((result) => result.result?.value);
    await dispose(webglProbeClient);
    await sleep(300);
    const right = await runAction('D', 'right', 1, 'steering-d-right-wasm.png');
    report.actions.push(right.action);
    const left = await runAction('A', 'left', -1, 'steering-a-left-wasm.png');
    report.actions.push(left.action);
    report.client_errors = { right: right.errors, left: left.errors };

    if (report.actions[0].yaw_delta <= 0.001 || report.actions[0].lateral_delta_m <= 0.001) {
      throw new Error(`D did not turn right: ${JSON.stringify(report.actions[0])}`);
    }
    if (report.actions[1].yaw_delta >= -0.001 || report.actions[1].lateral_delta_m >= -0.001) {
      throw new Error(`A did not turn left: ${JSON.stringify(report.actions[1])}`);
    }
    if (report.actions[0].steering_response.peak_angle < 0.015
      || report.actions[1].steering_response.peak_angle > -0.015) {
      throw new Error('Front fork did not reach the matching steering direction');
    }
    if (!report.webgl2) throw new Error('The client did not initialize WebGL2');
    fs.writeFileSync(reportPath, `${reportJson(report)}\n`);
    console.log(reportJson(report));
  } catch (error) {
    report.error = error.stack || String(error);
    report.live_state = client?.latest;
    fs.writeFileSync(path.join(artifactDir, 'ad-steering-wasm-failure.json'), `${reportJson(report)}\n`);
    console.error(reportJson(report));
    process.exitCode = 1;
  } finally {
    await dispose(client);
  }
})();

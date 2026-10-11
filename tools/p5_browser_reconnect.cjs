const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const WebSocket = require('ws');

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor(check, label, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await sleep(40);
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
      }, 10000);
      this.pending.set(id, { resolve, reject, timer });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  close() {
    if (this.socket.readyState === WebSocket.OPEN) this.socket.close();
  }
}

async function launchBrowser(port, url, profile) {
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
      const response = await fetch(`${endpoint}/json/list`);
      target = (await response.json()).find((item) => item.type === 'page');
      return target;
    } catch {
      return false;
    }
  }, `Chrome DevTools ${port}`);
  const cdp = await DevTools.connect(target.webSocketDebuggerUrl);
  const client = {
    browser,
    cdp,
    welcomes: [],
    snapshots: [],
    handshakes: [],
    websocketCloses: 0,
    exceptions: [],
    consoleErrors: [],
  };
  cdp.onEvent = (method, params) => {
    if (method === 'Runtime.exceptionThrown') {
      client.exceptions.push(params.exceptionDetails?.text || 'runtime exception');
    }
    if (method === 'Log.entryAdded' && params.entry?.level === 'error') {
      client.consoleErrors.push(params.entry.text);
    }
    if (method === 'Network.webSocketCreated') {
      client.handshakes.push(params.url || '');
    }
    if (method === 'Network.webSocketClosed') client.websocketCloses += 1;
    if (method !== 'Network.webSocketFrameReceived') return;
    try {
      const message = JSON.parse(params.response.payloadData);
      if (message.type === 'welcome') client.welcomes.push(message.payload);
      if (message.type === 'snapshot') {
        client.latest = message.payload;
        client.snapshots.push(message.payload);
      }
    } catch { /* Ignore non-protocol frames. */ }
  };
  await cdp.command('Page.enable');
  await cdp.command('Runtime.enable');
  await cdp.command('Log.enable');
  await cdp.command('Network.enable');
  await cdp.command('Page.addScriptToEvaluateOnNewDocument', {
    source: `Object.defineProperty(Document.prototype, 'hasFocus', { configurable: true, value: () => true });`,
  });
  await cdp.command('Page.navigate', { url });
  await waitFor(() => client.welcomes.length > 0, 'first browser Welcome');
  return client;
}

async function click(client, id) {
  await client.cdp.command('Runtime.evaluate', {
    expression: `document.getElementById(${JSON.stringify(id)})?.click()`,
  });
}

function directClient(address) {
  const socket = new WebSocket(address);
  const client = { socket, messages: [], latest: null };
  socket.on('message', (data) => {
    const message = JSON.parse(data.toString());
    client.messages.push(message);
    if (message.type === 'snapshot') client.latest = message.payload;
  });
  return new Promise((resolve, reject) => {
    socket.once('open', () => resolve(client));
    socket.once('error', reject);
  });
}

function send(client, type, payload) {
  client.socket.send(JSON.stringify(payload === undefined ? { type } : { type, payload }));
}

async function waitMessage(client, type, predicate = () => true) {
  return waitFor(
    () => client.messages.find((message) => message.type === type && predicate(message.payload)),
    type,
  );
}

async function waitSnapshot(client, predicate, label, timeoutMs = 20000) {
  return waitFor(() => predicate(client.latest) ? client.latest : false, label, timeoutMs);
}

async function closeBrowser(client) {
  if (!client) return;
  try { client.cdp.close(); } catch { /* Browser may already have exited. */ }
  client.browser.kill('SIGTERM');
}

async function main() {
  const root = path.resolve(__dirname, '..');
  const serverBinary = process.env.RIVERSIDE_SERVER_BIN
    || path.join(root, 'target/debug/riverside-server');
  const reportPath = process.env.BROWSER_RECONNECT_REPORT
    || path.join(root, 'docs/verification/p5/browser-reconnect-report.json');
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'riverside-p5-reconnect-'));
  const serverPort = Number(process.env.BROWSER_RECONNECT_PORT || 3114);
  const proxyPort = Number(process.env.BROWSER_RECONNECT_PROXY_PORT || 3115);
  const controlPort = Number(process.env.BROWSER_RECONNECT_CONTROL_PORT || 3116);
  const debugPort = Number(process.env.BROWSER_DEBUG_PORT || 9234);
  const siteBase = process.env.BROWSER_SITE_URL || 'http://127.0.0.1:8080/';
  if (!fs.existsSync(serverBinary)) throw new Error(`Server binary not found: ${serverBinary}`);
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });
  const server = spawn(serverBinary, [], {
    env: { ...process.env, RIVERSIDE_BIND: `127.0.0.1:${serverPort}` },
    stdio: 'ignore',
  });
  const proxy = spawn(process.execPath, [
    path.join(root, 'tools/ws_disconnect_proxy.cjs'),
    '--listen-port', String(proxyPort),
    '--control-port', String(controlPort),
    '--target-port', String(serverPort),
  ], { stdio: 'ignore' });
  let browser;
  let gunner;
  const report = { generated_at: new Date().toISOString(), checks: {} };
  const wsAddress = `ws://127.0.0.1:${serverPort}/ws`;
  const browserWsAddress = `ws://127.0.0.1:${proxyPort}/ws`;
  try {
    await waitFor(async () => {
      try { return (await fetch(`http://127.0.0.1:${serverPort}/health`)).ok; } catch { return false; }
    }, 'temporary server health');
    await waitFor(async () => {
      try { return (await fetch(`http://127.0.0.1:${controlPort}/health`)).ok; } catch { return false; }
    }, 'isolated WebSocket proxy health');
    const serverParam = encodeURIComponent(browserWsAddress);
    browser = await launchBrowser(
      debugPort,
      `${siteBase}?server=${serverParam}&create=1`,
      path.join(tempRoot, 'browser-profile'),
    );
    const firstWelcome = browser.welcomes[0];
    const roomCode = firstWelcome.room_code;
    const firstToken = firstWelcome.reconnect_token;
    if (!firstToken || firstToken.length !== 64) throw new Error('Browser Welcome lacked reconnect token');
    await waitFor(async () => {
      const result = await browser.cdp.command('Runtime.evaluate', {
        expression: 'document.getElementById("invite-url")?.value',
        returnByValue: true,
      });
      return result.result?.value;
    }, 'copyable invite URL');
    const inviteUrl = await browser.cdp.command('Runtime.evaluate', {
      expression: 'document.getElementById("invite-url")?.value',
      returnByValue: true,
    }).then((result) => result.result?.value);
    if (inviteUrl.includes(firstToken)) throw new Error('Reconnect token leaked into the invite URL');
    await click(browser, 'choose-rider');
    await waitFor(() => browser.snapshots.some((snapshot) => snapshot.seats.rider.connected), 'Rider selected');

    gunner = await directClient(`${wsAddress}?room=${roomCode}`);
    await waitMessage(gunner, 'welcome');
    send(gunner, 'choose_seat', { seat: 'gunner' });
    await waitMessage(gunner, 'seat_assigned', (payload) => payload.seat === 'gunner');
    await waitFor(async () => browser.cdp.command('Runtime.evaluate', {
      expression: `({role: document.getElementById("rider-status")?.textContent, readyDisabled: document.getElementById("ready-button")?.disabled})`,
      returnByValue: true,
    }).then((result) => {
      const value = result.result?.value;
      return value?.role?.includes('你已選擇') && value.readyDisabled === false;
    }), 'browser role assignment reflected in the Ready button');
    await click(browser, 'ready-button');
    await waitSnapshot(gunner, (snapshot) => snapshot?.seats.rider.ready, 'browser Rider Ready');
    send(gunner, 'set_ready', { ready: true });
    await waitSnapshot(gunner, (snapshot) => snapshot?.phase === 'running', 'initial match Running');
    await waitFor(() => browser.snapshots.some((snapshot) => snapshot.phase === 'running'), 'browser sees Running');

    const welcomeCount = browser.welcomes.length;
    await browser.cdp.command('Page.reload', { ignoreCache: true });
    await waitFor(() => browser.welcomes.length > welcomeCount, 'browser refresh reconnect');
    const secondWelcome = browser.welcomes.at(-1);
    if (secondWelcome.seat !== 'rider' || secondWelcome.reconnect_token === firstToken) {
      const diagnostic = await browser.cdp.command('Runtime.evaluate', {
        expression: `({active: sessionStorage.getItem("riverside-escape-active-room"), token: sessionStorage.getItem("riverside-escape-reconnect:${roomCode}"), href: location.href, status: document.getElementById("lobby-status")?.textContent})`,
        returnByValue: true,
      }).then((result) => result.result?.value);
      throw new Error(`Browser refresh did not restore Rider and rotate the token: ${JSON.stringify({
        firstWelcome: { seat: firstWelcome.seat, room_code: firstWelcome.room_code },
        secondWelcome: { seat: secondWelcome.seat, room_code: secondWelcome.room_code,
          token_rotated: secondWelcome.reconnect_token !== firstToken },
        diagnostic,
        handshakes: browser.handshakes.map((url) => url.replace(/reconnect=[^&]+/, 'reconnect=[REDACTED]')),
      })}`);
    }
    const paused = await waitSnapshot(
      gunner,
      (snapshot) => snapshot?.phase === 'paused' && snapshot.seats.rider.connected,
      'browser refresh reconnected the reserved Rider',
    );
    const storedToken = await browser.cdp.command('Runtime.evaluate', {
      expression: `sessionStorage.getItem("riverside-escape-reconnect:${roomCode}")`,
      returnByValue: true,
    }).then((result) => result.result?.value);
    if (storedToken !== secondWelcome.reconnect_token) {
      throw new Error('Rotated reconnect token was not saved in sessionStorage');
    }
    if (!browser.handshakes.some((url) => url.includes(`reconnect=${firstToken}`))) {
      throw new Error('Refresh did not authenticate its WebSocket with the stored reconnect token');
    }
    report.checks = {
      refresh_restored_original_seat: secondWelcome.seat === 'rider',
      refresh_rotated_token: secondWelcome.reconnect_token !== firstToken,
      rotated_token_saved_in_session_storage: storedToken === secondWelcome.reconnect_token,
      reconnect_token_absent_from_invite_url: !inviteUrl.includes(firstToken),
      reconnect_token_sent_only_to_websocket: browser.handshakes.some((url) => url.includes(`reconnect=${firstToken}`)),
      room_paused_until_ready: paused.phase === 'paused',
    };
    report.browser = {
      room_code: roomCode,
      seat_after_refresh: secondWelcome.seat,
      invite_url: inviteUrl,
      websocket_handshakes: browser.handshakes.map((url) =>
        url.replace(/reconnect=[^&]+/, 'reconnect=[REDACTED]')),
      console_errors: browser.consoleErrors,
      runtime_exceptions: browser.exceptions,
    };

    await click(browser, 'ready-button');
    send(gunner, 'set_ready', { ready: true });
    await waitSnapshot(browser, (snapshot) => snapshot?.phase === 'countdown', 'resume Countdown');
    const resumed = await waitSnapshot(browser, (snapshot) => snapshot?.phase === 'running', 'resume Running');
    report.checks.ready_both_resumed_match = resumed.phase === 'running';

    const welcomesBeforeDrop = browser.welcomes.length;
    const closesBeforeDrop = browser.websocketCloses;
    const dropped = await fetch(`http://127.0.0.1:${controlPort}/drop`, { method: 'POST' })
      .then((response) => response.json());
    if (dropped.dropped_connections !== 1) {
      throw new Error(`Expected to drop only the browser WebSocket, got ${JSON.stringify(dropped)}`);
    }
    await waitFor(
      () => browser.websocketCloses > closesBeforeDrop,
      'short TCP disconnect to close the browser WebSocket',
      5000,
    );
    await waitFor(() => browser.welcomes.length > welcomesBeforeDrop,
      'automatic reconnect after a short network drop', 15000);
    const dropReconnect = browser.welcomes.at(-1);
    if (dropReconnect.seat !== 'rider'
      || dropReconnect.reconnect_token === secondWelcome.reconnect_token) {
      throw new Error('Short-drop retry did not reclaim Rider and rotate its token');
    }
    await waitSnapshot(gunner, (snapshot) => snapshot?.phase === 'paused'
      && snapshot.seats.rider.connected, 'room paused and Rider auto-reconnected');
    await waitSnapshot(browser, (snapshot) => snapshot?.phase === 'paused', 'browser auto-reconnect paused snapshot');
    await waitFor(async () => browser.cdp.command('Runtime.evaluate', {
      expression: `({role: document.getElementById("rider-status")?.textContent, readyDisabled: document.getElementById("ready-button")?.disabled})`,
      returnByValue: true,
    }).then((result) => {
      const value = result.result?.value;
      return value?.role?.includes('你已選擇') && value.readyDisabled === false;
    }), 'automatic reconnect reflected in the Ready button');
    await click(browser, 'ready-button');
    await waitSnapshot(gunner, (snapshot) => snapshot?.seats.rider.ready,
      'browser Rider Ready after short drop');
    send(gunner, 'set_ready', { ready: true });
    await waitSnapshot(browser, (snapshot) => snapshot?.phase === 'countdown',
      'resume countdown after short drop');
    await waitSnapshot(browser, (snapshot) => snapshot?.phase === 'running', 'resume after automatic reconnect');
    report.checks.short_drop_auto_reconnected = true;
    if (browser.exceptions.length || browser.consoleErrors.length) {
      throw new Error(`Browser reported errors: ${JSON.stringify({ exceptions: browser.exceptions, consoleErrors: browser.consoleErrors })}`);
    }
    await click(browser, 'leave-room-button');
    await waitMessage(gunner, 'snapshot', (snapshot) => snapshot.phase === 'finished'
      && snapshot.match_result === 'aborted');
    fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
    console.log(JSON.stringify(report, null, 2));
  } finally {
    if (gunner?.socket?.readyState === WebSocket.OPEN) gunner.socket.close();
    await closeBrowser(browser);
    server.kill('SIGTERM');
    proxy.kill('SIGTERM');
    await new Promise((resolve) => server.once('exit', resolve));
    await new Promise((resolve) => proxy.once('exit', resolve));
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});

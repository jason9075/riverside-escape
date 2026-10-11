const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const WebSocket = require('ws');

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor(check, label, timeoutMs = 12_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await sleep(25);
  }
  throw new Error(`Timed out waiting for ${label}`);
}

async function openClient(address) {
  const socket = new WebSocket(address);
  const client = { socket, messages: [], latest: null };
  socket.on('message', (data) => {
    const message = JSON.parse(data.toString());
    client.messages.push(message);
    if (message.type === 'snapshot') client.latest = message.payload;
  });
  await new Promise((resolve, reject) => {
    socket.once('open', resolve);
    socket.once('error', reject);
  });
  await waitFor(
    () => client.messages.find((message) => message.type === 'welcome')
      || client.messages.find((message) => message.type !== 'snapshot'),
    'welcome or admission result',
  );
  return client;
}

function send(client, type, payload) {
  client.socket.send(JSON.stringify(payload === undefined ? { type } : { type, payload }));
}

async function waitMessage(client, type, predicate = () => true, label = type) {
  return waitFor(
    () => client.messages.find((message) => message.type === type && predicate(message.payload)),
    label,
  );
}

async function waitSnapshot(client, predicate, label, timeoutMs = 12_000) {
  return waitFor(() => predicate(client.latest) ? client.latest : false, label, timeoutMs);
}

async function close(client) {
  if (client?.socket?.readyState === WebSocket.OPEN) client.socket.terminate();
  await sleep(100);
}

async function main() {
  const root = path.resolve(__dirname, '..');
  const serverBinary = process.env.RIVERSIDE_SERVER_BIN
    || path.join(root, 'target/debug/riverside-server');
  const reportPath = process.env.RECONNECT_SMOKE_REPORT
    || path.join(root, 'docs/verification/p5/reconnect-smoke-report.json');
  const port = Number(process.env.RECONNECT_SMOKE_PORT || 3113);
  if (!fs.existsSync(serverBinary)) throw new Error(`Server binary not found: ${serverBinary}`);
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });

  const server = spawn(serverBinary, [], {
    env: { ...process.env, RIVERSIDE_BIND: `127.0.0.1:${port}`, RIVERSIDE_DEV_SOLO: '1' },
    stdio: 'ignore',
  });
  const clients = [];
  const report = {
    generated_at: new Date().toISOString(),
    server: `127.0.0.1:${port}`,
    reconnect_window_seconds: 30,
    checks: {},
  };
  const address = `ws://127.0.0.1:${port}/ws`;
  try {
    await waitFor(async () => {
      try {
        return (await fetch(`http://127.0.0.1:${port}/health`)).ok;
      } catch {
        return false;
      }
    }, 'temporary server health');

    const rider = await openClient(`${address}?create=1`);
    clients.push(rider);
    const firstWelcome = await waitMessage(rider, 'welcome');
    const roomCode = firstWelcome.payload.room_code;
    const oldToken = firstWelcome.payload.reconnect_token;
    if (typeof oldToken !== 'string' || oldToken.length !== 64) {
      throw new Error('Public room Welcome did not include a 256-bit reconnect token');
    }
    send(rider, 'choose_seat', { seat: 'rider' });
    await waitMessage(rider, 'seat_assigned', (payload) => payload.seat === 'rider');

    const gunner = await openClient(`${address}?room=${roomCode}`);
    clients.push(gunner);
    await waitMessage(gunner, 'welcome');
    send(gunner, 'choose_seat', { seat: 'gunner' });
    await waitMessage(gunner, 'seat_assigned', (payload) => payload.seat === 'gunner');
    send(rider, 'set_ready', { ready: true });
    send(gunner, 'set_ready', { ready: true });
    await waitSnapshot(rider, (snapshot) => snapshot?.phase === 'running', 'initial Running state');
    const beforeDrop = await waitSnapshot(
      rider,
      (snapshot) => snapshot?.phase === 'running' && snapshot.tick > 400,
      'running state before disconnect',
    );
    report.initial_match = {
      room_code: roomCode,
      rider_seat: firstWelcome.payload.seat,
      bike_health: beforeDrop.bike_health,
      helicopter_health: beforeDrop.helicopter_health,
      ammo: beforeDrop.gunner_ammo,
      bike: beforeDrop.bike,
    };

    await close(rider);
    const paused = await waitSnapshot(
      gunner,
      (snapshot) => snapshot?.phase === 'paused' && snapshot.seats.rider.reserved,
      'paused room and reserved Rider seat',
    );
    await sleep(600);
    const pausedAgain = gunner.latest;
    report.checks.unexpected_disconnect_pauses_room = paused.phase === 'paused'
      && pausedAgain?.phase === 'paused'
      && pausedAgain.bike.x === paused.bike.x
      && pausedAgain.bike.z === paused.bike.z
      && pausedAgain.bike_health === paused.bike_health
      && pausedAgain.helicopter_health === paused.helicopter_health
      && pausedAgain.gunner_ammo === paused.gunner_ammo
      && pausedAgain.seats.rider.connected === false
      && pausedAgain.seats.rider.reserved === true;
    if (!report.checks.unexpected_disconnect_pauses_room) {
      throw new Error('Disconnect did not pause and preserve the match snapshot');
    }

    const reconnect = await openClient(
      `${address}?room=${roomCode}&reconnect=${oldToken}`,
    );
    clients.push(reconnect);
    const resumedWelcome = await waitMessage(reconnect, 'welcome');
    if (resumedWelcome.payload.seat !== 'rider'
      || resumedWelcome.payload.reconnect_token === oldToken) {
      throw new Error('Reconnect failed to restore the seat and rotate its token');
    }
    report.checks.reconnect_restores_seat_and_rotates_token = true;
    report.reconnect = {
      original_seat: resumedWelcome.payload.seat,
      old_token_rejected_after_rotation: null,
      duplicate_active_token_rejected: null,
    };

    const duplicate = await openClient(
      `${address}?room=${roomCode}&reconnect=${resumedWelcome.payload.reconnect_token}`,
    );
    clients.push(duplicate);
    const duplicateError = await waitMessage(duplicate, 'reconnect_in_use');
    report.reconnect.duplicate_active_token_rejected = duplicateError.type === 'reconnect_in_use';
    await close(duplicate);

    const stale = await openClient(`${address}?room=${roomCode}&reconnect=${oldToken}`);
    clients.push(stale);
    const staleError = await waitMessage(stale, 'reconnect_unavailable');
    report.reconnect.old_token_rejected_after_rotation = staleError.type === 'reconnect_unavailable';
    await close(stale);

    const afterReconnect = await waitSnapshot(
      reconnect,
      (snapshot) => snapshot?.phase === 'paused' && snapshot.seats.rider.connected,
      'reconnected Rider snapshot',
    );
    if (afterReconnect.bike_health !== paused.bike_health
      || afterReconnect.helicopter_health !== paused.helicopter_health
      || afterReconnect.gunner_ammo !== paused.gunner_ammo
      || afterReconnect.bike.x !== paused.bike.x
      || afterReconnect.bike.z !== paused.bike.z) {
      throw new Error('Reconnect changed match state before resume');
    }
    send(reconnect, 'set_ready', { ready: true });
    send(gunner, 'set_ready', { ready: true });
    const resumeCountdown = await waitSnapshot(
      reconnect,
      (snapshot) => snapshot?.phase === 'countdown',
      'resume countdown',
    );
    const resumed = await waitSnapshot(
      reconnect,
      (snapshot) => snapshot?.phase === 'running',
      'match resumed after countdown',
    );
    if (resumed.bike_health !== paused.bike_health
      || resumed.helicopter_health !== paused.helicopter_health
      || resumed.gunner_ammo !== paused.gunner_ammo) {
      throw new Error('Resume refilled or damaged preserved match state');
    }
    report.checks.ready_both_and_resume = true;
    report.resume = {
      countdown_seconds: resumeCountdown.countdown_seconds,
      bike_health_before_and_after: [paused.bike_health, resumed.bike_health],
      helicopter_health_before_and_after: [paused.helicopter_health, resumed.helicopter_health],
      ammo_before_and_after: [paused.gunner_ammo, resumed.gunner_ammo],
    };

    send(gunner, 'leave_room');
    await waitMessage(gunner, 'room_left');
    const aborted = await waitSnapshot(
      reconnect,
      (snapshot) => snapshot?.phase === 'finished' && snapshot.match_result === 'aborted',
      'explicit leave abort result',
    );
    report.checks.explicit_leave_aborts_running_match = aborted.match_result === 'aborted';
    fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
    console.log(JSON.stringify(report, null, 2));
  } finally {
    for (const client of clients) await close(client);
    server.kill('SIGTERM');
    await new Promise((resolve) => server.once('exit', resolve));
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});

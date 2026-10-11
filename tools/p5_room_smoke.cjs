const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const WebSocket = require('ws');

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor(check, label, timeoutMs = 10000) {
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
  await waitFor(() => client.messages.find((message) => message.type === 'welcome')
    || client.messages.find((message) => message.type !== 'snapshot'), 'welcome or admission result');
  return client;
}

function send(client, type, payload) {
  client.socket.send(JSON.stringify({ type, payload }));
}

async function waitMessage(client, type, predicate = () => true, label = type) {
  return waitFor(
    () => client.messages.find((message) => message.type === type && predicate(message.payload)),
    label,
  );
}

async function waitSnapshot(client, predicate, label) {
  return waitFor(() => predicate(client.latest) ? client.latest : false, label);
}

function close(client) {
  if (client?.socket?.readyState === WebSocket.OPEN) client.socket.close();
}

function seatsConnected(snapshot) {
  return snapshot?.seats?.rider?.connected && snapshot?.seats?.gunner?.connected;
}

async function main() {
  const root = path.resolve(__dirname, '..');
  const serverBinary = process.env.RIVERSIDE_SERVER_BIN
    || path.join(root, 'target/debug/riverside-server');
  const reportPath = process.env.ROOM_SMOKE_REPORT
    || path.join(root, 'docs/verification/p5/room-smoke-report.json');
  const port = Number(process.env.ROOM_SMOKE_PORT || 3112);
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
    max_active_rooms: 4,
    room_creations: [],
    role_choices: [],
    checks: {},
  };
  const address = `ws://127.0.0.1:${port}/ws`;
  try {
    await waitFor(async () => {
      try {
        const response = await fetch(`http://127.0.0.1:${port}/health`);
        return response.ok;
      } catch {
        return false;
      }
    }, 'temporary server health');

    const firstHost = await openClient(`${address}?create=1`);
    clients.push(firstHost);
    const firstWelcome = await waitMessage(firstHost, 'welcome');
    const roomA = firstWelcome.payload.room_code;
    report.room_creations.push({ code: roomA, created_by: 'first member' });
    if (firstWelcome.payload.seat !== null) throw new Error('New room member was assigned a role automatically');

    send(firstHost, 'set_ready', { ready: true });
    const unassignedSnapshot = await waitSnapshot(
      firstHost,
      (snapshot) => snapshot?.unassigned_players === 1,
      'unassigned member snapshot',
    );
    report.checks.unassigned_ready_ignored = !unassignedSnapshot.seats.rider.ready
      && !unassignedSnapshot.seats.gunner.ready
      && unassignedSnapshot.phase === 'waiting_for_players';
    if (!report.checks.unassigned_ready_ignored) throw new Error('Unassigned member became Ready');

    const firstGuest = await openClient(`${address}?room=${roomA}`);
    clients.push(firstGuest);
    const firstGuestWelcome = await waitMessage(firstGuest, 'welcome');
    if (firstGuestWelcome.payload.room_code !== roomA || firstGuestWelcome.payload.seat !== null) {
      throw new Error('Invite-code join did not enter the requested room unassigned');
    }
    send(firstHost, 'choose_seat', { seat: 'gunner' });
    send(firstGuest, 'choose_seat', { seat: 'rider' });
    await waitMessage(firstHost, 'seat_assigned', (payload) => payload.seat === 'gunner');
    await waitMessage(firstGuest, 'seat_assigned', (payload) => payload.seat === 'rider');
    await waitSnapshot(firstHost, seatsConnected, 'room A seats assigned');
    report.role_choices.push({ room: roomA, join_order: ['Gunner', 'Rider'] });

    const secondHost = await openClient(`${address}?create=1`);
    clients.push(secondHost);
    const secondWelcome = await waitMessage(secondHost, 'welcome');
    const roomB = secondWelcome.payload.room_code;
    report.room_creations.push({ code: roomB, created_by: 'second room host' });
    const secondGuest = await openClient(`${address}?room=${roomB}`);
    clients.push(secondGuest);
    await waitMessage(secondGuest, 'welcome');

    // Concurrent claims for the same seat must produce one winner; the loser
    // can then choose the other open role.
    send(secondHost, 'choose_seat', { seat: 'rider' });
    send(secondGuest, 'choose_seat', { seat: 'rider' });
    const [hostClaim, guestClaim] = await Promise.all([
      Promise.race([
        waitMessage(secondHost, 'seat_assigned', (payload) => payload.seat === 'rider'),
        waitMessage(secondHost, 'seat_choice_rejected', (payload) => payload.reason === 'seat_occupied'),
      ]),
      Promise.race([
        waitMessage(secondGuest, 'seat_assigned', (payload) => payload.seat === 'rider'),
        waitMessage(secondGuest, 'seat_choice_rejected', (payload) => payload.reason === 'seat_occupied'),
      ]),
    ]);
    const hostWon = hostClaim.type === 'seat_assigned';
    const guestWon = guestClaim.type === 'seat_assigned';
    if (hostWon === guestWon) throw new Error('Concurrent role claim did not have exactly one winner');
    const loser = hostWon ? secondGuest : secondHost;
    const loserRole = 'gunner';
    send(loser, 'choose_seat', { seat: loserRole });
    await waitMessage(loser, 'seat_assigned', (payload) => payload.seat === loserRole);
    await waitSnapshot(secondHost, seatsConnected, 'room B seats assigned');
    report.role_choices.push({ room: roomB, concurrent_claim: 'one winner, loser chose Gunner' });

    const roomAInitial = firstHost.latest;
    const roomBInitial = secondHost.latest;
    send(firstHost, 'set_ready', { ready: true });
    send(firstGuest, 'set_ready', { ready: true });
    const roomACountdown = await waitSnapshot(
      firstHost,
      (snapshot) => snapshot?.phase === 'countdown',
      'room A countdown',
    );
    await sleep(250);
    const roomBCurrent = secondHost.latest;
    report.checks.room_isolation = roomACountdown.phase === 'countdown'
      && roomBCurrent?.phase === 'ready_check'
      && roomBCurrent.tick > roomBInitial.tick
      && roomBCurrent.seats.rider.ready === false
      && roomBCurrent.seats.gunner.ready === false;
    report.checks.room_ticks = {
      room_a_start: roomAInitial.tick,
      room_a_countdown: roomACountdown.tick,
      room_b_initial: roomBInitial.tick,
      room_b_while_a_counted_down: roomBCurrent.tick,
    };
    if (!report.checks.room_isolation) throw new Error('Room A state leaked into room B');

    const fullRoomGuest = await openClient(`${address}?room=${roomA}`);
    clients.push(fullRoomGuest);
    await waitMessage(fullRoomGuest, 'room_full');
    report.checks.room_capacity_two = true;

    const thirdRoom = await openClient(`${address}?create=1`);
    clients.push(thirdRoom);
    const thirdWelcome = await waitMessage(thirdRoom, 'welcome');
    report.room_creations.push({ code: thirdWelcome.payload.room_code, created_by: 'third room host' });
    const fourthRoom = await openClient(`${address}?create=1`);
    clients.push(fourthRoom);
    const fourthWelcome = await waitMessage(fourthRoom, 'welcome');
    report.room_creations.push({ code: fourthWelcome.payload.room_code, created_by: 'fourth room host' });
    const fifthRoom = await openClient(`${address}?create=1`);
    clients.push(fifthRoom);
    await waitMessage(fifthRoom, 'room_limit_reached');
    report.checks.four_room_limit = true;

    fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
    console.log(JSON.stringify(report, null, 2));
  } finally {
    for (const client of clients) close(client);
    server.kill('SIGTERM');
  }
}

main().catch((error) => {
  console.error(error.stack || String(error));
  process.exitCode = 1;
});

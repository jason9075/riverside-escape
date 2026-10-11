const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const WebSocket = require('ws');

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor(check, label, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const result = await check();
    if (result) return result;
    await sleep(50);
  }
  throw new Error(`Timed out waiting for ${label}`);
}

function openClient(url, roomIndex, seat) {
  const socket = new WebSocket(url);
  const client = {
    socket,
    roomIndex,
    seat,
    messages: [],
    latest: null,
    snapshotCount: 0,
    snapshotBytes: 0,
    lastSnapshotAt: null,
    snapshotRates: [],
    rateWindowStartedAt: Date.now(),
    rateWindowSnapshots: 0,
    runningSnapshotCount: 0,
    finishedSnapshotCount: 0,
    sequence: 0,
    errors: [],
  };
  socket.on('message', (data) => {
    const bytes = Buffer.byteLength(data);
    client.snapshotBytes += bytes;
    try {
      const message = JSON.parse(data.toString());
      client.messages.push(message);
      if (message.type === 'snapshot') {
        client.latest = message.payload;
        client.snapshotCount += 1;
        if (message.payload.phase === 'running') client.runningSnapshotCount += 1;
        if (message.payload.phase === 'finished') client.finishedSnapshotCount += 1;
        client.rateWindowSnapshots += 1;
        client.lastSnapshotAt = Date.now();
        if (client.rateWindowStartedAt + 10_000 <= client.lastSnapshotAt) {
          const elapsed = (client.lastSnapshotAt - client.rateWindowStartedAt) / 1000;
          client.snapshotRates.push(client.rateWindowSnapshots / elapsed);
          client.rateWindowStartedAt = client.lastSnapshotAt;
          client.rateWindowSnapshots = 0;
        }
      }
    } catch (error) {
      client.errors.push(`bad message: ${error.message}`);
    }
  });
  socket.on('error', (error) => client.errors.push(error.message));
  return new Promise((resolve, reject) => {
    socket.once('open', () => resolve(client));
    socket.once('error', reject);
  });
}

function send(client, type, payload) {
  if (client.socket.readyState !== WebSocket.OPEN) return;
  client.socket.send(JSON.stringify(payload === undefined ? { type } : { type, payload }));
}

function percentile(values, rank) {
  if (!values.length) return null;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * rank) - 1)];
}

async function getMetrics(port) {
  const response = await fetch(`http://127.0.0.1:${port}/metrics`);
  if (!response.ok) throw new Error(`/metrics returned HTTP ${response.status}`);
  return response.json();
}

function readProcessUsage(pid) {
  const statm = fs.readFileSync(`/proc/${pid}/statm`, 'utf8').trim().split(/\s+/).map(Number);
  const stat = fs.readFileSync(`/proc/${pid}/stat`, 'utf8');
  const afterCommand = stat.slice(stat.lastIndexOf(')') + 2).trim().split(/\s+/);
  const userTicks = Number(afterCommand[11]);
  const systemTicks = Number(afterCommand[12]);
  return {
    rss_mb: (statm[1] * 4096) / (1024 * 1024),
    cpu_ticks: userTicks + systemTicks,
  };
}

async function main() {
  const root = path.resolve(__dirname, '..');
  const serverBinary = process.env.RIVERSIDE_SERVER_BIN
    || path.join(root, 'target/debug/riverside-server');
  const reportPath = process.env.P6_LOAD_REPORT
    || path.join(root, 'docs/verification/p6/server-load-10m.json');
  const progressPath = process.env.P6_LOAD_PROGRESS || '/tmp/riverside-p6-load-progress.jsonl';
  const port = Number(process.env.P6_LOAD_PORT || 3117);
  const durationSeconds = Number(process.env.P6_LOAD_SECONDS || 600);
  if (!fs.existsSync(serverBinary)) throw new Error(`Server binary not found: ${serverBinary}`);
  if (!Number.isFinite(durationSeconds) || durationSeconds < 1) {
    throw new Error('P6_LOAD_SECONDS must be a positive number');
  }
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });

  const server = spawn(serverBinary, [], {
    env: { ...process.env, RIVERSIDE_BIND: `127.0.0.1:${port}`, RIVERSIDE_DEV_SOLO: '1' },
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  const serverErrors = [];
  server.stderr.setEncoding('utf8');
  server.stderr.on('data', (chunk) => serverErrors.push(chunk.trim()));
  const clients = [];
  const report = {
    generated_at: new Date().toISOString(),
    environment: {
      platform: process.platform,
      architecture: process.arch,
      node: process.version,
      hostname: os.hostname(),
      duration_seconds: durationSeconds,
      rooms: 4,
      players: 8,
      client_input_rate_hz: 20,
      snapshot_target_hz: 20,
      simulation_target_hz: 60,
      traffic: 'continuous pedal/steering/braking and gunner aim inputs; no application packet drops',
      proc_clock_ticks_per_second: Number(process.env.P6_CLK_TCK || 100),
    },
    samples: [],
    check: {},
  };

  const address = `ws://127.0.0.1:${port}/ws`;
  let inputTimer;
  let metricTimer;
  let usageTimer;
  try {
    await waitFor(async () => {
      try {
        const response = await fetch(`http://127.0.0.1:${port}/health`);
        return response.ok;
      } catch {
        return false;
      }
    }, 'temporary server health');
    await waitFor(async () => {
      try {
        return (await getMetrics(port)).tick_count > 0;
      } catch {
        return false;
      }
    }, 'metrics endpoint and first simulation ticks');

    for (let roomIndex = 0; roomIndex < 4; roomIndex += 1) {
      const rider = await openClient(`${address}?create=1`, roomIndex, 'rider');
      clients.push(rider);
      await waitFor(() => rider.messages.find((message) => message.type === 'welcome'),
        `room ${roomIndex + 1} Rider welcome`);
      const roomCode = rider.messages.find((message) => message.type === 'welcome').payload.room_code;
      send(rider, 'choose_seat', { seat: 'rider' });
      await waitFor(() => rider.messages.find((message) => message.type === 'seat_assigned'),
        `room ${roomIndex + 1} Rider seat`);

      const gunner = await openClient(`${address}?room=${roomCode}`, roomIndex, 'gunner');
      clients.push(gunner);
      await waitFor(() => gunner.messages.find((message) => message.type === 'welcome'),
        `room ${roomIndex + 1} Gunner welcome`);
      send(gunner, 'choose_seat', { seat: 'gunner' });
      await waitFor(() => gunner.messages.find((message) => message.type === 'seat_assigned'),
        `room ${roomIndex + 1} Gunner seat`);
      send(rider, 'set_ready', { ready: true });
      send(gunner, 'set_ready', { ready: true });
    }

    await waitFor(() => clients.every((client) => client.latest?.phase === 'running'),
      'all four rooms to reach Running', 15_000);
    const measuredStart = Date.now();
    let lastMetricTick = (await getMetrics(port)).tick_count;
    let lastMetricTime = measuredStart;
    let lastUsage = readProcessUsage(server.pid);
    const rematchInProgress = new Set();
    let rematchCyclesStarted = 0;
    inputTimer = setInterval(() => {
      const elapsed = (Date.now() - measuredStart) / 1000;
      for (const client of clients) {
        const tick = client.latest?.tick || 0;
        client.sequence = (client.sequence + 1) >>> 0;
        if (client.seat === 'rider') {
          const steeringPhase = Math.floor(elapsed / 8) % 4;
          const steer = [0, -0.55, 0.55, 0][steeringPhase];
          send(client, 'rider_input', {
            sequence: client.sequence,
            tick,
            input: {
              pedal_held: true,
              brake_held: Math.floor(elapsed / 6) % 5 === 4,
              steer,
            },
          });
        } else {
          send(client, 'gunner_input', {
            sequence: client.sequence,
            tick,
            input: { yaw: Math.sin(elapsed / 3) * 0.08, pitch: 0, aim_held: true },
          });
        }
      }
      for (let roomIndex = 0; roomIndex < 4; roomIndex += 1) {
        const roomClients = clients.filter((client) => client.roomIndex === roomIndex);
        if (rematchInProgress.has(roomIndex)) {
          if (roomClients.length === 2 && roomClients.every((client) => client.latest?.phase === 'running')) {
            rematchInProgress.delete(roomIndex);
          }
        } else if (roomClients.length === 2
          && roomClients.every((client) => client.latest?.phase === 'finished')) {
          rematchInProgress.add(roomIndex);
          rematchCyclesStarted += 1;
          roomClients.forEach((client) => send(client, 'set_ready', { ready: true }));
        }
      }
    }, 50);
    metricTimer = setInterval(async () => {
      try {
        const now = Date.now();
        const metrics = await getMetrics(port);
        const usage = readProcessUsage(server.pid);
        const elapsed = (now - measuredStart) / 1000;
        const sampleElapsed = Math.max(0.001, (now - lastMetricTime) / 1000);
        const phaseCounts = clients.reduce((counts, client) => {
          const phase = client.latest?.phase || 'not_seen';
          counts[phase] = (counts[phase] || 0) + 1;
          return counts;
        }, {});
        const sample = {
          elapsed_seconds: Math.round(elapsed),
          tick_rate_hz: (metrics.tick_count - lastMetricTick) / sampleElapsed,
          tick_duration_us: {
            p50: metrics.tick_duration_us.p50,
            p95: metrics.tick_duration_us.p95,
            p99: metrics.tick_duration_us.p99,
          },
          snapshot_payload_bytes_per_second: metrics.snapshot_payload_bytes_per_second,
          snapshot_send_rate_hz: metrics.snapshot_send_rate_hz,
          connected_sessions: metrics.connected_sessions,
          reserved_sessions: metrics.reserved_sessions,
          active_projectiles: metrics.active_projectiles,
          rss_mb: usage.rss_mb,
          cpu_ticks: usage.cpu_ticks,
          cpu_percent_of_one_core: ((usage.cpu_ticks - lastUsage.cpu_ticks)
            / report.environment.proc_clock_ticks_per_second / sampleElapsed) * 100,
          phase_counts: phaseCounts,
          rematch_cycles_started: rematchCyclesStarted,
        };
        report.samples.push({
          ...sample,
        });
        fs.appendFileSync(progressPath, `${JSON.stringify(sample)}\n`);
        lastMetricTick = metrics.tick_count;
        lastMetricTime = now;
        lastUsage = usage;
      } catch (error) {
        serverErrors.push(`metrics sampler: ${error.message}`);
      }
    }, 10_000);
    usageTimer = setInterval(() => {
      if (!server.killed) {
        try {
          fs.readFileSync(`/proc/${server.pid}/statm`, 'utf8');
        } catch (error) {
          serverErrors.push(`server process disappeared: ${error.message}`);
        }
      }
    }, 60_000);
    await sleep(durationSeconds * 1000);
    clearInterval(inputTimer);
    clearInterval(metricTimer);
    clearInterval(usageTimer);
    inputTimer = metricTimer = usageTimer = null;

    const finalMetrics = await getMetrics(port);
    const beforeLeave = clients.map((client) => ({
      room_index: client.roomIndex,
      seat: client.seat,
      phase: client.latest?.phase,
      snapshots: client.snapshotCount,
      snapshot_bytes: client.snapshotBytes,
      running_snapshots: client.runningSnapshotCount,
      finished_snapshots: client.finishedSnapshotCount,
      snapshot_rate_p50_hz: percentile(client.snapshotRates, 0.50),
      snapshot_rate_p95_hz: percentile(client.snapshotRates, 0.95),
      errors: client.errors,
    }));
    for (const client of clients) send(client, 'leave_room');
    await waitFor(async () => {
      const metrics = await getMetrics(port);
      return metrics.connected_sessions === 0 && metrics.reserved_sessions === 0
        && metrics.active_projectiles === 0;
    }, 'all player sessions and projectiles to clear after leave', 10_000);
    const afterLeave = await getMetrics(port);
    report.run = {
      measured_seconds: (Date.now() - measuredStart) / 1000,
      client_snapshot_summary: beforeLeave,
      tick_count: finalMetrics.tick_count,
      tick_duration_us: {
        p50: finalMetrics.tick_duration_us.p50,
        p95: finalMetrics.tick_duration_us.p95,
        p99: finalMetrics.tick_duration_us.p99,
        sample_count: finalMetrics.tick_duration_us.sample_count,
      },
      snapshot_count: finalMetrics.snapshot_count,
      snapshot_payload_bytes_total: finalMetrics.snapshot_payload_bytes_total,
      snapshot_payload_bytes_per_second: finalMetrics.snapshot_payload_bytes_per_second,
      snapshot_send_rate_hz: finalMetrics.snapshot_send_rate_hz,
      room_count: finalMetrics.room_count,
      connected_sessions_after_leave: afterLeave.connected_sessions,
      reserved_sessions_after_leave: afterLeave.reserved_sessions,
      active_projectiles_after_leave: afterLeave.active_projectiles,
      retained_empty_rooms_after_leave: afterLeave.room_count,
      server_errors: serverErrors,
    };
    const tickRates = report.samples.map((sample) => sample.tick_rate_hz);
    const rssValues = report.samples.map((sample) => sample.rss_mb);
    report.check = {
      four_rooms_eight_players: clients.length === 8 && finalMetrics.room_count === 4,
      running_snapshots_for_every_player: beforeLeave.every((client) => client.running_snapshots
        >= durationSeconds * 15),
      tick_p50_below_8ms: finalMetrics.tick_duration_us.p50 < 8000,
      tick_p95_below_8ms: finalMetrics.tick_duration_us.p95 < 8000,
      tick_p99_below_16_7ms: finalMetrics.tick_duration_us.p99 < 16_700,
      tick_rate_near_60_hz: percentile(tickRates, 0.50) >= 59
        && percentile(tickRates, 0.50) <= 61,
      no_session_projectile_leak: afterLeave.connected_sessions === 0
        && afterLeave.reserved_sessions === 0 && afterLeave.active_projectiles === 0,
      bounded_rss_drift_mb: rssValues.length > 1 ? Math.max(...rssValues) - Math.min(...rssValues) : null,
      no_server_errors: serverErrors.length === 0 && clients.every((client) => client.errors.length === 0),
    };
    report.check.bounded_rss_drift_pass = report.check.bounded_rss_drift_mb === null
      || report.check.bounded_rss_drift_mb < 64;
    report.metrics_samples = report.samples;
    fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
    console.log(JSON.stringify(report, null, 2));
    if (Object.entries(report.check).some(([key, value]) => key !== 'bounded_rss_drift_mb' && value === false)) {
      process.exitCode = 1;
    }
  } catch (error) {
    report.error = error.stack || String(error);
    report.server_errors = serverErrors;
    fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
    console.error(report.error);
    process.exitCode = 1;
  } finally {
    clearInterval(inputTimer);
    clearInterval(metricTimer);
    clearInterval(usageTimer);
    for (const client of clients) {
      try { client.socket.terminate(); } catch { /* best effort test cleanup */ }
    }
    server.kill('SIGTERM');
  }
}

main();

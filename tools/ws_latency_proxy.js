#!/usr/bin/env node
"use strict";

// Adds fixed one-way delay to a dedicated WebSocket TCP connection without
// changing host qdiscs or touching unrelated traffic. This is a latency-only
// test proxy; it does not inject jitter or packet loss.

const net = require("node:net");

function argument(name, fallback) {
  const index = process.argv.indexOf(name);
  return index < 0 ? fallback : Number(process.argv[index + 1]);
}

const listenPort = argument("--listen-port", 3101);
const targetPort = argument("--target-port", 3001);
const rttMs = argument("--rtt-ms", 50);
const oneWayDelayMs = rttMs / 2;
const listenHost = "127.0.0.1";
const targetHost = "127.0.0.1";

if (![listenPort, targetPort, rttMs].every(Number.isFinite) || rttMs <= 0) {
  console.error("Usage: ws_latency_proxy.js --rtt-ms <positive-ms> [--listen-port <port>]");
  process.exit(2);
}

const server = net.createServer((client) => {
  const upstream = net.connect({ host: targetHost, port: targetPort });
  client.setNoDelay(true);
  upstream.setNoDelay(true);

  const relay = (source, destination) => {
    const queue = [];
    let scheduled = false;
    let writing = false;
    let sourceEnded = false;

    const schedule = () => {
      if (scheduled || writing) return;
      const next = queue[0];
      if (!next) {
        if (sourceEnded) destination.end();
        return;
      }

      scheduled = true;
      const waitMs = Math.max(0, next.releaseAt - performance.now());
      setTimeout(() => {
        scheduled = false;
        writing = true;
        queue.shift();
        destination.write(next.chunk, (error) => {
          writing = false;
          if (error) {
            source.destroy();
            return;
          }
          schedule();
        });
      }, waitMs);
    };

    source.on("data", (chunk) => {
      queue.push({ chunk, releaseAt: performance.now() + oneWayDelayMs });
      schedule();
    });

    source.on("end", () => {
      sourceEnded = true;
      schedule();
    });
    source.on("error", () => destination.destroy());
    destination.on("error", () => source.destroy());
  };

  relay(client, upstream);
  relay(upstream, client);
});

server.on("error", (error) => {
  console.error(`Proxy error: ${error.message}`);
  process.exitCode = 1;
});

server.listen(listenPort, listenHost, () => {
  console.log(
    `WebSocket latency proxy 127.0.0.1:${listenPort} -> ${targetHost}:${targetPort}; ` +
      `fixed ${oneWayDelayMs} ms each direction (RTT target ${rttMs} ms; jitter/loss 0)`,
  );
});

const http = require('node:http');
const net = require('node:net');

function argument(name, fallback) {
  const index = process.argv.indexOf(name);
  return index < 0 ? fallback : Number(process.argv[index + 1]);
}

const listenPort = argument('--listen-port', 3115);
const controlPort = argument('--control-port', 3116);
const targetPort = argument('--target-port', 3114);
const active = new Map();
let nextId = 1;

const proxy = net.createServer((client) => {
  const id = nextId++;
  const upstream = net.connect({ host: '127.0.0.1', port: targetPort });
  client.setNoDelay(true);
  upstream.setNoDelay(true);
  active.set(id, { client, upstream });
  const closePair = () => {
    active.delete(id);
    client.destroy();
    upstream.destroy();
  };
  client.on('error', closePair);
  upstream.on('error', closePair);
  client.on('close', () => active.delete(id));
  upstream.on('close', () => active.delete(id));
  client.pipe(upstream);
  upstream.pipe(client);
});

const control = http.createServer((request, response) => {
  if (request.method === 'GET' && request.url === '/health') {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ active_connections: active.size }));
    return;
  }
  if (request.method === 'POST' && request.url === '/drop') {
    const count = active.size;
    for (const pair of active.values()) {
      pair.client.destroy();
      pair.upstream.destroy();
    }
    active.clear();
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ dropped_connections: count }));
    return;
  }
  response.writeHead(404);
  response.end();
});

proxy.listen(listenPort, '127.0.0.1', () => {
  console.log(`WebSocket disconnect proxy 127.0.0.1:${listenPort} -> 127.0.0.1:${targetPort}`);
});
control.listen(controlPort, '127.0.0.1', () => {
  console.log(`Test-only disconnect control bound to 127.0.0.1:${controlPort}`);
});

function shutdown() {
  for (const pair of active.values()) {
    pair.client.destroy();
    pair.upstream.destroy();
  }
  proxy.close();
  control.close();
}

process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);

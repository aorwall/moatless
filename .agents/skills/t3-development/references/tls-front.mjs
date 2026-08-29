/**
 * TLS front door for the Vite dev server, so a browser can reach it on an
 * allow-listed **https** origin without editing t3code's `vite.config.ts`.
 *
 * The backend's `T3_UI_RPC_ALLOWED_ORIGINS` lists https origins only, and the
 * Moatless session cookie is `Secure`, so a plain `http://localhost:5733` page
 * cannot open the socket or carry auth. Vite has no HTTPS switch that is not a
 * config edit, and `vite.config.ts` is an upstream file — a fork delta paid at
 * every merge for something only a test needs.
 *
 * This terminates TLS and pipes the decrypted bytes straight to Vite over TCP.
 * A passthrough rather than an HTTP proxy on purpose: after termination it is
 * ordinary HTTP/1.1, so the app's `/ws` socket and Vite's HMR socket ride the
 * same connection upgrade with nothing to special-case.
 *
 *   node tls-front.mjs --listen 8444 --target 5733 \
 *     --cert /tmp/pw/cert.pem --key /tmp/pw/key.pem
 */
import * as net from "node:net";
import * as tls from "node:tls";
import * as fs from "node:fs";

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? fallback : process.argv[i + 1];
};

const listenPort = Number(arg("listen", 8444));
const targetPort = Number(arg("target", 5733));
const targetHost = arg("target-host", "127.0.0.1");
const cert = fs.readFileSync(arg("cert", "/tmp/pw/cert.pem"));
const key = fs.readFileSync(arg("key", "/tmp/pw/key.pem"));

const server = tls.createServer({ cert, key }, (client) => {
  const upstream = net.connect(targetPort, targetHost);
  // Either half closing or erroring tears down the pair; without this a failed
  // upstream connect leaves the browser waiting on a socket nobody will answer.
  const end = () => {
    client.destroy();
    upstream.destroy();
  };
  client.on("error", end);
  upstream.on("error", end);
  client.pipe(upstream);
  upstream.pipe(client);
});

server.on("error", (error) => {
  console.error(`tls-front failed: ${error.message}`);
  process.exit(1);
});

server.listen(listenPort, "127.0.0.1", () => {
  console.log(`tls-front: https://127.0.0.1:${listenPort} -> http://${targetHost}:${targetPort}`);
});

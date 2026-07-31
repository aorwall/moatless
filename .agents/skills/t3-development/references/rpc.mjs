// Headless T3 UI-RPC probe against the shadow Moatless backend.
//
// Trades $MOATLESS_API_KEY for a WebSocket ticket, opens /ws, sends one
// Effect-RPC Request envelope and prints every frame it gets back with its
// serialized size — so it doubles as a payload-size check.
//
// A ticket-authenticated upgrade skips the Origin check, which is why this
// works from localhost while a browser on localhost:5733 gets a 403.
//
//   node rpc.mjs <tag> [payloadJson] [--stream] [--wait=ms]
//
//   node rpc.mjs server.getConfig
//   node rpc.mjs orchestration.subscribeShell '{}' --stream --wait=6000
//   node rpc.mjs orchestration.subscribeThread '{"threadId":"..."}' --stream

const BASE = process.env.MOATLESS_BASE ?? "http://localhost:8080";
const KEY = process.env.MOATLESS_API_KEY;
if (!KEY) throw new Error("MOATLESS_API_KEY is not set");

const [tag, payloadArg, ...flags] = process.argv.slice(2);
if (!tag) throw new Error("usage: node rpc.mjs <tag> [payloadJson] [--stream] [--wait=ms]");
const stream = flags.includes("--stream");
const waitMs = Number(
  flags.find((f) => f.startsWith("--wait="))?.slice(7) ?? (stream ? 8000 : 5000),
);

const ticketRes = await fetch(`${BASE}/api/auth/websocket-ticket`, {
  method: "POST",
  headers: { authorization: `Bearer ${KEY}` },
});
if (!ticketRes.ok) throw new Error(`ticket ${ticketRes.status}: ${await ticketRes.text()}`);
const { ticket } = await ticketRes.json();

const ws = new WebSocket(
  `${BASE.replace(/^http/, "ws")}/ws?wsTicket=${encodeURIComponent(ticket)}`,
);
const frames = [];

ws.addEventListener("open", () => {
  ws.send(
    JSON.stringify([
      {
        _tag: "Request",
        id: 1,
        tag,
        payload: payloadArg ? JSON.parse(payloadArg) : {},
        headers: [],
      },
    ]),
  );
});

ws.addEventListener("message", (ev) => {
  for (const frame of [].concat(JSON.parse(ev.data))) {
    // The server sends Effect keepalives; answer them or it closes the stream.
    if (frame._tag === "Ping") {
      ws.send(JSON.stringify({ _tag: "Pong" }));
      continue;
    }
    frames.push(frame);
    const size = JSON.stringify(frame).length;
    console.log(`<- ${frame._tag}${frame.exit?._tag ? `/${frame.exit._tag}` : ""} ${size}B`);
    // Chunks must be acked or the stream stalls waiting on the client.
    if (frame._tag === "Chunk")
      ws.send(JSON.stringify({ _tag: "Ack", requestId: frame.requestId }));
    if (frame._tag === "Exit" && !stream) ws.close();
  }
});

ws.addEventListener("error", (e) => console.error("ws error", e.message ?? e));

await new Promise((resolve) => setTimeout(resolve, waitMs));
try {
  ws.close();
} catch {}

console.log(JSON.stringify(frames, null, 2).slice(0, 2000));

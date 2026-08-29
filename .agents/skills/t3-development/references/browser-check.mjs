/**
 * Headless smoke check of the T3 UI against a Moatless backend.
 *
 * Asserts the three things a broken integration gets wrong and a screenshot
 * does not tell you: the app's `/ws` socket opens, a project name the backend
 * actually holds renders in the sidebar (rather than the convincing "No
 * projects yet" empty state a 403'd upgrade produces), and the console is
 * clean.
 *
 * Expects a TLS front door on --port for an allow-listed https origin (see
 * tls-front.mjs) and mints its own session from MOATLESS_API_KEY.
 *
 *   node browser-check.mjs --backend http://localhost:8080 \
 *     --host moatless-t3.staging.moatless.xyz --port 8444
 */
import { createRequire } from "node:module";
import * as fs from "node:fs";
import * as path from "node:path";

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? fallback : process.argv[i + 1];
};

// Playwright is installed in a scratch directory, not in either checkout, and
// ESM resolves from *this file's* location rather than the cwd — so a bare
// `import "playwright"` here cannot find it however you invoke the script.
// Resolve it from the scratch dir explicitly instead.
const scratch = arg("scratch", process.env.PW_SCRATCH ?? "/tmp/pw");
const { chromium } = createRequire(`${scratch}/`)("playwright");

/**
 * The chromium the image baked, whatever build that is.
 *
 * Playwright only launches the build its own version pins, and the npm release
 * that pins the baked one drifts as the image is rebuilt — so matching versions
 * is a chase. Naming the binary skips the question: any recent Playwright can
 * drive an older chrome. Returns undefined when nothing is baked, which lets
 * Playwright fall back to its own download.
 */
function bakedChromium() {
  const root = process.env.PLAYWRIGHT_BROWSERS_PATH ?? "/opt/playwright-browsers";
  const dir = fs
    .readdirSync(root, { withFileTypes: true })
    .filter((e) => e.isDirectory() && /^chromium-\d+$/.test(e.name))
    .map((e) => e.name)
    .sort((a, b) => Number(b.split("-")[1]) - Number(a.split("-")[1]))[0];
  if (!dir) return undefined;
  const bin = path.join(root, dir, "chrome-linux", "chrome");
  return fs.existsSync(bin) ? bin : undefined;
}

const HOST = arg("host", "moatless-t3.staging.moatless.xyz");
const PORT = arg("port", "8444");
const BACKEND = arg("backend", "http://localhost:8080");
const KEY = process.env.MOATLESS_API_KEY;
if (!KEY) throw new Error("MOATLESS_API_KEY is not set");
const bearer = { authorization: `Bearer ${KEY}`, "content-type": "application/json" };

// The cookie is minted off the API key: the browser has no other way in, and
// the socket needs a real session even though the HTTP routes take the bearer.
const minted = await fetch(`${BACKEND}/api/v1/auth/api-key/session`, {
  method: "POST",
  headers: bearer,
  body: "{}",
});
const setCookie = (
  minted.headers.getSetCookie ? minted.headers.getSetCookie() : [minted.headers.get("set-cookie")]
).join("\n");
const session = /moatless_session=([^;]+)/.exec(setCookie)?.[1];
if (!session) throw new Error(`could not mint a session (${minted.status})`);

// What the sidebar should be showing, straight from the backend. Comparing
// against live data rather than a hardcoded name is what makes this a check of
// the *integration* and not of a fixture.
const listed = await fetch(`${BACKEND}/api/v1/tasks?limit=20`, { headers: bearer });
const expectedNames = [
  ...new Set(
    ((await listed.json()).items ?? [])
      .map((task) => task.repositoryName)
      .filter((name) => typeof name === "string" && name.length > 0),
  ),
];
if (expectedNames.length === 0) throw new Error("backend lists no projects; nothing to assert");

const browser = await chromium.launch({
  executablePath: bakedChromium(),
  args: [
    "--no-sandbox",
    // Makes the allow-listed name resolve to the local front door, so the
    // browser sends the Origin the backend accepts without any DNS or ingress.
    `--host-resolver-rules=MAP ${HOST} 127.0.0.1:${PORT}`,
    "--ignore-certificate-errors",
  ],
});
const context = await browser.newContext({ ignoreHTTPSErrors: true });
// Domain is the parent so the cookie also covers *.preview hosts.
await context.addCookies([
  {
    name: "moatless_session",
    value: session,
    domain: ".moatless.xyz",
    path: "/",
    secure: true,
    httpOnly: true,
    sameSite: "Lax",
  },
]);

const page = await context.newPage();
const errors = [];
const sockets = [];
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
page.on("pageerror", (e) => errors.push(`PAGEERROR ${e.message}`));
// Only the app's socket. Vite's HMR socket is a separate one on the dev
// server's own origin and path "/", so match the path explicitly.
page.on("websocket", (ws) => /\/ws(\?|$)/.test(ws.url()) && sockets.push(ws.url()));

await page.goto(`https://${HOST}/`, { waitUntil: "domcontentloaded", timeout: 60_000 });

// Poll for a real project name. Do NOT poll on "No projects yet" clearing:
// that string disappears on first paint, well before the socket has delivered
// anything, so a check that waits on its absence passes against a blank page.
const deadline = Date.now() + 90_000;
let shown = null;
let body = "";
while (Date.now() < deadline && !shown) {
  body = await page.locator("body").innerText();
  shown = expectedNames.find((name) => body.includes(name)) ?? null;
  if (!shown) await page.waitForTimeout(1000);
}

console.log(`app /ws sockets: ${sockets.length}`);
console.log(`project shown:   ${shown ?? "NONE"} (backend lists ${expectedNames.length})`);
console.log(`empty state:     ${body.includes("No projects yet")}`);
console.log(`console errors:  ${errors.length}`);
for (const e of errors.slice(0, 8)) console.log(`  - ${e.slice(0, 160)}`);
await page.screenshot({ path: arg("screenshot", `${scratch}/browser-check.png`) });

await browser.close();
const ok = sockets.length === 1 && Boolean(shown) && errors.length === 0;
console.log(ok ? "PASS" : "FAIL");
process.exit(ok ? 0 : 1);

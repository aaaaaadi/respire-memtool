import { createServer } from "node:http";
import { createReadStream, existsSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};

const dir = resolve(arg("--dir", join(here, "updates")));
const port = Number(arg("--port", process.env.UPDATE_PORT || 17651));
const rate = Number(process.env.UPDATE_RATE || 0);

function streamAtRate(file, bytesPerSec, res) {
  const stream = createReadStream(file, { highWaterMark: 64 * 1024 });
  const started = Date.now();
  let sent = 0;
  stream.on("data", (chunk) => {
    sent += chunk.length;
    const delay = Math.max(0, (sent / bytesPerSec) * 1000 - (Date.now() - started));
    if (delay > 0) {
      stream.pause();
      setTimeout(() => stream.resume(), Math.min(delay, 400));
    }
  });
  stream.pipe(res);
}

const server = createServer((req, res) => {
  const url = new URL(req.url || "/", "http://127.0.0.1");
  if (url.pathname === "/health") {
    res.writeHead(200, { "Content-Type": "text/plain; charset=utf-8" });
    res.end("ok");
    return;
  }

  const name = decodeURIComponent(url.pathname.replace(/^\/+/, ""));
  const file = resolve(dir, name);
  if (!file.startsWith(dir) || !existsSync(file) || !statSync(file).isFile()) {
    res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
    res.end("not found");
    return;
  }

  const isManifest = name === "update.json";
  res.writeHead(200, {
    "Content-Type": isManifest
      ? "application/json; charset=utf-8"
      : "application/octet-stream",
    "Content-Length": statSync(file).size,
    "Cache-Control": "no-store",
    "Access-Control-Allow-Origin": "*",
  });
  if (isManifest) res.end(readFileSync(file));
  else if (rate > 0 && statSync(file).size / rate > 0.3) streamAtRate(file, rate, res);
  else createReadStream(file).pipe(res);
});

server.listen(port, "127.0.0.1", () => {
  console.log(`[update] serving ${dir}`);
  console.log(`[update] manifest http://127.0.0.1:${port}/update.json`);
});

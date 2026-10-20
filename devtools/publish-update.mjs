import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const conf = JSON.parse(
  readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8")
);
const exe = join(root, "src-tauri", "target", "release", "memtools.exe");
if (!existsSync(exe)) {
  console.error("缺少 src-tauri/target/release/memtools.exe，先跑：npm run tauri build -- --no-bundle");
  process.exit(1);
}

const outDir = join(root, "devtools", "updates");
mkdirSync(outDir, { recursive: true });
const name = `memtools-${conf.version}.exe`;
const target = join(outDir, name);
copyFileSync(exe, target);

const buf = readFileSync(target);
const port = process.env.UPDATE_PORT || 17651;
const notes = process.argv.slice(2).join(" ").trim() || `memTools ${conf.version}`;
const manifest = {
  version: conf.version,
  notes,
  pub_date: new Date().toISOString(),
  url: `http://127.0.0.1:${port}/${name}`,
  sha256: createHash("sha256").update(buf).digest("hex"),
  size: buf.length,
  mandatory: false,
};
writeFileSync(join(outDir, "update.json"), JSON.stringify(manifest, null, 2) + "\n");

console.log(`[publish] v${conf.version}  ${name}  ${buf.length} bytes`);
console.log(`[publish] sha256 ${manifest.sha256}`);
console.log(`[publish] manifest ${join(outDir, "update.json")}`);

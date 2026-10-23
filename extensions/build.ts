// Builds every extension in src/ into dist/<id>.js and writes dist/index.json + dist/dubs.json.
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Manifest } from "./src/types";

const SRC = "src";
const DIST = "dist";

const entries = readdirSync(SRC).filter(
  (f) => f.endsWith(".ts") && f !== "types.ts" && !f.endsWith(".test.ts"),
);

rmSync(DIST, { recursive: true, force: true });
mkdirSync(DIST, { recursive: true });

const extensions = [];
for (const file of entries) {
  const mod = (await import(`./${SRC}/${file}`)) as { manifest: Manifest };
  const m = mod.manifest;
  const result = await Bun.build({
    entrypoints: [join(SRC, file)],
    outdir: DIST,
    target: "browser",
    format: "esm",
    minify: false,
    naming: `${m.id}.js`,
  });
  if (!result.success) {
    console.error(result.logs);
    process.exit(1);
  }
  const code = readFileSync(join(DIST, `${m.id}.js`));
  const sha256 = createHash("sha256").update(code).digest("hex");
  extensions.push({
    id: m.id,
    name: m.name,
    version: m.version,
    apiVersion: m.apiVersion,
    file: `${m.id}.js`,
    sha256,
    domains: m.domains,
    allowedHosts: m.allowedHosts ?? [],
  });
  console.log(`built ${m.id}@${m.version} ${sha256.slice(0, 12)}`);
}

copyFileSync("dubs.json", join(DIST, "dubs.json"));
writeFileSync(
  join(DIST, "index.json"),
  JSON.stringify({ name: "ninari-extensions", dubs: "dubs.json", extensions }, null, 2) + "\n",
);
console.log(`index.json: ${extensions.length} extension(s)`);

// Builds one extension into a temp file and runs the sandbox check on it only:
//   bun run check-one.ts src/yummy.ts <out-dir>
import { mkdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";

const [entry, outDir = "tmp-build"] = process.argv.slice(2);
if (!entry) throw new Error("usage: bun run check-one.ts src/<ext>.ts [out-dir]");
mkdirSync(outDir, { recursive: true });
const name = basename(entry).replace(/\.ts$/, ".js");
const result = await Bun.build({ entrypoints: [entry], outdir: outDir, target: "browser", format: "esm", naming: name });
if (!result.success) {
  console.error(result.logs);
  process.exit(1);
}
const file = join(outDir, name);
const code = readFileSync(file, "utf8");
const FORBIDDEN: [RegExp, string][] = [
  [/\bnew URL\(|(?<![.\w])URL\(/, "URL"],
  [/\bURLSearchParams\b/, "URLSearchParams"],
  [/(?<![.\w])atob\(|(?<![.\w])btoa\(/, "atob/btoa (use ctx.base64)"],
  [/\bTextDecoder\b|\bTextEncoder\b/, "TextDecoder/TextEncoder"],
  [/(?<![.\w])setTimeout\(|(?<![.\w])setInterval\(/, "timers"],
  [/(?<![.\w])fetch\(/, "global fetch (use ctx.fetch)"],
  [/\bDOMParser\b|\bdocument\.|\bwindow\./, "DOM (use ctx.html.select)"],
  [/\bBuffer\./, "Buffer"],
  [/\bcrypto\./, "crypto"],
  [/\brequire\(/, "require"],
];
let bad = false;
for (const [re, what] of FORBIDDEN) if (re.test(code)) (console.error(`${name}: uses ${what}`), (bad = true));
if (bad) process.exit(1);
console.log(`ok: ${file}`);

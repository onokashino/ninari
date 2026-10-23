// Builds the extensions and fails if a bundle uses web APIs the ninari QuickJS sandbox
// does not have. Extensions must go through ctx (ctx.fetch, ctx.base64, ctx.html).
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

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

const build = Bun.spawnSync(["bun", "run", "build.ts"], { stdout: "inherit", stderr: "inherit" });
if (build.exitCode !== 0) process.exit(build.exitCode ?? 1);

let failed = false;
for (const file of readdirSync("dist").filter((f) => f.endsWith(".js"))) {
  const code = readFileSync(join("dist", file), "utf8");
  for (const [re, what] of FORBIDDEN) {
    if (re.test(code)) {
      console.error(`${file}: uses ${what}, which the sandbox does not provide`);
      failed = true;
    }
  }
}
if (failed) process.exit(1);
console.log("sandbox check passed");

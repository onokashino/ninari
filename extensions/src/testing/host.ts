// Lightweight stand-in for the ninari host, for unit tests under bun.
// It uses the global fetch directly, so tests that call it hit the live network.

import type { Ctx } from "../types";

export function makeCtx(baseUrl: string): Ctx {
  const kv = new Map<string, string>();
  const fail = (kind: string) => (msg?: string): never => {
    throw new Error(`__EXT:${kind}:${msg ?? ""}`);
  };
  return {
    baseUrl,
    async fetch(url, init) {
      const r = await fetch(url, { method: init?.method, headers: init?.headers, body: init?.body });
      const body = await r.text();
      return {
        status: r.status,
        url: r.url,
        ok: r.ok,
        headers: Object.fromEntries(r.headers.entries()),
        text: async () => body,
        json: async () => JSON.parse(body),
      };
    },
    html: {
      select() {
        throw new Error("html.select is not available in the test host");
      },
    },
    kv: {
      get: async (k) => kv.get(k) ?? null,
      set: async (k, v) => void kv.set(k, v),
    },
    base64: {
      encode: (s) => btoa(s),
      decode: (s) => atob(s),
    },
    fail: {
      geoBlocked: fail("GeoBlocked"),
      notFound: fail("NotFound"),
      challenge: fail("Challenge"),
    },
    log: (...a) => console.log(...a),
  };
}

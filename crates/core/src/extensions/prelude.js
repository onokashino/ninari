// Runs before every extension module. Builds the `ctx` object on top of the __host_* functions.
// Errors that the core must classify are thrown as "__EXT:<Kind>:<message>".

globalThis.__call = async (f, args, ctx) => f(...args, ctx);

globalThis.__makeCtx = (baseUrl) => ({
  baseUrl,
  async fetch(url, init) {
    const raw = await __host_fetch(String(url), JSON.stringify(init ?? {}));
    const r = JSON.parse(raw);
    if (r.error) throw new Error("__EXT:" + r.error.kind + ":" + r.error.message);
    const body = r.body;
    return {
      status: r.status,
      url: r.url,
      headers: Object.fromEntries(r.headers),
      ok: r.status >= 200 && r.status < 300,
      text: async () => body,
      json: async () => JSON.parse(body),
    };
  },
  html: {
    select: (html, css) => JSON.parse(__host_html_select(String(html), String(css))),
  },
  kv: {
    get: async (key) => {
      const v = __host_kv_get(String(key));
      return v === undefined ? null : v;
    },
    set: async (key, value, ttlSec) => {
      __host_kv_set(String(key), String(value), Number(ttlSec ?? 0));
    },
  },
  base64: {
    encode: (s) => __host_b64(String(s), true),
    decode: (s) => __host_b64(String(s), false),
  },
  fail: {
    geoBlocked: (msg) => {
      throw new Error("__EXT:GeoBlocked:" + (msg ?? ""));
    },
    notFound: (msg) => {
      throw new Error("__EXT:NotFound:" + (msg ?? ""));
    },
    challenge: (url) => {
      throw new Error("__EXT:Challenge:" + (url ?? ""));
    },
  },
  log: (...args) => __host_log(args.map(String).join(" ")),
});

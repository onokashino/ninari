// src/anilibria.ts
var manifest = {
  id: "anilibria",
  name: "AniLibria",
  version: "0.1.0",
  apiVersion: 1,
  domains: ["https://anilibria.top", "https://api.anilibria.app", "https://aniliberty.top"],
  allowedHosts: ["cache.libria.fun"]
};
var DUB_ID = "aniliberty";
var QUALITIES = [1080, 720, 480];
var span = (x) => x && x.start != null && x.stop != null ? [x.start, x.stop] : undefined;
var maxQuality = (e) => QUALITIES.find((q) => e[`hls_${q}`]);
async function release(id, ctx) {
  const res = await ctx.fetch(`${ctx.baseUrl}/api/v1/anime/releases/${encodeURIComponent(id)}`);
  if (res.status === 404)
    ctx.fail.notFound(`release ${id}`);
  if (!res.ok)
    throw new Error(`anilibria: HTTP ${res.status}`);
  const r = await res.json();
  if (r.is_blocked_by_geo)
    ctx.fail.geoBlocked("AniLibria закрыла релиз в этом регионе");
  return r;
}
async function search(q, ctx) {
  const fuzzy = new Map;
  for (const name of q.names.slice(0, 4)) {
    const res = await ctx.fetch(`${ctx.baseUrl}/api/v1/app/search/releases?query=${encodeURIComponent(name)}`);
    if (!res.ok)
      continue;
    const items = await res.json();
    for (const r of items) {
      const title = r.name?.main ?? name;
      if (r.shikimori?.id === q.shikimoriId)
        return [{ id: String(r.id), title, confidence: "exact" }];
      if (!fuzzy.has(r.id))
        fuzzy.set(r.id, { id: String(r.id), title, confidence: "fuzzy" });
    }
  }
  return [...fuzzy.values()];
}
async function episodes(m, ctx) {
  const r = await release(m.id, ctx);
  return {
    dubs: [{ id: DUB_ID, name: "AniLibria", type: "voice" }],
    episodes: (r.episodes ?? []).map((e) => ({
      number: e.ordinal,
      available: [{ dubId: DUB_ID, maxQuality: maxQuality(e) }],
      skip: { opening: span(e.opening), ending: span(e.ending) }
    }))
  };
}
async function resolve(m, episode, _dubId, ctx) {
  const r = await release(m.id, ctx);
  const e = (r.episodes ?? []).find((x) => x.ordinal === episode);
  if (!e)
    ctx.fail.notFound(`episode ${episode}`);
  const out = [];
  for (const q of QUALITIES) {
    const url = e[`hls_${q}`];
    if (url)
      out.push({ url, kind: "hls", quality: q });
  }
  if (out.length === 0)
    ctx.fail.notFound(`episode ${episode} has no streams`);
  return out;
}
export {
  episodes,
  manifest,
  resolve,
  search
};

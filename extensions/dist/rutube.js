// src/rutube.ts
var manifest = {
  id: "rutube",
  name: "Rutube",
  version: "0.1.0",
  apiVersion: 1,
  domains: ["https://rutube.ru"],
  allowedHosts: ["*.rutube.ru", "*.rtbcdn.ru"]
};
var HEADERS = { Accept: "application/json" };
var VIDEOS_TTL = 600;
var QUALITY_TTL = 7 * 24 * 3600;
var MAX_PAGES = 20;
var normalize = (s) => s.toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");
var CHINESE_DIGITS = { 一: 1, 二: 2, 三: 3, 四: 4, 五: 5, 六: 6, 七: 7, 八: 8, 九: 9, 十: 10 };
var PART = /\b(?:part|cour)\s*\d|(?<![а-яё])часть(?![а-яё])/i;
var MARKERS = [
  /\s*\b(\d+)(?:st|nd|rd|th)\s+season\b/i,
  /\s*\bseason\s+(\d+)\b/i,
  /\s*\b(\d+)(?:-?й)?\s+сезон(?![а-яё])/i,
  /\s*第([一二三四五六七八九十\d]+)季/
];
var TRAILING = /\s+(\d{1,2})$/;
var toNumber = (s) => /^\d+$/.test(s) ? Number(s) : CHINESE_DIGITS[s] ?? NaN;
function seasonOf(names) {
  if (names.some((n) => PART.test(n)))
    return null;
  let marked;
  let trailing;
  const bases = [];
  for (const name of names) {
    let base = name.trim();
    for (const re of MARKERS) {
      const m = re.exec(base);
      if (!m)
        continue;
      marked ??= toNumber(m[1]);
      base = base.replace(re, "").trim();
    }
    const t = TRAILING.exec(base);
    if (t && Number(t[1]) >= 2 && Number(t[1]) <= 20) {
      trailing ??= Number(t[1]);
      base = base.replace(TRAILING, "").trim();
    }
    if (base && !bases.includes(base))
      bases.push(base);
  }
  const season = marked ?? trailing ?? 1;
  return Number.isFinite(season) ? { season, names: bases } : null;
}
function feedMatches(feedName, names) {
  const parts = feedName.split("/").map(normalize).filter(Boolean);
  return names.some((n) => parts.includes(normalize(n)));
}
var showWide = (title) => {
  const m = /\((\d+)\s*серия\)/i.exec(title);
  return m ? Number(m[1]) : undefined;
};
function episodeMap(videos, season, limit) {
  const byAuthor = new Map;
  for (const v of videos) {
    if (v.season < 1 || v.episode < 1)
      continue;
    const key = String(v.author.id);
    byAuthor.set(key, [...byAuthor.get(key) ?? [], v]);
  }
  const out = new Map;
  for (const [author, list] of byAuthor) {
    const starts = list.filter((v) => v.season === season).map((v) => {
      const n = showWide(v.title);
      return n === undefined ? undefined : n - v.episode + 1;
    }).filter((n) => n !== undefined);
    const start = starts.length > 0 ? Math.min(...starts) : undefined;
    const numbered = [];
    for (const v of list) {
      const wide = showWide(v.title);
      const n = start !== undefined && wide !== undefined ? wide - start + 1 : v.season === season ? v.episode : undefined;
      if (n === undefined || n < 1 || limit > 0 && n > limit)
        continue;
      if (!numbered.some(([k]) => k === n))
        numbered.push([n, v.id]);
    }
    numbered.sort((a, b) => a[0] - b[0]);
    if (numbered.length > 0)
      out.set(author, new Map(numbered));
  }
  return out;
}
var qualityOf = (w, h) => w >= 1900 || h >= 1000 ? 1080 : w >= 1260 || h >= 700 ? 720 : w >= 840 || h >= 460 ? 480 : w >= 620 || h >= 340 ? 360 : h;
function variants(master) {
  const lines = master.split(/\r?\n/).map((l) => l.trim());
  const out = [];
  for (let i = 0;i < lines.length; i++) {
    const m = /^#EXT-X-STREAM-INF:.*RESOLUTION=(\d+)x(\d+)/.exec(lines[i]);
    if (!m)
      continue;
    const url = lines.slice(i + 1).find((l) => l && !l.startsWith("#"));
    const quality = qualityOf(Number(m[1]), Number(m[2]));
    if (url?.startsWith("http") && !out.some((v) => v.quality === quality))
      out.push({ url, quality });
  }
  return out.sort((a, b) => b.quality - a.quality);
}
async function api(path, ctx) {
  const res = await ctx.fetch(`${ctx.baseUrl}${path}`, { headers: HEADERS });
  if (!res.ok)
    throw new Error(`rutube: HTTP ${res.status} on ${path}`);
  return await res.json();
}
async function cached(key, ttl, ctx, load) {
  const hit = await ctx.kv.get(key);
  if (hit) {
    try {
      return JSON.parse(hit);
    } catch {}
  }
  const value = await load();
  await ctx.kv.set(key, JSON.stringify(value), ttl);
  return value;
}
var hasCyrillic = (s) => /[а-яё]/i.test(s);
async function search(q, ctx) {
  const s = seasonOf(q.names);
  if (!s)
    return [];
  const queries = [...s.names.filter(hasCyrillic), ...s.names.filter((n) => !hasCyrillic(n))].slice(0, 2);
  for (const query of queries) {
    const page = await api(`/api/search/video/?query=${encodeURIComponent(query)}`, ctx);
    for (const hit of page.results ?? []) {
      const feed = /\/metainfo\/tv\/(\d+)/.exec(hit.feed_url ?? "")?.[1];
      if (!feed || !hit.feed_name || !feedMatches(hit.feed_name, s.names))
        continue;
      const name = hit.feed_name.split("/")[0].trim();
      return [
        {
          id: feed,
          title: s.season > 1 ? `${name} ${s.season}` : name,
          confidence: "exact",
          data: { season: s.season, limit: q.episodes ?? 0 }
        }
      ];
    }
  }
  return [];
}
function feedVideos(feed, ctx) {
  return cached(`videos:${feed}`, VIDEOS_TTL, ctx, async () => {
    const seasons = await api(`/api/metainfo/tv/${feed}/season/`, ctx);
    const out = [];
    for (const { number } of seasons) {
      if (number < 1)
        continue;
      for (let page = 1;page <= MAX_PAGES; page++) {
        const p = await api(`/api/metainfo/tv/${feed}/video?season=${number}&show_all=1&page=${page}`, ctx);
        for (const v of p.results ?? []) {
          if (!v.author?.id || v.season == null || v.episode == null)
            continue;
          out.push({
            id: v.id,
            season: v.season,
            episode: v.episode,
            title: v.title ?? "",
            author: { id: v.author.id, name: v.author.name ?? "Rutube" }
          });
        }
        if (!p.has_next)
          break;
      }
    }
    return out;
  });
}
var settings = (m) => {
  const d = m.data ?? {};
  return { season: d.season ?? 1, limit: d.limit ?? 0 };
};
async function master(videoId, ctx) {
  const options = await api(`/api/play/options/${videoId}/?no_404=true&referer=${encodeURIComponent("https://rutube.ru")}`, ctx);
  const url = options.video_balancer?.m3u8;
  if (!url)
    ctx.fail.notFound("rutube: видео недоступно");
  const res = await ctx.fetch(url);
  if (!res.ok)
    throw new Error(`rutube: HTTP ${res.status} on the playlist`);
  return res.text();
}
function bestQuality(videoId, ctx) {
  return cached(`quality:${videoId}`, QUALITY_TTL, ctx, async () => {
    try {
      return variants(await master(videoId, ctx))[0]?.quality ?? null;
    } catch (e) {
      ctx.log(`rutube: no quality for ${videoId}: ${String(e)}`);
      return null;
    }
  });
}
async function episodes(m, ctx) {
  const { season, limit } = settings(m);
  const videos = await feedVideos(m.id, ctx);
  const map = episodeMap(videos, season, limit);
  const dubs = [];
  const available = new Map;
  for (const [author, eps] of map) {
    const own = videos.filter((v) => String(v.author.id) === author);
    const subtitled = own.filter((v) => /субтитр/i.test(v.title)).length > own.length / 2;
    dubs.push({ id: author, name: own[0].author.name, type: subtitled ? "subtitles" : "voice" });
    const latest = [...eps.values()].pop();
    const quality = await bestQuality(latest, ctx) ?? undefined;
    for (const n of eps.keys()) {
      available.set(n, [...available.get(n) ?? [], quality ? { dubId: author, maxQuality: quality } : { dubId: author }]);
    }
  }
  return {
    dubs,
    episodes: [...available.entries()].sort((a, b) => a[0] - b[0]).map(([number, avail]) => ({ number, available: avail }))
  };
}
async function resolve(m, episode, dubId, ctx) {
  const { season, limit } = settings(m);
  const videoId = episodeMap(await feedVideos(m.id, ctx), season, limit).get(dubId)?.get(episode);
  if (!videoId)
    ctx.fail.notFound(`rutube: серии ${episode} нет`);
  return variants(await master(videoId, ctx)).map((v) => ({ url: v.url, kind: "hls", quality: v.quality }));
}
export {
  episodeMap,
  episodes,
  feedMatches,
  manifest,
  resolve,
  search,
  seasonOf,
  variants
};

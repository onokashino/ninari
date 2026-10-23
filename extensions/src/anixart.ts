// Anixart mobile API (anonymous GETs). Endpoints as used by AniX-org/AniX (MIT): release -> type (dub) -> source -> episode.
import type { Ctx, Dub, EpisodeList, Manifest, Match, SearchQuery, Stream } from "./types";
import { isKodik, KODIK_HOSTS, resolveKodik } from "./players/kodik";
import { isSibnet, resolveSibnet, SIBNET_HOSTS } from "./players/sibnet";

export const manifest: Manifest = {
  id: "anixart",
  name: "Anixart",
  version: "0.1.1",
  apiVersion: 1,
  domains: [
    "https://api-s.anixsekai.com",
    "https://api.anixsekai.com",
    "https://api.anixart.tv",
    "https://api.anixart.app",
    "https://api-s2.anixart.tv",
  ],
  allowedHosts: ["*.libria.fun", ...KODIK_HOSTS, ...SIBNET_HOSTS],
};

/** Write calls check for the app's agent; reads do not, but look like the app anyway. */
const HEADERS = {
  "User-Agent": "AnixartApp/9.0 BETA 5-25062213 (Android 9; SDK 28; arm64-v8a; samsung SM-G975N; en)",
  "API-Version": "v2",
};
const SOURCES_TTL = 3600;
const EPISODES_TTL = 1800;
/** Title search is loose (any word), so only the best candidates are worth keeping. */
const MAX_MATCHES = 10;

type Player = "libria" | "kodik" | "sibnet" | "other";
/** Best first: AniLibria's own HLS goes to 1080p, Kodik stops at 720p, Sibnet is a single mp4. */
const RANK: Record<Player, number> = { libria: 0, kodik: 1, sibnet: 2, other: 3 };
const MAX_QUALITY: Partial<Record<Player, number>> = { libria: 1080, kodik: 720 };

interface ApiRelease {
  id: number;
  title_ru?: string | null;
  title_original?: string | null;
  title_alt?: string | null;
  year?: string | number | null;
  episodes_total?: number | null;
}

interface ApiType {
  id: number;
  name: string;
  is_sub?: boolean;
  episodes_count?: number;
}

interface ApiSource {
  id: number;
  name: string;
  episodes_count?: number;
}

interface ApiEpisode {
  position: number;
  name?: string | null;
  url?: string | null;
}

interface Target {
  number: number;
  url: string;
}

const norm = (s: string) => s.toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");

function playerOf(sourceName: string): Player {
  const n = sourceName.toLowerCase();
  if (n.includes("не работает")) return "other";
  if (n.includes("kodik")) return "kodik";
  if (n.includes("sibnet")) return "sibnet";
  if (n.includes("libria") || n.includes("liberty")) return "libria";
  return "other";
}

async function api<T>(path: string, ctx: Ctx, init?: { method: string; body: string }): Promise<T> {
  const headers: Record<string, string> = init ? { ...HEADERS, "Content-Type": "application/json" } : HEADERS;
  const res = await ctx.fetch(`${ctx.baseUrl}${path}`, { ...init, headers });
  if (res.status === 404) ctx.fail.notFound(`anixart: ${path}`);
  if (!res.ok) throw new Error(`anixart: HTTP ${res.status} on ${path}`);
  const body = (await res.json()) as T & { code?: number };
  if (body.code !== undefined && body.code !== 0) throw new Error(`anixart: code ${body.code} on ${path}`);
  return body;
}

async function cached<T>(key: string, ttl: number, ctx: Ctx, load: () => Promise<T>): Promise<T> {
  const hit = await ctx.kv.get(key);
  if (hit) {
    try {
      return JSON.parse(hit) as T;
    } catch {
      // reload below
    }
  }
  const value = await load();
  await ctx.kv.set(key, JSON.stringify(value), ttl);
  return value;
}

// ---------- search ----------

function titles(r: ApiRelease): string[] {
  const alt = (r.title_alt ?? "").split(/[\n;|]/);
  return [r.title_ru, r.title_original, ...alt].filter((t): t is string => !!t && !!t.trim()).map(norm);
}

function score(r: ApiRelease, q: SearchQuery, wanted: Set<string>): number {
  let s = titles(r).some((t) => wanted.has(t)) ? 4 : 0;
  const year = Number(r.year);
  // Premiere dates near New Year land in different years on Shikimori and Anixart.
  if (q.year && year) s += Math.abs(year - q.year) <= 1 ? 2 : -3;
  if (q.episodes && r.episodes_total) s += r.episodes_total === q.episodes ? 1 : -1;
  return s;
}

/** The API has no Shikimori/MAL ids: an exact title plus a matching year counts as exact. */
export async function search(q: SearchQuery, ctx: Ctx): Promise<Match[]> {
  const wanted = new Set(q.names.map(norm).filter(Boolean));
  const found = new Map<number, { r: ApiRelease; s: number }>();
  for (const name of q.names.slice(0, 3)) {
    const body = await api<{ releases?: ApiRelease[]; content?: ApiRelease[] }>("/search/releases/0", ctx, {
      method: "POST",
      body: JSON.stringify({ query: name, searchBy: 0 }),
    });
    for (const r of body.releases ?? body.content ?? []) {
      if (!found.has(r.id)) found.set(r.id, { r, s: score(r, q, wanted) });
    }
    const best = Math.max(-99, ...[...found.values()].map((f) => f.s));
    if (best >= 6 || (best >= 4 && !q.year)) break;
  }
  return [...found.values()]
    .sort((a, b) => b.s - a.s)
    .slice(0, MAX_MATCHES)
    .map(({ r, s }, i) => ({
      id: String(r.id),
      title: r.title_ru || r.title_original || String(r.id),
      confidence: i === 0 && s >= 4 && (s >= 6 || !q.year) ? "exact" : "fuzzy",
    }));
}

// ---------- episodes ----------

const sourcesOf = (release: string, type: number, ctx: Ctx) =>
  cached(`src:${release}:${type}`, SOURCES_TTL, ctx, async () =>
    ((await api<{ sources?: ApiSource[] }>(`/episode/${release}/${type}`, ctx)).sources ?? []).map((s) => ({
      id: s.id,
      name: s.name,
      episodes_count: s.episodes_count ?? 0,
    })),
  );

const usable = (sources: ApiSource[]) =>
  sources
    .filter((s) => playerOf(s.name) !== "other" && (s.episodes_count ?? 0) > 0)
    .sort((a, b) => RANK[playerOf(a.name)] - RANK[playerOf(b.name)]);

export async function episodes(m: Match, ctx: Ctx): Promise<EpisodeList> {
  const types = ((await api<{ types?: ApiType[] }>(`/episode/${m.id}`, ctx)).types ?? []).filter(
    (t) => (t.episodes_count ?? 0) > 0,
  );
  // One request per dub, all at once: a dub whose only source is dead is dropped, the rest get a quality.
  const perType = await Promise.all(types.map((t) => sourcesOf(m.id, t.id, ctx).catch(() => null)));
  const dubs: Dub[] = [];
  const counts: { dubId: string; count: number; maxQuality?: number }[] = [];
  types.forEach((t, i) => {
    const sources = perType[i];
    const good = sources ? usable(sources) : [];
    if (sources && good.length === 0) return;
    const dubId = String(t.id);
    dubs.push({ id: dubId, name: t.name.trim(), type: t.is_sub ? "subtitles" : "voice" });
    counts.push({
      dubId,
      count: good.length ? Math.max(...good.map((s) => s.episodes_count ?? 0)) : t.episodes_count ?? 0,
      maxQuality: good.length ? MAX_QUALITY[playerOf(good[0].name)] : undefined,
    });
  });
  const total = Math.max(0, ...counts.map((c) => c.count));
  return {
    dubs,
    episodes: Array.from({ length: total }, (_, i) => ({
      number: i + 1,
      available: counts.filter((c) => c.count > i).map(({ dubId, maxQuality }) => ({ dubId, maxQuality })),
    })),
  };
}

// ---------- resolve ----------

/** Sibnet sources count positions from 0, the others from 1; the name ("5 серия") is the most reliable. */
export function numbered(list: ApiEpisode[]): Target[] {
  const offset = list.some((e) => e.position === 0) ? 1 : 0;
  return list
    .filter((e): e is ApiEpisode & { url: string } => !!e.url)
    .map((e) => {
      const named = /^\s*(\d+)\s*серия/i.exec(e.name ?? "")?.[1];
      return { number: named ? Number(named) : e.position + offset, url: e.url };
    });
}

const targetsOf = (release: string, type: string, source: number, ctx: Ctx) =>
  cached(`eps:${release}:${type}:${source}`, EPISODES_TTL, ctx, async () =>
    numbered((await api<{ episodes?: ApiEpisode[] }>(`/episode/${release}/${type}/${source}`, ctx)).episodes ?? []),
  );

/** Parses the Playerjs config of anixart.libria.fun iframes: "[480p]url,[720p]url,[1080p]url" per episode. */
export function parseLibriaIframe(html: string, url: string): Stream[] {
  const raw = /file\s*:\s*(\[[\s\S]*\])\s*\}\s*\)/.exec(html)?.[1];
  if (!raw) return [];
  let files: { id?: string; file?: string }[];
  try {
    files = JSON.parse(raw);
  } catch {
    return [];
  }
  const ep = /[?&]ep=(\d+)/.exec(url)?.[1];
  const entry = (ep && files.find((f) => f.id === `s${ep}`)) || (files.length === 1 ? files[0] : undefined);
  const out: Stream[] = [];
  for (const m of (entry?.file ?? "").matchAll(/\[(\d+)p\]([^,[]+)/g)) {
    out.push({ url: m[2].trim(), kind: "hls", quality: Number(m[1]) });
  }
  return out.sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0));
}

async function resolveLibria(url: string, ctx: Ctx): Promise<Stream[]> {
  const res = await ctx.fetch(url, { headers: { Referer: "https://anixart.tv/" } });
  if (!res.ok) throw new Error(`anixart: libria iframe HTTP ${res.status}`);
  return parseLibriaIframe(await res.text(), url);
}

function play(url: string, ctx: Ctx): Promise<Stream[]> {
  const abs = url.startsWith("//") ? `https:${url}` : url;
  if (/^https?:\/\/[^/]*libria\.fun\//i.test(abs)) return resolveLibria(abs, ctx);
  if (isKodik(abs)) return resolveKodik(abs, ctx);
  if (isSibnet(abs)) return resolveSibnet(abs, ctx);
  if (/\.m3u8(\?|$)/i.test(abs)) return Promise.resolve([{ url: abs, kind: "hls" }]);
  if (/\.mp4(\?|$)/i.test(abs)) return Promise.resolve([{ url: abs, kind: "mp4" }]);
  return Promise.reject(new Error(`anixart: unsupported player ${abs}`));
}

/** Tries the dub's sources best first and returns the first one that plays. */
export async function resolve(m: Match, episode: number, dubId: string, ctx: Ctx): Promise<Stream[]> {
  const sources = usable(await sourcesOf(m.id, Number(dubId), ctx));
  let last: unknown;
  for (const s of sources) {
    const target = (await targetsOf(m.id, dubId, s.id, ctx)).find((t) => t.number === episode);
    if (!target) continue;
    try {
      const streams = await play(target.url, ctx);
      if (streams.length) return streams;
    } catch (e) {
      ctx.log(`anixart: ${s.name} failed: ${e}`);
      last = e;
    }
  }
  if (last) throw last;
  ctx.fail.notFound(`anixart: episode ${episode} of dub ${dubId}`);
}

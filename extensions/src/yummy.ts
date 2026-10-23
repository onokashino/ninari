// YummyAnime API: https://api.yani.tv/swagger. The API is marked "ONLY FOR PERSONAL USE"; the X-Application
// token it documents was not enforced when this was written (2026-09), so requests go without one.
// Each video is a third-party player iframe (Kodik, CVH, Sibnet, Aksor, Alloha); the playable ones are
// resolved through the shared player modules.
import { CVH_HOSTS, cvhDubQualities, cvhKey, isCvh, parseCvh, resolveCvh } from "./players/cvh";
import { isKodik, KODIK_HOSTS, resolveKodik } from "./players/kodik";
import { isSibnet, resolveSibnet, SIBNET_HOSTS } from "./players/sibnet";
import type { Ctx, Dub, EpisodeList, Manifest, Match, SearchQuery, Stream } from "./types";

export const manifest: Manifest = {
  id: "yummy",
  name: "YummyAnime",
  version: "0.1.2",
  apiVersion: 1,
  domains: ["https://api.yani.tv", "https://ru.yummyani.me/api"],
  allowedHosts: [...KODIK_HOSTS, ...CVH_HOSTS, ...SIBNET_HOSTS],
};

const HEADERS = { Accept: "application/json", Lang: "ru" };
const VIDEOS_TTL = 600;

interface Player {
  id: string;
  /** Best picture the player serves, when it is known up front. */
  quality?: number;
  matches(url: string): boolean;
  resolve(url: string, ctx: Ctx): Promise<Stream[]>;
}

// Best first: resolve tries them in this order. Aksor (DASH only) and Alloha are not supported yet.
const PLAYERS: Player[] = [
  { id: "cvh", quality: 1080, matches: isCvh, resolve: resolveCvh },
  { id: "kodik", quality: 720, matches: isKodik, resolve: (url, ctx) => resolveKodik(url, ctx) },
  { id: "sibnet", matches: isSibnet, resolve: resolveSibnet },
];

interface ApiAnime {
  anime_id: number;
  title?: string;
  year?: number;
  type?: { alias?: string };
  remote_ids?: { shikimori_id?: number; myanimelist_id?: number };
}

interface ApiSkip {
  time: number;
  length: number;
}

interface ApiVideo {
  number?: string;
  iframe_url?: string;
  data?: { player?: string; dubbing?: string };
  skips?: { opening?: ApiSkip | null; ending?: ApiSkip | null };
}

/** One playable video, kept small because the whole list is cached between episodes() and resolve(). */
interface Entry {
  n: number;
  dub: string;
  url: string;
  player: string;
  op?: [number, number];
  ed?: [number, number];
}

const normalize = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");

const httpsUrl = (url: string) => url.trim().replace(/^(?:https?:)?\/\//i, "https://");

const span = (s?: ApiSkip | null): [number, number] | undefined =>
  s && s.length > 0 ? [s.time, s.time + s.length] : undefined;

function playerOf(url: string): Player | undefined {
  return PLAYERS.find((p) => {
    try {
      return p.matches(url);
    } catch {
      return false;
    }
  });
}

async function api<T>(path: string, ctx: Ctx): Promise<T> {
  const res = await ctx.fetch(`${ctx.baseUrl}${path}`, { headers: HEADERS });
  if (res.status === 404) ctx.fail.notFound(`yummy: ${path}`);
  if (!res.ok) throw new Error(`yummy: HTTP ${res.status} for ${path}`);
  return ((await res.json()) as { response: T }).response;
}

function toMatch(a: ApiAnime, confidence: Match["confidence"]): Match {
  return { id: String(a.anime_id), title: a.title ?? String(a.anime_id), confidence };
}

/** Fuzzy candidates: titles that do not name a different shikimori entry, the closest first. */
function rankFuzzy(found: ApiAnime[], q: SearchQuery): Match[] {
  const names = new Set(q.names.map(normalize));
  const score = (a: ApiAnime) =>
    (names.has(normalize(a.title ?? "")) ? 4 : 0) + (q.year && a.year === q.year ? 2 : 0) + (q.kind && a.type?.alias === q.kind ? 1 : 0);
  const conflicts = (a: ApiAnime) =>
    !!q.shikimoriId && !!a.remote_ids?.shikimori_id && a.remote_ids.shikimori_id !== q.shikimoriId;
  return found
    .filter((a) => !conflicts(a))
    .sort((a, b) => score(b) - score(a))
    .map((a) => toMatch(a, "fuzzy"));
}

export async function search(q: SearchQuery, ctx: Ctx): Promise<Match[]> {
  const byId: [string, number][] = [["shikimori_ids", q.shikimoriId]];
  if (q.malId && q.malId !== q.shikimoriId) byId.push(["mal_ids", q.malId]);
  for (const [param, id] of byId) {
    if (!id) continue;
    const found = await api<ApiAnime[]>(`/anime?${param}=${id}&limit=5`, ctx);
    // The filter is not trusted blindly: an unknown parameter would return the whole catalog.
    const hit = found.find((a) => a.remote_ids?.shikimori_id === q.shikimoriId || (q.malId && a.remote_ids?.myanimelist_id === q.malId));
    if (hit) return [toMatch(hit, "exact")];
  }
  const found = new Map<number, ApiAnime>();
  for (const name of q.names.slice(0, 3)) {
    for (const a of await api<ApiAnime[]>(`/anime?q=${encodeURIComponent(name)}&limit=10`, ctx)) {
      if (q.shikimoriId && a.remote_ids?.shikimori_id === q.shikimoriId) return [toMatch(a, "exact")];
      if (!found.has(a.anime_id)) found.set(a.anime_id, a);
    }
  }
  return rankFuzzy([...found.values()], q);
}

function toEntry(v: ApiVideo): Entry | undefined {
  const number = (v.number ?? "").trim();
  const dub = (v.data?.dubbing ?? "").trim();
  if (!/^\d+$/.test(number) || !dub || !v.iframe_url) return undefined;
  const url = httpsUrl(v.iframe_url);
  const player = playerOf(url);
  if (!player) return undefined;
  return { n: Number(number), dub, url, player: player.id, op: span(v.skips?.opening), ed: span(v.skips?.ending) };
}

async function videos(id: string, ctx: Ctx): Promise<Entry[]> {
  const key = `videos:${id}`;
  const cached = await ctx.kv.get(key);
  if (cached) {
    try {
      return JSON.parse(cached) as Entry[];
    } catch {
      // refetch below
    }
  }
  const list = await api<ApiVideo[]>(`/anime/${encodeURIComponent(id)}/videos`, ctx);
  const entries = (list ?? []).map(toEntry).filter((e): e is Entry => e !== undefined);
  await ctx.kv.set(key, JSON.stringify(entries), VIDEOS_TTL);
  return entries;
}

/** "Озвучка AniDUB" is a voice-over by AniDUB, "Субтитры Crunchyroll" are subtitles. */
function dubOf(raw: string): Dub {
  const sub = /^Субтитры\s*(.*)$/i.exec(raw);
  // The app keeps subtitles apart from a voice-over by the same team by their type.
  if (sub) return { id: raw, name: sub[1] || "Субтитры", type: "subtitles" };
  const name = raw.replace(/^Озвучка\s+/i, "") || raw;
  return { id: raw, name, type: /^(Оригинал|Original)\b/i.test(name) ? "original" : "voice" };
}

/** The timing most dubs agree on: cuts differ between releases, a lone outlier should not win. */
function commonSpan(spans: ([number, number] | undefined)[]): [number, number] | undefined {
  const counts = new Map<string, number>();
  for (const s of spans) if (s) counts.set(JSON.stringify(s), (counts.get(JSON.stringify(s)) ?? 0) + 1);
  let best: string | undefined;
  for (const [k, c] of counts) if (best === undefined || c > counts.get(best)!) best = k;
  return best === undefined ? undefined : (JSON.parse(best) as [number, number]);
}

/** The quality of an entry: what CVH really has for its dub, else the usual ceiling of its player. */
const qualityOf = (e: Entry, cvh: Map<string, number>) => {
  if (e.player === "cvh") {
    const studio = parseCvh(e.url)?.studio;
    const known = studio ? cvh.get(cvhKey(studio)) : undefined;
    if (known) return known;
  }
  return PLAYERS.find((p) => p.id === e.player)?.quality;
};

const maxDefined = (a?: number, b?: number) => (a === undefined ? b : b === undefined ? a : Math.max(a, b));

export async function episodes(m: Match, ctx: Ctx): Promise<EpisodeList> {
  const entries = await videos(m.id, ctx);
  // CVH lists no quality and 4K uploads exist, so it is asked per dub instead of assuming 1080p.
  const cvhEntry = entries.find((e) => e.player === "cvh");
  const cvhReq = cvhEntry ? parseCvh(cvhEntry.url) : null;
  const cvh = cvhReq?.id ? await cvhDubQualities(cvhReq.id, ctx, cvhReq.pub, cvhReq.aggr) : new Map<string, number>();
  const dubs = new Map<string, Dub>();
  const byEpisode = new Map<number, Entry[]>();
  for (const e of entries) {
    if (!dubs.has(e.dub)) dubs.set(e.dub, dubOf(e.dub));
    const bucket = byEpisode.get(e.n);
    if (bucket) bucket.push(e);
    else byEpisode.set(e.n, [e]);
  }
  const list = [...byEpisode.entries()].sort(([a], [b]) => a - b);
  return {
    dubs: [...dubs.values()],
    episodes: list.map(([number, es]) => {
      const best = new Map<string, number | undefined>();
      for (const e of es) best.set(e.dub, maxDefined(best.get(e.dub), qualityOf(e, cvh)));
      return {
        number,
        available: [...best].map(([dubId, maxQuality]) => ({ dubId, maxQuality })),
        skip: { opening: commonSpan(es.map((e) => e.op)), ending: commonSpan(es.map((e) => e.ed)) },
      };
    }),
  };
}

const rank = (e: Entry) => PLAYERS.findIndex((p) => p.id === e.player);

export async function resolve(m: Match, episode: number, dubId: string, ctx: Ctx): Promise<Stream[]> {
  const entries = (await videos(m.id, ctx))
    .filter((e) => e.n === episode && e.dub === dubId)
    .sort((a, b) => rank(a) - rank(b));
  if (entries.length === 0) ctx.fail.notFound(`episode ${episode} in ${dubId}`);
  const errors: unknown[] = [];
  for (const e of entries) {
    const player = PLAYERS.find((p) => p.id === e.player);
    if (!player) continue;
    try {
      const streams = await player.resolve(e.url, ctx);
      if (streams.length > 0) return streams;
    } catch (err) {
      ctx.log(`yummy: ${player.id} failed for ${e.url}: ${String(err)}`);
      errors.push(err);
    }
  }
  // Prefer a classified error (NotFound, GeoBlocked, Challenge) over "not implemented" and the like.
  throw errors.find((err) => String(err).includes("__EXT:")) ?? errors[errors.length - 1] ?? new Error("yummy: no playable player");
}

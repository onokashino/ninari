// AnimeLib REST API (the backend behind animelib.org). Endpoints as used by vypivshiy/anicli-api (MIT).
// Anonymous clients only get Kodik players; AnimeLib's own player needs a logged-in Bearer token.
import type { Ctx, Dub, EpisodeList, Manifest, Match, SearchQuery, Stream } from "./types";
import { isKodik, KODIK_HOSTS, resolveKodik } from "./players/kodik";

export const manifest: Manifest = {
  id: "animelib",
  name: "AnimeLib",
  version: "0.1.1",
  apiVersion: 1,
  // api.cdnlibs.org is the historical host; DDoS-Guard geo-blocks it outside Russia, api.animelib.org answers everywhere.
  domains: ["https://api.animelib.org", "https://api.cdnlibs.org"],
  allowedHosts: [...KODIK_HOSTS],
};

const HEADERS = { Accept: "application/json", "Site-Id": "5" };
/** The API bans clients that hammer it, so lists and players are cached. */
const LIST_TTL = 1800;
const PLAYERS_TTL = 6 * 3600;

interface ApiAnime {
  id: number;
  name?: string;
  rus_name?: string;
  eng_name?: string;
  shikimori_href?: string | null;
  releaseDate?: string | null;
}

interface ApiEpisode {
  id: number;
  number: string;
  item_number?: number;
}

interface ApiPlayer {
  player: string;
  src?: string | null;
  translation_type?: { id: number; label?: string } | null;
  team?: { id: number; name: string } | null;
}

/** An episode of the list: AnimeLib id and the (integer) number ninari uses. */
interface Ep {
  id: number;
  n: number;
}

/** A player reduced to what we keep: dub key, dub name and type, embed. */
interface Play {
  dub: string;
  name: string;
  type: Dub["type"];
  player: string;
  src: string;
}

class Retry extends Error {}

async function fetchOn(base: string, path: string, ctx: Ctx): Promise<unknown> {
  let res;
  try {
    res = await ctx.fetch(`${base}/api${path}`, { headers: HEADERS });
  } catch (e) {
    if (/__EXT:(Challenge|Blocked)/.test(String(e))) throw new Retry(String(e));
    throw e;
  }
  if (res.status === 404) ctx.fail.notFound(`animelib: ${path}`);
  if (res.status === 403 || res.status >= 500) throw new Retry(`animelib: HTTP ${res.status} on ${base}`);
  if (!res.ok) throw new Error(`animelib: HTTP ${res.status} on ${path}`);
  return res.json();
}

/** ctx.baseUrl first, then the other API hosts: a geo-blocked host answers with a DDoS-Guard page, not an error the core retries. */
async function api<T>(path: string, ctx: Ctx): Promise<T> {
  const bases = [ctx.baseUrl, ...manifest.domains.filter((d) => d !== ctx.baseUrl)];
  let last: unknown;
  for (const base of bases) {
    try {
      return (await fetchOn(base, path, ctx)) as T;
    } catch (e) {
      if (!(e instanceof Retry)) throw e;
      last = e;
    }
  }
  throw last;
}

async function cached<T>(key: string, ttl: number, ctx: Ctx, load: () => Promise<T>, fresh = false): Promise<T> {
  const hit = fresh ? null : await ctx.kv.get(key);
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

const shikiId = (href?: string | null) => Number(/\/animes\/[a-z]*(\d+)/.exec(href ?? "")?.[1] ?? NaN);

const norm = (s: string) => s.toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");

export async function search(q: SearchQuery, ctx: Ctx): Promise<Match[]> {
  const wanted = new Set(q.names.map(norm).filter(Boolean));
  const fuzzy = new Map<number, Match & { named: boolean }>();
  for (const name of q.names.slice(0, 3)) {
    const found = (await api<{ data?: ApiAnime[] }>(`/anime?q=${encodeURIComponent(name)}`, ctx)).data ?? [];
    for (const a of found) {
      const title = a.rus_name || a.name || a.eng_name || String(a.id);
      if (shikiId(a.shikimori_href) === q.shikimoriId) return [{ id: String(a.id), title, confidence: "exact" }];
      const named = [a.rus_name, a.name, a.eng_name].some((t) => !!t && wanted.has(norm(t)));
      if (!fuzzy.has(a.id)) fuzzy.set(a.id, { id: String(a.id), title, confidence: "fuzzy", named });
    }
  }
  return [...fuzzy.values()].sort((a, b) => Number(b.named) - Number(a.named)).map(({ named: _, ...m }) => m);
}

// ---------- episodes ----------

/** Numbers restart per season in some entries; then the running item_number is the episode number. */
export function numbered(list: ApiEpisode[]): Ep[] {
  const byNumber = list.map((e) => ({ id: e.id, n: Number(e.number) }));
  const unique = new Set(byNumber.map((e) => e.n)).size === byNumber.length;
  const eps = unique ? byNumber : list.map((e, i) => ({ id: e.id, n: e.item_number ?? i + 1 }));
  return eps.filter((e) => Number.isInteger(e.n));
}

const episodeList = (anime: string, ctx: Ctx) =>
  cached(`eps:${anime}`, LIST_TTL, ctx, async () =>
    numbered((await api<{ data?: ApiEpisode[] }>(`/episodes?anime_id=${encodeURIComponent(anime)}`, ctx)).data ?? []),
  );

function dubType(p: ApiPlayer): Dub["type"] {
  const label = (p.translation_type?.label ?? "").toLowerCase();
  if (p.translation_type?.id === 1 || label.includes("субтитр")) return "subtitles";
  if (label.includes("оригинал")) return "original";
  return "voice";
}

export function reducePlayers(players: ApiPlayer[]): Play[] {
  return players
    .filter((p) => p.team && p.src)
    .map((p) => {
      const type = dubType(p);
      return { dub: `${p.team!.id}-${type[0]}`, name: p.team!.name.trim(), type, player: p.player, src: p.src! };
    });
}

const playersOf = (episodeId: number, ctx: Ctx, fresh = false) =>
  cached(
    `pl:${episodeId}`,
    PLAYERS_TTL,
    ctx,
    async () => reducePlayers((await api<{ data?: { players?: ApiPlayer[] } }>(`/episodes/${episodeId}`, ctx)).data?.players ?? []),
    fresh,
  );

async function knownPlayers(episodeId: number, ctx: Ctx): Promise<Play[] | null> {
  const hit = await ctx.kv.get(`pl:${episodeId}`);
  if (!hit) return null;
  try {
    return JSON.parse(hit) as Play[];
  } catch {
    return null;
  }
}

const maxQuality = (p: Play) => (isKodik(p.src) ? Number(/\/(\d{3,4})p$/.exec(p.src)?.[1] ?? 720) : undefined);

/**
 * Players are per episode, one request each. Only the first and the last episode are fetched (plus any already
 * cached by resolve); a team seen in episode k is assumed to have every episode up to k, as teams dub in order.
 */
export function availability(eps: Ep[], known: (Play[] | null)[]): EpisodeList {
  const dubs = new Map<string, Dub>();
  const last = new Map<string, number>();
  const quality = new Map<string, number | undefined>();
  known.forEach((plays, i) => {
    for (const p of plays ?? []) {
      if (!dubs.has(p.dub)) dubs.set(p.dub, { id: p.dub, name: p.name, type: p.type });
      last.set(p.dub, Math.max(i, last.get(p.dub) ?? -1));
      quality.set(p.dub, Math.max(quality.get(p.dub) ?? 0, maxQuality(p) ?? 0) || undefined);
    }
  });
  return {
    dubs: [...dubs.values()],
    episodes: eps.map((e, i) => {
      const exact = known[i];
      const ids = exact ? [...new Set(exact.map((p) => p.dub))] : [...last].filter(([, k]) => k >= i).map(([d]) => d);
      return { number: e.n, available: ids.map((dubId) => ({ dubId, maxQuality: quality.get(dubId) })) };
    }),
  };
}

export async function episodes(m: Match, ctx: Ctx): Promise<EpisodeList> {
  const eps = await episodeList(m.id, ctx);
  if (eps.length === 0) return { dubs: [], episodes: [] };
  const sampled = new Set([0, eps.length - 1]);
  const known = await Promise.all(
    eps.map((e, i) => (sampled.has(i) ? playersOf(e.id, ctx) : knownPlayers(e.id, ctx))),
  );
  return availability(eps, known);
}

// ---------- resolve ----------

async function play(p: Play, ctx: Ctx): Promise<Stream[]> {
  if (isKodik(p.src)) return resolveKodik(p.src, ctx);
  throw new Error(`animelib: unsupported player ${p.player}`);
}

export async function resolve(m: Match, episode: number, dubId: string, ctx: Ctx): Promise<Stream[]> {
  const ep = (await episodeList(m.id, ctx)).find((e) => e.n === episode);
  if (!ep) ctx.fail.notFound(`animelib: episode ${episode}`);
  let plays = (await playersOf(ep.id, ctx)).filter((p) => p.dub === dubId);
  // A cached list may predate the team's upload.
  if (plays.length === 0) plays = (await playersOf(ep.id, ctx, true)).filter((p) => p.dub === dubId);
  if (plays.length === 0) ctx.fail.notFound(`animelib: episode ${episode} has no dub ${dubId}`);
  let last: unknown;
  for (const p of plays) {
    try {
      const streams = await play(p, ctx);
      if (streams.length) return streams;
    } catch (e) {
      ctx.log(`animelib: ${p.player} failed: ${e}`);
      last = e;
    }
  }
  throw last ?? new Error(`animelib: no playable player for dub ${dubId}`);
}

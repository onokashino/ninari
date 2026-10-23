// Shikicinema (smarthard.net): the community index of player links per Shikimori id that the Shikicinema
// browser extension (github.com/Smarthard/shikicinema, BSD-2-Clause) shows on Shikimori. Public read API,
// no key: GET /api/shikivideos/<id>?limit=all or ?episode=<n>, and /api/shikivideos/<id>/length
// (swagger at /v1/api/docs). Most links point at hosts that are gone; only the ones a player module
// here can play are listed.
import { DZEN_HOSTS, isDzen, resolveDzen } from "./players/dzen";
import { isKinogram, KINOGRAM_HOSTS, kinogramKey, kinogramTracks, parseKinogramUrl, resolveKinogram } from "./players/kinogram";
import { isKodik, KODIK_HOSTS, resolveKodik } from "./players/kodik";
import { isMatreshka, MATRESHKA_HOSTS, resolveMatreshka } from "./players/matreshka";
import { isOkru, OKRU_HOSTS, resolveOkru } from "./players/okru";
import { isSibnet, resolveSibnet, SIBNET_HOSTS } from "./players/sibnet";
import { isVk, resolveVk, VK_HOSTS } from "./players/vk";
import type { Ctx, Dub, EpisodeList, Manifest, Match, SearchQuery, Stream } from "./types";

export const manifest: Manifest = {
  id: "shikicinema",
  name: "Shikicinema",
  version: "0.1.0",
  apiVersion: 1,
  domains: ["https://smarthard.net"],
  allowedHosts: [
    ...new Set([...SIBNET_HOSTS, ...OKRU_HOSTS, ...DZEN_HOSTS, ...VK_HOSTS, ...MATRESHKA_HOSTS, ...KINOGRAM_HOSTS, ...KODIK_HOSTS]),
  ],
};

const API = "/api/shikivideos";
const HEADERS = { Accept: "application/json" };
/** Up to this many episodes the whole list is one request (Frieren: 605 links, 145 KB). */
const FULL_LIST_MAX = 64;
/** Longer series (One Piece: 53k links, 12.7 MB) are sampled episode by episode, this many per call. */
const PROBES_PER_CALL = 16;
const PROBE_MAX_AGE = 7 * 24 * 3600;
/** New uploads land on the latest episodes first, so those are asked again after this long. */
const TAIL_MAX_AGE = 3600;
const LENGTH_TTL = 600;
/** A link a player reported as removed or blocked is not offered again for this long. */
const DEAD_TTL = 7 * 24 * 3600;
/** resolve() starts no new attempt after this long, so a few slow hosts cannot use up the 15 s call. */
const RESOLVE_BUDGET_MS = 7000;

interface Player {
  id: string;
  matches(url: string): boolean;
  resolve(url: string, ctx: Ctx): Promise<Stream[]>;
}

/**
 * Best first, as measured in 2026-09: Dzen and Matreshka carry recent uploads up to 1080p, Kinogram
 * 720p with several dubs in one stream, Kodik 720p, Sibnet a single MP4 that rarely disappears. Most
 * VK and ok.ru uploads of anime are blocked on a rights holder's request, so they go last.
 */
const PLAYERS: Player[] = [
  { id: "dzen", matches: isDzen, resolve: resolveDzen },
  { id: "matreshka", matches: isMatreshka, resolve: resolveMatreshka },
  { id: "kinogram", matches: isKinogram, resolve: resolveKinogram },
  { id: "kodik", matches: isKodik, resolve: (url, ctx) => resolveKodik(url, ctx) },
  { id: "sibnet", matches: isSibnet, resolve: resolveSibnet },
  { id: "vk", matches: isVk, resolve: resolveVk },
  { id: "okru", matches: isOkru, resolve: resolveOkru },
];

interface ApiVideo {
  id: number;
  url?: string | null;
  episode?: number | null;
  kind?: string | null;
  language?: string | null;
  author?: string | null;
  watches_count?: number | null;
}

/** One playable link. */
interface Entry {
  id: number;
  n: number;
  dub: string;
  url: string;
  player: string;
  watches: number;
}

interface DubInfo {
  name: string;
  type: Dub["type"];
}

/** Sampled episodes of a long series: which dubs each one has, and when that was asked. */
interface Probes {
  dubs: Record<string, DubInfo>;
  eps: Record<string, [number, string[]]>;
}

const KINDS: Record<string, Dub["type"]> = { "озвучка": "voice", "субтитры": "subtitles", "оригинал": "original" };
const TYPE_CODE: Record<Dub["type"], string> = { voice: "v", subtitles: "s", original: "o" };

/**
 * Descriptions put where the team name belongs ("Русская Озвучка | Оригинал", Kinogram's "Рус. Проф.
 * многоголосый", "Часть 1", an episode range); they name no one. Matched against the normalized name.
 */
const GENERIC_RE =
  /^(?:\d+|часть\d*|(?:рус|русск(?:ий|ая|ие)|укр|англ|eng|проф|профессиональн(?:ый|ая)|любительск(?:ий|ая)|многоголосый|одноголосый|двухголосый|закадровый|озвучка|субтитры|оригинал|original|sub|dub)+)$/;

const norm = (s: string) => s.toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");

const nowSec = () => Math.floor(Date.now() / 1000);

/**
 * The team behind a link: "AniMedia (Shachiburi)" is AniMedia with Shachiburi's voice, "DreamCast (Airis,
 * Belka, ...)" is Dream Cast, "AniMedia (Shachiburi) (277-278)" is AniMedia too; what is in parentheses at
 * the end is dropped so one team is one dub.
 */
export function teamOf(author: string | null | undefined): string | undefined {
  let a = (author ?? "").replace(/\s+/g, " ").trim();
  for (;;) {
    const cast = /^(.*?\S)\s*\([^()]*\)$/.exec(a);
    // "(036-107) Фриза" keeps its parentheses: they are not at the end. "I.D.A. (...)" is still a team.
    if (!cast || (cast[1].match(/[\p{L}\p{N}]/gu) ?? []).length < 2) break;
    a = cast[1].trim();
  }
  return a && !GENERIC_RE.test(norm(a)) ? a : undefined;
}

/**
 * The dub a link belongs to. Links without a team: subtitles become "Субтитры" (as Animego and YummyAnime
 * call them), the original audio is "Оригинал", and an unnamed voice-over is dropped, since one "Озвучка"
 * would mix unrelated teams and the app could not tell them apart.
 */
export function dubOf(kind: string | null | undefined, author: string | null | undefined): (DubInfo & { id: string }) | undefined {
  const type = KINDS[(kind ?? "").trim().toLowerCase()];
  if (!type) return undefined;
  const team = type === "original" ? "Оригинал" : (teamOf(author) ?? (type === "subtitles" ? "Субтитры" : undefined));
  if (!team) return undefined;
  return { id: `${TYPE_CODE[type]}|${norm(team)}`, name: team, type };
}

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
  if (!res.ok) throw new Error(`shikicinema: HTTP ${res.status} for ${path}`);
  return (await res.json()) as T;
}

function animeId(m: Match): number {
  const id = Number(m.id);
  if (!Number.isInteger(id) || id <= 0) throw new Error(`shikicinema: bad match id ${m.id}`);
  return id;
}

/** The highest episode number any link has; 0 when Shikicinema has nothing for the title. */
async function episodeCount(id: number, ctx: Ctx): Promise<number> {
  const key = `length:${id}`;
  const cached = await ctx.kv.get(key);
  if (cached !== null) return Number(cached) || 0;
  const { length } = await api<{ length?: number }>(`${API}/${id}/length`, ctx);
  const n = Number(length) || 0;
  await ctx.kv.set(key, String(n), LENGTH_TTL);
  return n;
}

/** Links a player reported as gone, by link id, with the time they may be tried again. */
async function loadDead(id: number, ctx: Ctx): Promise<Record<string, number>> {
  const raw = await ctx.kv.get(`dead:${id}`);
  if (!raw) return {};
  try {
    const all = JSON.parse(raw) as Record<string, number>;
    const now = nowSec();
    return Object.fromEntries(Object.entries(all).filter(([, until]) => until > now));
  } catch {
    return {};
  }
}

/**
 * Turns API rows into playable entries: known player, known dub, not reported gone. Kinogram links carry
 * a generic author ("Русская Озвучка | Оригинал") and several audio tracks; they are filed under the track
 * the player starts with, read from the Kinogram page (one request per title, cached).
 */
async function toEntries(
  videos: ApiVideo[],
  dead: Record<string, number>,
  ctx: Ctx,
  dubs: Record<string, DubInfo>,
  tracks = new Map<string, Promise<Record<string, string[]>>>(),
): Promise<Entry[]> {
  const out: Entry[] = [];
  for (const v of videos) {
    const url = (v.url ?? "").trim();
    const n = Number(v.episode);
    if (!url || !Number.isInteger(n) || n < 1 || dead[String(v.id)]) continue;
    // English links would merge with the Russian subtitles of the same team.
    if (/^(english|en)$/i.test(v.language ?? "")) continue;
    const player = playerOf(url);
    if (!player) continue;
    let dub = dubOf(v.kind, v.author);
    if (player.id === "kinogram") {
      const ref = parseKinogramUrl(url);
      if (!ref) continue;
      if (!tracks.has(ref.page)) tracks.set(ref.page, kinogramTracks(ref.page, ctx).catch((): Record<string, string[]> => ({})));
      const first = (await tracks.get(ref.page)!)[kinogramKey(ref)]?.[0];
      if (!first) continue;
      dub = /original|оригинал/i.test(first) ? dubOf("оригинал", null) : dubOf("озвучка", first);
    }
    if (!dub) continue;
    dubs[dub.id] ??= { name: dub.name, type: dub.type };
    out.push({ id: v.id, n, dub: dub.id, url, player: player.id, watches: v.watches_count ?? 0 });
  }
  return out;
}

// ---------------------------------------------------------------- search

/** Shikicinema is keyed by the Shikimori id, so a title either is there or is not. */
export async function search(q: SearchQuery, ctx: Ctx): Promise<Match[]> {
  if (!q.shikimoriId) return [];
  if ((await episodeCount(q.shikimoriId, ctx)) === 0) return [];
  return [{ id: String(q.shikimoriId), title: q.names[0] ?? `Shikimori ${q.shikimoriId}`, confidence: "exact" }];
}

// ---------------------------------------------------------------- episodes

function toList(dubs: Record<string, DubInfo>, byEpisode: Map<number, Set<string>>): EpisodeList {
  const used = new Set<string>();
  const episodes = [...byEpisode.entries()]
    .filter(([, ids]) => ids.size > 0)
    .sort(([a], [b]) => a - b)
    .map(([number, ids]) => {
      for (const d of ids) used.add(d);
      // Quality is not in the index (its tv/web/bd field is the rip source), players report it on resolve.
      return { number, available: [...ids].map((dubId) => ({ dubId })) };
    });
  return { dubs: [...used].map((id) => ({ id, ...dubs[id] })), episodes };
}

async function listAll(id: number, ctx: Ctx): Promise<EpisodeList> {
  const dubs: Record<string, DubInfo> = {};
  const [videos, dead] = await Promise.all([api<ApiVideo[]>(`${API}/${id}?limit=all`, ctx), loadDead(id, ctx)]);
  const byEpisode = new Map<number, Set<string>>();
  for (const e of await toEntries(videos ?? [], dead, ctx, dubs)) {
    const set = byEpisode.get(e.n) ?? new Set<string>();
    set.add(e.dub);
    byEpisode.set(e.n, set);
  }
  return toList(dubs, byEpisode);
}

async function loadProbes(id: number, ctx: Ctx): Promise<Probes> {
  const raw = await ctx.kv.get(`probes:${id}`);
  const probes: Probes = { dubs: {}, eps: {} };
  if (!raw) return probes;
  try {
    const cached = JSON.parse(raw) as Probes;
    const oldest = nowSec() - PROBE_MAX_AGE;
    probes.dubs = cached.dubs ?? {};
    for (const [n, p] of Object.entries(cached.eps ?? {})) if (p[0] >= oldest) probes.eps[n] = p;
  } catch {
    // start over
  }
  return probes;
}

const saveProbes = (id: number, p: Probes, ctx: Ctx) => ctx.kv.set(`probes:${id}`, JSON.stringify(p), PROBE_MAX_AGE);

function record(p: Probes, n: number, entries: Entry[], dubs: Record<string, DubInfo>): void {
  const ids = [...new Set(entries.filter((e) => e.n === n).map((e) => e.dub))];
  for (const d of ids) p.dubs[d] ??= dubs[d];
  p.eps[String(n)] = [nowSec(), ids];
}

/** A stretch whose two ends list the same dubs probably has nothing new inside: it counts at this share of its width. */
const SAME_ENDS_WEIGHT = 0.25;

/**
 * Which episodes to ask next: the two after the one last played (so the next episode is known exactly),
 * the last two if they are stale, the first one, then the middle of the widest stretch not asked yet,
 * preferring stretches where a dub starts or stops. All picks go out at once, so a picked middle splits
 * its stretch in two for the next pick.
 */
export function nextProbes(eps: Probes["eps"], length: number, count: number, near?: number, staleTail: number[] = []): number[] {
  const known = new Set(Object.keys(eps).map(Number));
  const picks: number[] = [];
  const take = (n: number) => {
    if (picks.length < count && n >= 1 && n <= length && !known.has(n)) {
      known.add(n);
      picks.push(n);
    }
  };
  if (near) [near + 1, near + 2].forEach(take);
  for (const n of staleTail) {
    if (picks.length < count && n >= 1 && n <= length && !picks.includes(n)) {
      known.add(n);
      picks.push(n);
    }
  }
  take(1);
  take(length);
  const sameEnds = (a: number, b: number) => {
    const x = eps[String(a)]?.[1];
    const y = eps[String(b)]?.[1];
    return !!x && !!y && x.length === y.length && x.every((d) => y.includes(d));
  };
  const points = [...known].sort((a, b) => a - b);
  const gaps: { a: number; b: number; w: number }[] = [];
  for (let i = 1; i < points.length; i++) {
    const [a, b] = [points[i - 1], points[i]];
    if (b - a > 1) gaps.push({ a, b, w: sameEnds(a, b) ? SAME_ENDS_WEIGHT : 1 });
  }
  while (picks.length < count && gaps.length > 0) {
    let best = 0;
    for (let i = 1; i < gaps.length; i++) if ((gaps[i].b - gaps[i].a) * gaps[i].w > (gaps[best].b - gaps[best].a) * gaps[best].w) best = i;
    const { a, b, w } = gaps.splice(best, 1)[0];
    const mid = Math.floor((a + b) / 2);
    take(mid);
    if (mid - a > 1) gaps.push({ a, b: mid, w });
    if (b - mid > 1) gaps.push({ a: mid, b, w });
  }
  return picks;
}

/**
 * Which dubs each episode has, from the sampled ones: a dub found on two neighbouring samples is taken
 * to cover the episodes between them. Teams upload runs of episodes, so this is right about 95% of the
 * time; resolve() asks the exact episode, and every call samples more.
 */
export function coverage(eps: Probes["eps"], length: number): Map<number, Set<string>> {
  const points = Object.keys(eps).map(Number).filter((n) => n <= length).sort((a, b) => a - b);
  const out = new Map<number, Set<string>>();
  const add = (n: number, d: string) => {
    const set = out.get(n) ?? new Set<string>();
    set.add(d);
    out.set(n, set);
  };
  points.forEach((n, i) => {
    const here = eps[String(n)][1];
    for (const d of here) add(n, d);
    if (i === 0) return;
    const prev = points[i - 1];
    const shared = eps[String(prev)][1].filter((d) => here.includes(d));
    for (let k = prev + 1; k < n; k++) for (const d of shared) add(k, d);
  });
  return out;
}

async function listSampled(id: number, length: number, ctx: Ctx): Promise<EpisodeList> {
  const [probes, dead, nearRaw] = await Promise.all([loadProbes(id, ctx), loadDead(id, ctx), ctx.kv.get(`near:${id}`)]);
  const staleBefore = nowSec() - TAIL_MAX_AGE;
  const staleTail = [length, length - 1].filter((n) => n >= 1 && (probes.eps[String(n)]?.[0] ?? 0) < staleBefore);
  const picks = nextProbes(probes.eps, length, PROBES_PER_CALL, Number(nearRaw) || undefined, staleTail);
  const dubs: Record<string, DubInfo> = { ...probes.dubs };
  const tracks = new Map<string, Promise<Record<string, string[]>>>();
  await Promise.all(
    picks.map(async (n) => {
      try {
        const videos = await api<ApiVideo[]>(`${API}/${id}?episode=${n}`, ctx);
        record(probes, n, await toEntries(videos ?? [], dead, ctx, dubs, tracks), dubs);
      } catch (e) {
        ctx.log(`shikicinema: episode ${n} of ${id} not sampled: ${String(e)}`);
      }
    }),
  );
  await saveProbes(id, probes, ctx);
  return toList(probes.dubs, coverage(probes.eps, length));
}

export async function episodes(m: Match, ctx: Ctx): Promise<EpisodeList> {
  const id = animeId(m);
  const length = await episodeCount(id, ctx);
  if (length === 0) return { dubs: [], episodes: [] };
  return length <= FULL_LIST_MAX ? listAll(id, ctx) : listSampled(id, length, ctx);
}

// ---------------------------------------------------------------- resolve

const rank = (e: Entry) => PLAYERS.findIndex((p) => p.id === e.player);

/** Best host first, then the most watched link, then the newest upload. */
const byPreference = (a: Entry, b: Entry) => rank(a) - rank(b) || b.watches - a.watches || b.id - a.id;

export async function resolve(m: Match, episode: number, dubId: string, ctx: Ctx): Promise<Stream[]> {
  const started = Date.now();
  const id = animeId(m);
  const dead = await loadDead(id, ctx);
  const dubs: Record<string, DubInfo> = {};
  const videos = await api<ApiVideo[]>(`${API}/${id}?episode=${episode}`, ctx);
  const entries = await toEntries(videos ?? [], dead, ctx, dubs);
  // The same link is often uploaded twice.
  const tries = entries
    .filter((e) => e.n === episode && e.dub === dubId)
    .sort(byPreference)
    .filter((e, i, all) => all.findIndex((x) => x.url === e.url) === i);
  const failures: string[] = [];
  let challenge: unknown;
  let streams: Stream[] | undefined;
  for (const [i, e] of tries.entries()) {
    if (Date.now() - started > RESOLVE_BUDGET_MS) {
      failures.push(`${tries.length - i} more not tried in time`);
      break;
    }
    const player = PLAYERS.find((p) => p.id === e.player)!;
    try {
      const out = await player.resolve(e.url, ctx);
      if (out.length > 0) {
        streams = out;
        break;
      }
      failures.push(`${player.id}: no streams`);
    } catch (err) {
      const msg = String(err instanceof Error ? err.message : err);
      ctx.log(`shikicinema: ${player.id} failed for ${e.url}: ${msg}`);
      // The core reads the first "__EXT:<Kind>:" marker anywhere in a message, so the summary drops them.
      failures.push(`${player.id}: ${msg.replace(/__EXT:(\w+):/g, "$1: ")}`);
      if (msg.includes("__EXT:NotFound:")) dead[String(e.id)] = nowSec() + DEAD_TTL;
      if (msg.includes("__EXT:Challenge:")) challenge ??= err;
    }
  }
  await ctx.kv.set(`dead:${id}`, JSON.stringify(dead), DEAD_TTL);
  await ctx.kv.set(`near:${id}`, String(episode), PROBE_MAX_AGE);
  // A long series keeps samples: this episode is now known exactly, dead links left out.
  const probes = await ctx.kv.get(`probes:${id}`);
  if (probes) {
    const p = await loadProbes(id, ctx);
    record(p, episode, entries.filter((e) => !dead[String(e.id)]), dubs);
    await saveProbes(id, p, ctx);
  }
  if (streams) return streams;
  if (challenge) throw challenge;
  return ctx.fail.notFound(tries.length === 0 ? `Shikicinema has no episode ${episode} in this dub` : failures.join("; "));
}

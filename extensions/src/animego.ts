// AnimeGO (animego.me): a catalog whose episodes play in Aniboom (up to 1080p), CVH (up to 1080p) and Kodik (720p).
// Site endpoints as in YaNesyTortiK/AnimeParsers parser_animego.py and ialakey/anime-dl-core (both MIT), re-checked live.
import { ANIBOOM_HOSTS, resolveAniboom } from "./players/aniboom";
import { CVH_HOSTS, cvhKey, cvhPlaylist, cvhPlaylistUrl, cvhStudio, parseCvh, resolveCvh, type CvhItem } from "./players/cvh";
import { KODIK_HOSTS, resolveKodik } from "./players/kodik";
import type { Ctx, Dub, EpisodeList, Manifest, Match, SearchQuery, Stream } from "./types";

export const manifest: Manifest = {
  id: "animego",
  name: "AnimeGO",
  version: "0.1.2",
  apiVersion: 1,
  // The old domains redirect to the current one; they stay as a way back if the main domain moves again.
  domains: ["https://animego.me", "https://animego.org", "https://animego.one", "https://animego.club"],
  allowedHosts: [...ANIBOOM_HOSTS, ...CVH_HOSTS, ...KODIK_HOSTS],
};

/** Players by preference: Aniboom serves 1080p HLS, CVH up to 1080p, Kodik 720p. */
const PLAYER_ORDER = ["aniboom", "cvh", "kodik"];
const HD = 1080;
const SD = 720;
/** Probing per-episode dub lists stops after this long so search + episodes fit the core's 8 s budget. */
const PROBE_BUDGET_MS = 2500;
const MAX_PROBES = 6;
/** Seconds a cached player page is reused by episodes() (right after search) and kept for resolve(). */
const PLAYER_REUSE = 60;
const PLAYER_TTL = 600;
const PROBES_TTL = 3 * 3600;

/** One player button of an episode: a dub (translation) in one player. */
interface Provider {
  tr: string;
  name: string;
  slug: string;
  url: string;
}

/** The /player/<id> page: episode ids of its first page, all episode numbers and the default episode's players. */
interface Player {
  at: number;
  ids: Record<number, string>;
  numbers: number[];
  current: number;
  providers: Provider[];
  names: Record<string, string>;
}

/** Per episode number: dub id -> players that carry it. Cached between calls to refine availability. */
type Probes = Record<number, Record<string, { name: string; slugs: string[]; cvh?: string }>>;

interface MatchData {
  cvh?: string;
}

interface Hit {
  id: string;
  title: string;
  names: string[];
  year?: number;
  score: number;
}

const absolute = (url: string): string => {
  const u = url.trim().replace(/&amp;/g, "&");
  return u.startsWith("//") ? `https:${u}` : u;
};

const norm = (s: string): string => s.toLowerCase().replace(/ё/g, "е").replace(/[^\p{L}\p{N}]+/gu, "");

function bigrams(s: string): Map<string, number> {
  const out = new Map<string, number>();
  for (let i = 0; i < s.length - 1; i++) out.set(s.slice(i, i + 2), (out.get(s.slice(i, i + 2)) ?? 0) + 1);
  return out;
}

/** Dice coefficient over character bigrams of the normalized names, 1 for an exact match. */
function similarity(a: string, b: string): number {
  const x = norm(a);
  const y = norm(b);
  if (!x || !y) return 0;
  if (x === y) return 1;
  const bx = bigrams(x);
  const by = bigrams(y);
  let common = 0;
  for (const [g, n] of bx) common += Math.min(n, by.get(g) ?? 0);
  return (2 * common) / (x.length - 1 + y.length - 1 || 1);
}

async function xhr(path: string, ctx: Ctx): Promise<string> {
  const res = await ctx.fetch(`${ctx.baseUrl}${path}`, {
    headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json", Referer: `${ctx.baseUrl}/` },
  });
  if (res.status === 404) ctx.fail.notFound(`animego ${path}`);
  if (!res.ok) throw new Error(`animego: HTTP ${res.status} for ${path}`);
  const body = (await res.json()) as { status?: string; data?: { content?: string | null } };
  if (body.status !== "success") throw new Error(`animego: ${path} answered ${body.status}`);
  return body.data?.content ?? "";
}

function parseProviders(html: string, ctx: Ctx): Provider[] {
  return ctx.html
    .select(html, '[data-anime-player-target="provider"]')
    .map((n) => ({
      tr: n.attrs["data-translation-id"] ?? "",
      name: (n.attrs["data-translation-title"] ?? "").trim(),
      slug: n.attrs["data-provider-slug"] ?? "",
      url: absolute(n.attrs["data-player"] ?? ""),
    }))
    .filter((p) => p.tr && p.url);
}

function parseEpisodes(html: string, ctx: Ctx): { number: number; id: string; active: boolean }[] {
  return ctx.html
    .select(html, "[data-episode-number][data-episode]")
    .map((n) => ({
      number: Number(n.attrs["data-episode-number"]),
      id: n.attrs["data-episode"] ?? "",
      active: /\bactive\b/.test(n.attrs.class ?? ""),
    }))
    .filter((e) => Number.isInteger(e.number) && e.id !== "");
}

/** Episode numbers beyond the first page come from the pager metadata. */
function pagedNumbers(html: string, ctx: Ctx): number[] {
  const raw = ctx.html.select(html, "[data-anime-player-episodes-pages-value]")[0]?.attrs["data-anime-player-episodes-pages-value"];
  if (!raw) return [];
  try {
    const pages = JSON.parse(raw) as { firstNumber: number; lastNumber: number; count: number }[];
    const out: number[] = [];
    for (const p of pages) {
      if (p.count !== p.lastNumber - p.firstNumber + 1) continue;
      for (let n = p.firstNumber; n <= p.lastNumber; n++) out.push(n);
    }
    return out;
  } catch {
    return [];
  }
}

async function fetchPlayer(id: string, ctx: Ctx): Promise<Player> {
  const html = await xhr(`/player/${encodeURIComponent(id)}`, ctx);
  const eps = parseEpisodes(html, ctx);
  const providers = parseProviders(html, ctx);
  const names: Record<string, string> = {};
  for (const n of ctx.html.select(html, '[data-anime-player-target="translation"]')) {
    const tr = n.attrs["data-translation-id"];
    const name = n.text.replace("(ошибка)", "").replace(/\s+/g, " ").trim();
    if (tr && name) names[tr] = name;
  }
  for (const p of providers) if (p.name) names[p.tr] = p.name;
  const numbers = [...new Set([...eps.map((e) => e.number), ...pagedNumbers(html, ctx)])].sort((a, b) => a - b);
  // Movies have no episode bar: their only "episode" is 1.
  if (numbers.length === 0 && providers.length > 0) numbers.push(1);
  return {
    at: Date.now(),
    ids: Object.fromEntries(eps.map((e) => [e.number, e.id])),
    numbers,
    current: eps.find((e) => e.active)?.number ?? numbers[0] ?? 1,
    providers,
    names,
  };
}

/**
 * search() loads the page to verify a match and episodes() right after reuses it; resolve() only needs
 * episode ids and the CVH id, which do not change, so it accepts an older copy.
 */
async function player(id: string, ctx: Ctx, maxAgeSec: number): Promise<Player> {
  const cached = await ctx.kv.get(`player:${id}`);
  const p = cached ? (JSON.parse(cached) as Player) : null;
  if (p && Date.now() - p.at < maxAgeSec * 1000) return p;
  const fresh = await fetchPlayer(id, ctx);
  await ctx.kv.set(`player:${id}`, JSON.stringify(fresh), PLAYER_TTL);
  return fresh;
}

const cvhIdOf = (providers: Provider[]): string | undefined => {
  for (const p of providers) {
    if (p.slug !== "cvh") continue;
    const id = parseCvh(p.url)?.id;
    if (id) return id;
  }
  return undefined;
};

/** Ids beyond the first page come 100 at a time; they are kept in `pl` for the next lookups. */
async function episodeId(animeId: string, n: number, pl: Player, ctx: Ctx): Promise<string | undefined> {
  if (pl.ids[n]) return pl.ids[n];
  const html = await xhr(`/player/${encodeURIComponent(animeId)}/episodes?number=${n}`, ctx);
  for (const e of parseEpisodes(html, ctx)) pl.ids[e.number] = e.id;
  return pl.ids[n];
}

/** The players of one episode. A movie without an episode bar has them on the player page itself. */
async function episodeProviders(animeId: string, n: number, pl: Player, ctx: Ctx): Promise<Provider[]> {
  const id = await episodeId(animeId, n, pl, ctx);
  if (!id) return n === pl.current ? pl.providers : [];
  return parseProviders(await xhr(`/player/videos/${id}`, ctx), ctx);
}

function toProbe(providers: Provider[]): Probes[number] {
  const out: Probes[number] = {};
  for (const p of providers) {
    const e = (out[p.tr] ??= { name: p.name, slugs: [] });
    if (!e.slugs.includes(p.slug)) e.slugs.push(p.slug);
    if (p.slug === "cvh") e.cvh = parseCvh(p.url)?.studio ?? e.cvh;
  }
  return out;
}

// ---------------------------------------------------------------- search

async function searchPage(name: string, q: SearchQuery, ctx: Ctx): Promise<Hit[]> {
  const res = await ctx.fetch(`${ctx.baseUrl}/search/anime?q=${encodeURIComponent(name)}`);
  if (!res.ok) throw new Error(`animego: search answered HTTP ${res.status}`);
  const html = await res.text();
  const links = ctx.html.select(html, ".ani-grid__item-title a");
  const bodies = ctx.html.select(html, ".ani-grid__item-body");
  const hits: Hit[] = [];
  links.forEach((a, i) => {
    const id = /-(\d+)\/?$/.exec(a.attrs.href ?? "")?.[1];
    if (!id) return;
    const title = (a.attrs.title ?? a.text).trim();
    // Each card body holds the original name, the Russian title, the kind and the year on separate lines.
    const lines = (bodies.length === links.length ? bodies[i].text : "").split("\n").map((s) => s.trim()).filter(Boolean);
    const year = Number(lines.find((l) => /^(19|20)\d\d$/.test(l)));
    const names = [title, ...lines.filter((l) => l.length > 2 && !/^(19|20)\d\d$/.test(l))];
    let score = Math.max(...names.flatMap((n) => q.names.map((qn) => similarity(n, qn))));
    if (q.year && year) score += year === q.year ? 0.1 : Math.abs(year - q.year) > 1 ? -0.3 : 0;
    hits.push({ id, title, names, year: year || undefined, score });
  });
  return hits;
}

/** Animego shows no external ids, but its CVH iframes carry the MAL id: that is the exact check. */
async function verify(hit: Hit, q: SearchQuery, ctx: Ctx): Promise<Match | null> {
  let cvh: string | undefined;
  try {
    cvh = cvhIdOf((await player(hit.id, ctx, PLAYER_REUSE)).providers);
  } catch {
    cvh = undefined;
  }
  const ids = [String(q.malId), String(q.shikimoriId)];
  if (cvh && !ids.includes(cvh)) return null;
  const nameAndYear = hit.score >= 1 && q.year != null && hit.year === q.year;
  const data: MatchData = cvh ? { cvh } : {};
  return { id: hit.id, title: hit.title, confidence: cvh || nameAndYear ? "exact" : "fuzzy", data };
}

/** Name similarity (with the year bonus) a search hit needs before it is checked at all. */
const MIN_SCORE = 0.8;

export async function search(q: SearchQuery, ctx: Ctx): Promise<Match[]> {
  const hits = new Map<string, Hit>();
  for (const name of q.names.slice(0, 3)) {
    for (const h of await searchPage(name, q, ctx)) {
      const seen = hits.get(h.id);
      if (!seen || seen.score < h.score) hits.set(h.id, h);
    }
    if ([...hits.values()].some((h) => h.score >= 1)) break;
  }
  // 0.8: names sharing only a word or two ("Гробница богов 3" and "Убийца богов 3") score about 0.7.
  const top = [...hits.values()].filter((h) => h.score >= MIN_SCORE).sort((a, b) => b.score - a.score).slice(0, 3);
  const verified = await Promise.all(top.map((h) => verify(h, q, ctx)));
  return verified
    .filter((m): m is Match => m !== null)
    .sort((a, b) => Number(b.confidence === "exact") - Number(a.confidence === "exact"));
}

// ---------------------------------------------------------------- episodes

/** Episode ranges where a dub is known to be: between two probed episodes that both have it. */
function coverage(probes: Probes, tr: string): [number, number][] {
  const points = Object.keys(probes).map(Number).sort((a, b) => a - b);
  const runs: [number, number][] = [];
  let start: number | undefined;
  let prev = 0;
  for (const n of points) {
    if (probes[n][tr]) {
      start ??= n;
      prev = n;
    } else if (start !== undefined) {
      runs.push([start, prev]);
      start = undefined;
    }
  }
  if (start !== undefined) runs.push([start, prev]);
  return runs;
}

/** The middle of the widest stretch where some dub appears or disappears between two probes. */
function nextProbe(probes: Probes, skip: Set<string>): number | undefined {
  const points = Object.keys(probes).map(Number).sort((a, b) => a - b);
  const trs = new Set(points.flatMap((n) => Object.keys(probes[n])));
  let best: [number, number] | undefined;
  for (let i = 1; i < points.length; i++) {
    const [a, b] = [points[i - 1], points[i]];
    if (b - a < 2 || (best && b - a <= best[1] - best[0])) continue;
    for (const tr of trs) {
      if (!skip.has(tr) && !probes[a][tr] !== !probes[b][tr]) {
        best = [a, b];
        break;
      }
    }
  }
  return best && Math.floor((best[0] + best[1]) / 2);
}

/**
 * Which dubs each episode has. Asking every episode is too slow (and DDoS-Guard would notice), so this
 * probes both ends, then bisects where a dub appears or disappears. Probes are cached, so later calls
 * refine the picture further; the ends are always asked fresh because new dubs show up there first.
 */
class EpisodeProber {
  readonly probes: Probes = {};
  private readonly started = Date.now();

  constructor(
    private readonly m: Match,
    private readonly pl: Player,
    private readonly ctx: Ctx,
  ) {}

  private async probe(n: number): Promise<void> {
    this.probes[n] = toProbe(await episodeProviders(this.m.id, n, this.pl, this.ctx));
  }

  async ends(): Promise<void> {
    const cached = await this.ctx.kv.get(`probes:${this.m.id}`);
    if (cached) Object.assign(this.probes, JSON.parse(cached) as Probes);
    this.probes[this.pl.current] = toProbe(this.pl.providers);
    const { numbers, current } = this.pl;
    const ends = new Set([numbers[0], numbers[numbers.length - 1]].filter((n) => n !== undefined && n !== current));
    await Promise.all([...ends].map((n) => this.probe(n)));
  }

  /** `skip`: dubs CVH already lists episode by episode. */
  async refine(skip: Set<string>): Promise<Probes> {
    for (let i = 0; i < MAX_PROBES && Date.now() - this.started < PROBE_BUDGET_MS; i++) {
      const n = nextProbe(this.probes, skip);
      if (n === undefined || this.probes[n]) break;
      await this.probe(n);
    }
    await this.ctx.kv.set(`probes:${this.m.id}`, JSON.stringify(this.probes), PROBES_TTL);
    await this.ctx.kv.set(`player:${this.m.id}`, JSON.stringify(this.pl), PLAYER_TTL);
    return this.probes;
  }
}

interface CvhDub {
  name: string;
  subtitles: boolean;
  episodes: Set<number>;
}

function groupCvh(items: CvhItem[]): Map<string, CvhDub> {
  const out = new Map<string, CvhDub>();
  for (const i of items) {
    const name = cvhStudio(i);
    const k = cvhKey(name);
    if (!k) continue;
    const d = out.get(k) ?? { name, subtitles: true, episodes: new Set<number>() };
    d.episodes.add(i.episode ?? 1);
    d.subtitles &&= /субтитр/i.test(i.voiceType ?? "");
    out.set(k, d);
  }
  return out;
}

/** CVH's own name for an Animego dub: the one in Animego's CVH iframe, else the same spelling. */
function cvhFor(iframeStudio: string | undefined, name: string, cvh: Map<string, CvhDub>): string | undefined {
  for (const wanted of [iframeStudio, name]) {
    const k = cvhKey(wanted);
    if (!k) continue;
    if (cvh.has(k)) return k;
    if (wanted === iframeStudio) for (const key of cvh.keys()) if (key.includes(k) || k.includes(key)) return key;
  }
  return undefined;
}

/** Dub ids: "<translation>" for Animego-only, "<translation>|<CVH studio>" when CVH has it too, "|<CVH studio>" for CVH-only. */
function parseDubId(dubId: string): { tr?: string; studio?: string } {
  const bar = dubId.indexOf("|");
  if (bar < 0) return { tr: dubId };
  return { tr: dubId.slice(0, bar) || undefined, studio: dubId.slice(bar + 1) || undefined };
}

export async function episodes(m: Match, ctx: Ctx): Promise<EpisodeList> {
  const pl = await player(m.id, ctx, PLAYER_REUSE);
  const cvhId = cvhIdOf(pl.providers) ?? (m.data as MatchData | undefined)?.cvh;
  const prober = new EpisodeProber(m, pl, ctx);
  const [items] = await Promise.all([
    cvhId ? cvhPlaylist(cvhId, ctx).catch((): CvhItem[] => []) : Promise.resolve([] as CvhItem[]),
    prober.ends(),
  ]);
  const cvh = groupCvh(items);
  const mapped = new Set<string>();
  for (const probe of Object.values(prober.probes)) {
    for (const [tr, e] of Object.entries(probe)) if (cvhFor(e.cvh, pl.names[tr] ?? e.name, cvh)) mapped.add(tr);
  }
  const probes = await prober.refine(mapped);

  const trs: string[] = [];
  const names: Record<string, string> = { ...pl.names };
  const iframeStudio: Record<string, string> = {};
  const aniboom = new Set<string>();
  for (const n of Object.keys(probes).map(Number).sort((a, b) => a - b)) {
    for (const [tr, e] of Object.entries(probes[n])) {
      if (!trs.includes(tr)) trs.push(tr);
      names[tr] ??= e.name;
      if (e.cvh) iframeStudio[tr] ??= e.cvh;
      if (e.slugs.includes("aniboom")) aniboom.add(tr);
    }
  }
  const order = Object.keys(pl.names);
  trs.sort((a, b) => (order.indexOf(a) + 1 || 1e9) - (order.indexOf(b) + 1 || 1e9));

  const dubs: Dub[] = [];
  const avail = new Map<string, (n: number) => number | undefined>();
  const used = new Set<string>();
  for (const tr of trs) {
    const name = names[tr] || `Озвучка ${tr}`;
    const key = cvhFor(iframeStudio[tr], name, cvh);
    const c = key ? cvh.get(key) : undefined;
    if (key) used.add(key);
    const id = c ? `${tr}|${c.name}` : tr;
    const subtitles = /субтитр/i.test(name) || (c?.subtitles ?? false);
    dubs.push({ id, name, type: subtitles ? "subtitles" : "voice" });
    const runs = coverage(probes, tr);
    const own = aniboom.has(tr) ? HD : SD;
    avail.set(id, (n) => (c?.episodes.has(n) ? HD : runs.some(([a, b]) => n >= a && n <= b) ? own : undefined));
  }
  for (const [key, c] of cvh) {
    if (used.has(key)) continue;
    const id = `|${c.name}`;
    dubs.push({ id, name: c.name, type: c.subtitles ? "subtitles" : "voice" });
    avail.set(id, (n) => (c.episodes.has(n) ? HD : undefined));
  }

  const numbers = new Set(pl.numbers);
  for (const c of cvh.values()) for (const n of c.episodes) numbers.add(n);
  return {
    dubs,
    episodes: [...numbers]
      .sort((a, b) => a - b)
      .map((number) => ({
        number,
        available: dubs.flatMap((d) => {
          const q = avail.get(d.id)?.(number);
          return q ? [{ dubId: d.id, maxQuality: q }] : [];
        }),
      })),
  };
}

// ---------------------------------------------------------------- resolve

interface Attempt {
  player: string;
  run: () => Promise<Stream[]>;
}

function attempts(providers: Provider[], studio: string | undefined, cvhId: string | undefined, episode: number, ctx: Ctx): Attempt[] {
  const out: Attempt[] = [];
  const sorted = [...providers].sort((a, b) => (PLAYER_ORDER.indexOf(a.slug) + 1 || 99) - (PLAYER_ORDER.indexOf(b.slug) + 1 || 99));
  let cvhAdded = false;
  const addCvh = (fromIframe?: string) => {
    if (cvhAdded) return;
    const parsed = fromIframe ? parseCvh(fromIframe) : null;
    const id = parsed?.id ?? cvhId;
    const dub = studio ?? parsed?.studio;
    if (!id || !dub) return;
    cvhAdded = true;
    const url = cvhPlaylistUrl({ id, studio: dub, episode, season: parsed?.season });
    out.push({ player: "cvh", run: () => resolveCvh(url, ctx) });
  };
  for (const p of sorted) {
    if (p.slug === "aniboom") out.push({ player: "aniboom", run: () => resolveAniboom(p.url, ctx, `${ctx.baseUrl}/`) });
    else if (p.slug === "cvh") addCvh(p.url);
    else if (p.slug === "kodik") {
      addCvh();
      out.push({ player: "kodik", run: () => resolveKodik(p.url, ctx) });
    }
  }
  addCvh();
  return out;
}

export async function resolve(m: Match, episode: number, dubId: string, ctx: Ctx): Promise<Stream[]> {
  const { tr, studio } = parseDubId(dubId);
  const pl = await player(m.id, ctx, PLAYER_TTL);
  const cvhId = cvhIdOf(pl.providers) ?? (m.data as MatchData | undefined)?.cvh;
  const providers = tr ? (await episodeProviders(m.id, episode, pl, ctx)).filter((p) => p.tr === tr) : [];
  const tries = attempts(providers, studio, cvhId, episode, ctx);
  if (tries.length === 0) ctx.fail.notFound(`AnimeGO has no episode ${episode} in this dub`);
  const failures: string[] = [];
  let challenge: Error | undefined;
  let allNotFound = true;
  for (const t of tries) {
    try {
      const streams = await t.run();
      if (streams.length > 0) return streams;
      failures.push(`${t.player}: no streams`);
      allNotFound = false;
    } catch (e) {
      const err = e instanceof Error ? e : new Error(String(e));
      ctx.log(`animego: ${t.player} failed: ${err.message}`);
      // The core reads the first "__EXT:<Kind>:" marker anywhere in a message, so the summary drops them.
      failures.push(`${t.player}: ${err.message.replace(/__EXT:(\w+):/g, "$1: ")}`);
      if (err.message.includes("__EXT:Challenge:")) challenge ??= err;
      if (!err.message.includes("__EXT:NotFound:")) allNotFound = false;
    }
  }
  if (challenge) throw challenge;
  const summary = failures.join("; ");
  if (allNotFound) ctx.fail.notFound(summary);
  throw new Error(`animego: every player failed (${summary})`);
}

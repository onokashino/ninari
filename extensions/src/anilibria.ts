// AniLibria (AniLiberty) public API v1: https://anilibria.top/api/docs/v1
import type { Ctx, EpisodeList, Manifest, Match, SearchQuery, Stream } from "./types";

export const manifest: Manifest = {
  id: "anilibria",
  name: "AniLibria",
  version: "0.1.0",
  apiVersion: 1,
  domains: ["https://anilibria.top", "https://api.anilibria.app", "https://aniliberty.top"],
  allowedHosts: ["cache.libria.fun"],
};

const DUB_ID = "aniliberty";
const QUALITIES = [1080, 720, 480] as const;

interface ApiRelease {
  id: number;
  name?: { main?: string };
  shikimori?: { id?: number };
  is_blocked_by_geo?: boolean;
  episodes?: ApiEpisode[];
}

interface ApiEpisode {
  ordinal: number;
  hls_480?: string | null;
  hls_720?: string | null;
  hls_1080?: string | null;
  opening?: { start: number | null; stop: number | null };
  ending?: { start: number | null; stop: number | null };
}

const span = (x?: { start: number | null; stop: number | null }): [number, number] | undefined =>
  x && x.start != null && x.stop != null ? [x.start, x.stop] : undefined;

const maxQuality = (e: ApiEpisode) => QUALITIES.find((q) => e[`hls_${q}`]);

async function release(id: string, ctx: Ctx): Promise<ApiRelease> {
  const res = await ctx.fetch(`${ctx.baseUrl}/api/v1/anime/releases/${encodeURIComponent(id)}`);
  if (res.status === 404) ctx.fail.notFound(`release ${id}`);
  if (!res.ok) throw new Error(`anilibria: HTTP ${res.status}`);
  const r = (await res.json()) as ApiRelease;
  if (r.is_blocked_by_geo) ctx.fail.geoBlocked("AniLibria закрыла релиз в этом регионе");
  return r;
}

export async function search(q: SearchQuery, ctx: Ctx): Promise<Match[]> {
  const fuzzy = new Map<number, Match>();
  for (const name of q.names.slice(0, 4)) {
    const res = await ctx.fetch(
      `${ctx.baseUrl}/api/v1/app/search/releases?query=${encodeURIComponent(name)}`,
    );
    if (!res.ok) continue;
    const items = (await res.json()) as ApiRelease[];
    for (const r of items) {
      const title = r.name?.main ?? name;
      if (r.shikimori?.id === q.shikimoriId) return [{ id: String(r.id), title, confidence: "exact" }];
      if (!fuzzy.has(r.id)) fuzzy.set(r.id, { id: String(r.id), title, confidence: "fuzzy" });
    }
  }
  return [...fuzzy.values()];
}

export async function episodes(m: Match, ctx: Ctx): Promise<EpisodeList> {
  const r = await release(m.id, ctx);
  return {
    dubs: [{ id: DUB_ID, name: "AniLibria", type: "voice" }],
    episodes: (r.episodes ?? []).map((e) => ({
      number: e.ordinal,
      available: [{ dubId: DUB_ID, maxQuality: maxQuality(e) }],
      skip: { opening: span(e.opening), ending: span(e.ending) },
    })),
  };
}

export async function resolve(m: Match, episode: number, _dubId: string, ctx: Ctx): Promise<Stream[]> {
  const r = await release(m.id, ctx);
  const e = (r.episodes ?? []).find((x) => x.ordinal === episode);
  if (!e) ctx.fail.notFound(`episode ${episode}`);
  const out: Stream[] = [];
  for (const q of QUALITIES) {
    const url = e[`hls_${q}`];
    if (url) out.push({ url, kind: "hls", quality: q });
  }
  if (out.length === 0) ctx.fail.notFound(`episode ${episode} has no streams`);
  return out;
}

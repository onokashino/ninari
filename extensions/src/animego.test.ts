// Contract test against the live animego.me and the players behind it.
import { describe, expect, test } from "bun:test";
import * as ext from "./animego";
import { makeCtx } from "./testing/host";
import type { Ctx, HtmlNode, Match } from "./types";

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };
const unescape = (s: string) =>
  s.replace(/&(#x[0-9a-f]+|#\d+|\w+);/gi, (m, e: string) =>
    e[0] === "#" ? String.fromCodePoint(e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : Number(e.slice(1))) : (ENTITIES[e.toLowerCase()] ?? m),
  );

/** The test host has no ctx.html.select; HTMLRewriter gives the same attrs and text (entities decoded), no inner html. */
function select(html: string, css: string): HtmlNode[] {
  const out: HtmlNode[] = [];
  const open: HtmlNode[] = [];
  new HTMLRewriter()
    .on(css, {
      element(el) {
        const node: HtmlNode = { text: "", html: "", attrs: Object.fromEntries([...el.attributes].map(([k, v]) => [k, unescape(v)])) };
        out.push(node);
        open.push(node);
        try {
          el.onEndTag(() => void open.splice(open.indexOf(node), 1));
        } catch {
          open.pop();
        }
      },
      text(t) {
        for (const n of open) n.text += t.text;
      },
    })
    .transform(html);
  return out.map((n) => ({ ...n, text: unescape(n.text) }));
}

const ctx: Ctx = { ...makeCtx(ext.manifest.domains[0]), html: { select } };
const frieren = {
  shikimoriId: 52991,
  malId: 52991,
  names: ["Провожающая в последний путь Фрирен", "Sousou no Frieren"],
};
const FRIEREN: Match = { id: "2430", title: "Провожающая в последний путь Фрирен", confidence: "exact", data: { cvh: "52991" } };

describe("animego (live)", () => {
  test("manifest lets the players reach their hosts", () => {
    expect(ext.manifest.domains[0]).toBe("https://animego.me");
    expect(ext.manifest.allowedHosts).toEqual(expect.arrayContaining(["aniboom.one", "plapi.cdnvideohub.com"]));
  });

  test("finds Frieren and verifies it by the MAL id in its CVH iframe", async () => {
    const matches = await ext.search(frieren, ctx);
    expect(matches[0]).toMatchObject({ id: "2430", confidence: "exact", data: { cvh: "52991" } });
    // The second season shows up in the results but carries another MAL id.
    expect(matches.some((m) => m.id === "2911")).toBe(false);
  }, 60_000);

  test("finds a movie by its romaji name", async () => {
    const matches = await ext.search({ shikimoriId: 32281, malId: 32281, names: ["Kimi no Na wa."] }, ctx);
    expect(matches[0]).toMatchObject({ id: "107", confidence: "exact" });
  }, 60_000);

  test("does not claim a title Animego lacks", async () => {
    const matches = await ext.search({ shikimoriId: 44511, malId: 44511, names: ["Человек-бензопила", "Chainsaw Man"] }, ctx);
    expect(matches.filter((m) => m.confidence === "exact")).toEqual([]);
  }, 60_000);

  test("does not take a title with a similar name for the one asked", async () => {
    // "Гробница богов 3" (Shen Mu) once matched "Убийца богов 3", another donghua, by "богов 3".
    const found = await ext.search(
      { shikimoriId: 61921, malId: 61921, names: ["Гробница богов 3", "Shen Mu: Nian Fan"], year: 2025, kind: "ona", episodes: 52 },
      ctx,
    );
    expect(found.map((m) => m.title)).not.toContain("Убийца богов 3");
  }, 60_000);

  test("lists 28 episodes with Animego and CVH dubs", async () => {
    const list = await ext.episodes(FRIEREN, ctx);
    expect(list.episodes.length).toBe(28);
    const names = list.dubs.map((d) => d.name);
    expect(names).toEqual(expect.arrayContaining(["Dream Cast", "AniLibria", "Студийная Банда", "AniStar"]));
    // CVH carries dubs Animego's own menu does not show.
    expect(list.dubs).toContainEqual({ id: "|Kitsune Studio", name: "Kitsune Studio", type: "voice" });
    expect(list.dubs).toContainEqual({ id: "81|Dream Cast", name: "Dream Cast", type: "voice" });
    expect(list.dubs.find((d) => d.name === "Субтитры")?.type).toBe("subtitles");
    for (const n of [1, 14, 28]) {
      const ep = list.episodes.find((e) => e.number === n)!;
      expect(ep.available).toContainEqual({ dubId: "81|Dream Cast", maxQuality: 1080 });
    }
  }, 60_000);

  test("resolves Dream Cast episode 1 through Aniboom in 1080p", async () => {
    const streams = await ext.resolve(FRIEREN, 1, "81|Dream Cast", ctx);
    expect(streams[0]).toMatchObject({ kind: "hls", quality: 1080, headers: { Referer: "https://aniboom.one/" } });
    expect(streams[0].url).toEndWith("master.m3u8");
  }, 60_000);

  test("resolves a CVH-only dub", async () => {
    const streams = await ext.resolve(FRIEREN, 2, "|Kitsune Studio", ctx);
    expect(streams[0]).toMatchObject({ kind: "hls", quality: 1080 });
    expect(streams[0].url).toContain("okcdn.ru");
  }, 60_000);

  test("falls back to CVH when the dub has no Aniboom", async () => {
    const streams = await ext.resolve(FRIEREN, 3, "4|SHIZA Project", ctx);
    expect(streams[0].url).toContain("okcdn.ru");
  }, 60_000);

  test("movie: one episode, resolves", async () => {
    const movie: Match = { id: "107", title: "Твоё имя", confidence: "exact" };
    const list = await ext.episodes(movie, ctx);
    expect(list.episodes.map((e) => e.number)).toEqual([1]);
    const dub = list.dubs.find((d) => d.name === "Дубляж")!;
    const streams = await ext.resolve(movie, 1, dub.id, ctx);
    expect(streams[0].kind).toBe("hls");
  }, 60_000);

  test("unknown episode is NotFound", async () => {
    await expect(ext.resolve(FRIEREN, 999, "81|Dream Cast", ctx)).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});

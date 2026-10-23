// Aniboom player: contract tests against the live aniboom.one (behind DDoS-Guard).
import { describe, expect, test } from "bun:test";
import { makeCtx } from "../testing/host";
import type { Ctx, HtmlNode } from "../types";
import { isAniboom, resolveAniboom } from "./aniboom";

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

const ctx: Ctx = { ...makeCtx("https://animego.me"), html: { select } };
// Frieren, episode 1, Dream Cast, exactly as Animego's player list gives it.
const FRIEREN =
  "//aniboom.one/embed/vy7q8mlM8bg?episode=1&amp;translation=30&amp;episodes=1&amp;parent=https%3A%2F%2Fanimego.me%2Fanime%2Fprovozhayuschaya-v-posledniy-put-friren-2430";

describe("aniboom", () => {
  test("isAniboom", () => {
    expect(isAniboom(FRIEREN)).toBe(true);
    expect(isAniboom("https://aniboom.one/embed/9G1MJlRdV8z?translation=25")).toBe(true);
    expect(isAniboom("//kodikplayer.com/seria/1/abc/720p")).toBe(false);
    expect(isAniboom("https://notaniboom.one/embed/x")).toBe(false);
  });

  test("embed -> 1080p HLS master with audio in EXT-X-MEDIA", async () => {
    const streams = await resolveAniboom(FRIEREN, ctx);
    expect(streams[0]).toMatchObject({ kind: "hls", quality: 1080, headers: { Referer: "https://aniboom.one/" } });
    expect(streams[0].url).toEndWith("master.m3u8");
    const master = await fetch(streams[0].url, { headers: streams[0].headers });
    expect(master.status).toBe(200);
    const body = await master.text();
    expect(body).toContain("RESOLUTION=1920x1080");
    expect(body).toContain("#EXT-X-MEDIA:TYPE=AUDIO");
  }, 60_000);

  test("fMP4 segments play with the returned headers", async () => {
    const [s] = await resolveAniboom(FRIEREN, ctx);
    const master = await (await fetch(s.url, { headers: s.headers })).text();
    const variant = new URL(master.split("\n").filter((l) => l && !l.startsWith("#")).pop()!, s.url).href;
    const media = await (await fetch(variant, { headers: s.headers })).text();
    const segment = new URL(media.split("\n").find((l) => l && !l.startsWith("#"))!, variant).href;
    const r = await fetch(segment, { headers: { ...s.headers, Range: "bytes=0-1023" } });
    expect([200, 206]).toContain(r.status);
  }, 60_000);

  test("widescreen AV1 upload: 1920x804 counts as 1080p, the H.264 fallback follows", async () => {
    const streams = await resolveAniboom("//aniboom.one/embed/8rVX61VMl40?episode=13&amp;translation=49&amp;episodes=1", ctx);
    expect(streams[0].quality).toBe(1080);
    expect(streams[1].kind).toBe("hls");
    expect(streams[1].quality).toBeUndefined();
    expect(streams[1].url).toContain("/h264/");
  }, 60_000);

  test("movie embed has no episode", async () => {
    const streams = await resolveAniboom("//aniboom.one/embed/9G1MJlRdV8z?translation=25&amp;episodes=1", ctx);
    expect(streams[0].kind).toBe("hls");
  }, 60_000);

  test("missing episode is NotFound", async () => {
    await expect(resolveAniboom("https://aniboom.one/embed/vy7q8mlM8bg?episode=99&translation=30", ctx)).rejects.toThrow(
      "__EXT:NotFound",
    );
  }, 60_000);
});

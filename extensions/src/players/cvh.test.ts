// CVH player: input parsing offline, then contract tests against the live plapi.cdnvideohub.com.
import { describe, expect, test } from "bun:test";
import { makeCtx } from "../testing/host";
import { cvhDubQualities, cvhKey, cvhPlaylistUrl, isCvh, parseCvh, resolveCvh } from "./cvh";

const ctx = makeCtx("https://animego.me");

describe("cvh qualities (live)", () => {
  test("tells the real best quality of every dub and remembers it", async () => {
    // Shen Mu: Nian Fan (a donghua): AniStar uploaded it in 4K.
    const counted = makeCtx("https://animego.me");
    let requests = 0;
    const fetch = counted.fetch.bind(counted);
    counted.fetch = (url, init) => {
      requests++;
      return fetch(url, init);
    };
    const first = await cvhDubQualities("61921", counted);
    expect(first.get(cvhKey("AniStar"))).toBe(2160);
    const before = requests;
    const again = await cvhDubQualities("61921", counted);
    expect(again.get(cvhKey("AniStar"))).toBe(2160);
    expect(requests - before).toBe(1);
  }, 60_000);
});

describe("cvh input", () => {
  test("old Animego iframe keeps the dub in the path", () => {
    expect(parseCvh("//animego.me/cdn-iframe/52991/Dream Cast/1/2")).toEqual({
      pub: "747",
      aggr: "mali",
      id: "52991",
      studio: "Dream Cast",
      season: 1,
      episode: 2,
    });
  });

  test("new Animego iframe has the dub in ?dubbing=", () => {
    expect(parseCvh("//animego.me/cdn-iframe/61619/1/3?dubbing=AniMy%20%2F%20RuChiMe")).toMatchObject({
      id: "61619",
      studio: "AniMy / RuChiMe",
      season: 1,
      episode: 3,
    });
  });

  test("Animego movie iframe has no season or episode", () => {
    const r = parseCvh("//animego.me/cdn-iframe/32281?dubbing=SHIZA%20Project");
    expect(r).toMatchObject({ id: "32281", studio: "SHIZA Project" });
    expect(r?.episode).toBeUndefined();
  });

  test("YummyAnime iframe uses its own publisher", () => {
    expect(
      parseCvh("//ru.yummyani.me/iframeCVH.html?dubbing_code=AnilibriaTV&anime_id=52991&episode=1&dubbing=%D0%9E%D0%B7%D0%B2%D1%83%D1%87%D0%BA%D0%B0+AniLibria"),
    ).toEqual({ pub: "745", aggr: "mali", id: "52991", studio: "AnilibriaTV", season: undefined, episode: 1 });
  });

  test("the canonical API form round-trips", () => {
    const url = cvhPlaylistUrl({ id: 57658, studio: "Студийная Банда", episode: 12 });
    expect(parseCvh(url)).toMatchObject({ id: "57658", studio: "Студийная Банда", episode: 12 });
    expect(parseCvh("https://plapi.cdnvideohub.com/api/v1/player/sv/video/7584549132906")?.video).toBe("7584549132906");
  });

  test("isCvh tells CVH links from other players", () => {
    expect(isCvh("//animego.me/cdn-iframe/52991/AniDUB/1/1")).toBe(true);
    expect(isCvh("//ru.yummyani.me/iframeCVH.html?dubbing_code=X&anime_id=1&episode=1")).toBe(true);
    expect(isCvh("//animego.me/seria/1558479/00b7d55b0a69b8e1d0bd98c1de7b8d20/720p")).toBe(false);
    expect(isCvh("//kodikplayer.com/seria/1/abc/720p")).toBe(false);
    expect(isCvh("//aniboom.one/embed/vy7q8mlM8bg?episode=1")).toBe(false);
  });
});

describe("cvh (live)", () => {
  test("Animego iframe: 1080p HLS master first, then MP4 by quality", async () => {
    const streams = await resolveCvh("//animego.me/cdn-iframe/52991/Dream Cast/1/1", ctx);
    expect(streams[0]).toMatchObject({ kind: "hls", quality: 1080 });
    expect(streams.find((s) => s.kind === "mp4")?.quality).toBe(1080);
    const master = await fetch(streams[0].url);
    expect(master.status).toBe(200);
    expect(await master.text()).toContain("RESOLUTION=1920x1080");
  }, 60_000);

  test("MP4 answers ranged requests without extra headers", async () => {
    const streams = await resolveCvh("//animego.me/cdn-iframe/52991/1/2?dubbing=AnilibriaTV", ctx);
    const mp4 = streams.find((s) => s.kind === "mp4");
    const r = await fetch(mp4!.url, { headers: { Range: "bytes=0-1023" } });
    expect(r.status).toBe(206);
    expect(r.headers.get("content-type")).toContain("video/mp4");
  }, 60_000);

  test("YummyAnime iframe and a loosely spelled dub", async () => {
    const streams = await resolveCvh("//ru.yummyani.me/iframeCVH.html?dubbing_code=anilibria%20tv&anime_id=52991&episode=5", ctx);
    expect(streams[0].kind).toBe("hls");
  }, 60_000);

  test("season numbers do not have to agree with CVH (JJK S3 is 3xN there)", async () => {
    const streams = await resolveCvh("//animego.me/cdn-iframe/57658/Jam Club/1/1", ctx);
    expect(streams.length).toBeGreaterThan(0);
  }, 60_000);

  test("movie", async () => {
    const streams = await resolveCvh("//animego.me/cdn-iframe/32281?dubbing=SHIZA%20Project", ctx);
    expect(streams[0].kind).toBe("hls");
  }, 60_000);

  test("unknown dub, unknown episode and unknown title are NotFound", async () => {
    await expect(resolveCvh("//animego.me/cdn-iframe/52991/No Such Team/1/1", ctx)).rejects.toThrow("__EXT:NotFound");
    await expect(resolveCvh("//animego.me/cdn-iframe/52991/Dream Cast/1/99", ctx)).rejects.toThrow("__EXT:NotFound");
    await expect(resolveCvh(cvhPlaylistUrl({ id: 999999999 }), ctx)).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});

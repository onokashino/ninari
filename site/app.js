// ninari site: motion that answers scrolling and the pointer, and the live data of the download
// and plugins pages. Every page reads in full without this file.

const REGISTRY = "https://raw.githubusercontent.com/onokashino/ninari/master/extensions/dist/index.json";
const RELEASES = "https://api.github.com/repos/onokashino/ninari/releases/latest";
const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;

function plural(n, [one, few, many]) {
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
}

// ---------- motion ----------

/** "и ещё 34 озвучки" counts up, the word agreeing with every number on the way. */
function countUp(el) {
  const target = Number(el.dataset.count);
  const word = el.parentElement.querySelector("[data-plural]");
  const forms = word?.dataset.plural.split(",");
  const start = performance.now();
  const tick = (now) => {
    const k = Math.min(1, (now - start) / 900);
    const n = Math.round(target * (1 - Math.pow(1 - k, 3)));
    el.textContent = String(n);
    if (forms) word.textContent = plural(n, forms);
    if (k < 1) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

function numberChildren(el) {
  [...el.children].forEach((child, i) => child.style.setProperty("--i", String(i)));
}

/** Blocks marked data-reveal play their entrance once, when they come into view. */
function setupReveal() {
  const blocks = document.querySelectorAll("[data-reveal]");
  blocks.forEach(numberChildren);
  // The player panel switches its settings on one after another.
  document.querySelectorAll(".panel .on").forEach((el, i) => el.style.setProperty("--i", String(i)));
  if (calm || !("IntersectionObserver" in window)) {
    blocks.forEach((el) => el.classList.add("in"));
    return;
  }
  const seen = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        e.target.classList.add("in");
        e.target.querySelectorAll("[data-count]").forEach(countUp);
        seen.unobserve(e.target);
      }
    },
    { threshold: 0.2 },
  );
  blocks.forEach((el) => seen.observe(el));
}

/** The hero screenshot leans toward the pointer, with a soft light where it points. */
function setupTilt() {
  if (calm || !matchMedia("(pointer: fine)").matches) return;
  for (const el of document.querySelectorAll("[data-tilt]")) {
    const area = el.closest("section") ?? el;
    let frame = 0;
    area.addEventListener("pointermove", (e) => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const r = el.getBoundingClientRect();
        const x = (e.clientX - r.left) / r.width;
        const y = (e.clientY - r.top) / r.height;
        el.style.setProperty("--ry", `${((x - 0.5) * 7).toFixed(2)}deg`);
        el.style.setProperty("--rx", `${((0.5 - y) * 5).toFixed(2)}deg`);
        el.style.setProperty("--gx", `${(x * 100).toFixed(1)}%`);
        el.style.setProperty("--gy", `${(y * 100).toFixed(1)}%`);
        el.classList.add("tilting");
      });
    });
    area.addEventListener("pointerleave", () => {
      cancelAnimationFrame(frame);
      el.style.setProperty("--ry", "0deg");
      el.style.setProperty("--rx", "0deg");
      el.classList.remove("tilting");
    });
  }
}

// ---------- plugins page ----------

const ABOUT = {
  anilibria: "Релизы AniLibria с их собственной озвучкой, до 1080p.",
  animego: "Каталог AnimeGO: плееры Aniboom и CVH до 1080p, Kodik до 720p.",
  animelib: "Каталог AnimeLib, серии через плеер Kodik.",
  anixart: "Каталог приложения Anixart с разбивкой по командам озвучки.",
  rutube: "Официальные загрузки команд озвучки на Rutube, до 1080p.",
  shikicinema: "Ссылки на серии, которые пользователи Shikimori собрали из VK, ok.ru, Дзена и других плееров.",
  yummy: "Каталог YummyAnime: Kodik до 720p и CVH до 4K.",
};

const hostOf = (url) => String(url).replace(/^[a-z]+:\/\//i, "").split("/")[0];

function element(tag, className, text) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}

/** One catalogue card; everything from the registry goes in as text, never as markup. */
function pluginCard(ext) {
  const li = element("li", "plugin");
  const head = element("div", "plugin-head");
  head.append(element("h3", "", String(ext.name ?? ext.id)), element("span", "ver", String(ext.version ?? "")));
  const sites = [...new Set((Array.isArray(ext.domains) ? ext.domains : []).map(hostOf))];
  li.append(head, element("p", "", ABOUT[ext.id] ?? "Источник серий."), element("p", "sites", sites.join(", ")));
  return li;
}

async function loadPlugins() {
  const list = document.querySelector("[data-plugins]");
  if (!list) return;
  try {
    const res = await fetch(REGISTRY, { cache: "no-store" });
    if (!res.ok) return;
    const index = await res.json();
    const exts = Array.isArray(index.extensions) ? index.extensions : [];
    if (exts.length === 0) return;
    list.replaceChildren(...exts.map(pluginCard));
    numberChildren(list);
    const count = document.querySelector("[data-plugin-count]");
    if (count) count.textContent = String(exts.length);
  } catch {
    // The cards written into the page stay.
  }
}

function setupCopy() {
  for (const button of document.querySelectorAll("[data-copy-registry]")) {
    const label = button.textContent;
    button.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(REGISTRY);
        button.textContent = "Ссылка скопирована";
      } catch {
        button.textContent = REGISTRY;
      }
      setTimeout(() => (button.textContent = label), 2600);
    });
  }
}

/** The browser asks to open ninari; when it does not, the app is probably not installed yet. */
function setupInstallAll() {
  const link = document.querySelector("[data-install-all]");
  const hint = document.querySelector("[data-install-hint]");
  if (!link || !hint) return;
  link.addEventListener("click", () => {
    let left = false;
    const away = () => (left = true);
    window.addEventListener("blur", away, { once: true });
    setTimeout(() => {
      window.removeEventListener("blur", away);
      if (!left) hint.classList.add("attention");
    }, 1600);
  });
}

// ---------- download page ----------

async function loadRelease() {
  const link = document.querySelector("[data-release-link]");
  if (!link) return;
  try {
    const res = await fetch(RELEASES, { headers: { Accept: "application/vnd.github+json" } });
    if (!res.ok) return;
    const rel = await res.json();
    const assets = Array.isArray(rel.assets) ? rel.assets : [];
    const exe = assets.find((a) => /x64-setup\.exe$/i.test(a.name)) ?? assets.find((a) => /\.exe$/i.test(a.name));
    const version = String(rel.tag_name ?? "").replace(/^v/, "");
    const date = rel.published_at
      ? new Date(rel.published_at).toLocaleDateString("ru-RU", { day: "numeric", month: "long", year: "numeric" })
      : "";
    if (exe) {
      link.href = exe.browser_download_url;
      link.textContent = `Скачать ${version}`;
    }
    const line = document.querySelector("[data-release-line]");
    if (line && version) {
      const size = exe ? `, установщик ${Math.round(exe.size / 1048576)} МБ` : "";
      line.textContent = `Версия ${version}${date ? ` от ${date}` : ""}${size}. Дальше приложение обновляется само.`;
    }
    const notes = document.querySelector("[data-release-notes]");
    if (notes && rel.body) {
      notes.querySelector("[data-release-version]").textContent = version;
      notes.querySelector("[data-release-body]").textContent = rel.body;
      notes.hidden = false;
    }
  } catch {
    // The button keeps pointing at the releases page.
  }
}

setupReveal();
setupTilt();
setupCopy();
setupInstallAll();
loadPlugins();
loadRelease();

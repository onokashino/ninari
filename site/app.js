// ninari site: entrances as blocks come into view, the header, and the live data of the download
// and plugins pages. Scrolling is the browser's own. Every page reads in full without this file.

const REGISTRY = "https://raw.githubusercontent.com/onokashino/ninari/master/extensions/dist/index.json";
const RELEASES = "https://api.github.com/repos/onokashino/ninari/releases/latest";
const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
const TEXT = { copy: "Копировать", copied: "Скопировано", failed: "Не получилось", source: "Источник серий." };

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

/** In the docs, the heading on screen lights its line in "On this page". */
function setupToc() {
  const links = [...document.querySelectorAll(".docs-toc a")];
  if (!links.length || !("IntersectionObserver" in window)) return;
  const heads = links.map((l) => document.getElementById(decodeURIComponent(l.hash.slice(1)))).filter(Boolean);
  const watch = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        links.forEach((l) => l.classList.toggle("on", l.hash.slice(1) === e.target.id || decodeURIComponent(l.hash.slice(1)) === e.target.id));
      }
    },
    { rootMargin: "-15% 0px -75% 0px" },
  );
  heads.forEach((h) => watch.observe(h));
}

/** The header: it narrows once the page scrolls, a highlight slides to the link under the
 *  pointer, the section on screen marks its link, and on narrow screens a button opens the menu. */
function setupNav() {
  const bar = document.querySelector(".bar");
  if (!bar) return;
  const onScroll = () => bar.classList.toggle("scrolled", scrollY > 24);
  addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  const nav = bar.querySelector(".nav");
  const glow = nav?.querySelector(".nav-glow");
  const links = nav ? [...nav.querySelectorAll("a")] : [];
  const moveTo = (link) => {
    if (!glow) return;
    if (!link) {
      glow.style.opacity = "0";
      return;
    }
    glow.style.width = `${link.offsetWidth}px`;
    glow.style.translate = `${link.offsetLeft}px 0`;
    glow.style.opacity = "1";
  };
  links.forEach((link) => link.addEventListener("pointerenter", () => moveTo(link)));
  nav?.addEventListener("pointerleave", () => moveTo(null));

  const spied = links.filter((l) => l.dataset.spy && document.getElementById(l.dataset.spy));
  if (spied.length && "IntersectionObserver" in window) {
    const watch = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const link = spied.find((l) => l.dataset.spy === e.target.id);
          link?.classList.toggle("on", e.isIntersecting);
        }
      },
      { rootMargin: "-40% 0px -55% 0px" },
    );
    spied.forEach((l) => watch.observe(document.getElementById(l.dataset.spy)));
  }

  const toggle = bar.querySelector(".menu-toggle");
  const sheet = document.getElementById("menu");
  if (toggle && sheet) {
    const set = (open) => {
      toggle.setAttribute("aria-expanded", String(open));
      sheet.hidden = !open;
    };
    toggle.addEventListener("click", () => set(sheet.hidden));
    sheet.addEventListener("click", (e) => e.target.closest("a") && set(false));
    addEventListener("keydown", (e) => e.key === "Escape" && set(false));
  }
}

/** Questions open and close with their height easing, instead of jumping. */
function setupFaq() {
  if (calm) return;
  for (const item of document.querySelectorAll(".faq details")) {
    const summary = item.querySelector("summary");
    const body = item.querySelector("summary + *");
    if (!summary || !body) continue;
    summary.addEventListener("click", (e) => {
      e.preventDefault();
      const opening = !item.open;
      if (opening) item.open = true;
      const full = body.scrollHeight;
      const run = body.animate(
        [
          { height: opening ? "0px" : `${full}px`, opacity: opening ? 0 : 1 },
          { height: opening ? `${full}px` : "0px", opacity: opening ? 1 : 0 },
        ],
        { duration: 320, easing: "cubic-bezier(0.2, 0.7, 0.2, 1)" },
      );
      if (!opening) run.onfinish = () => (item.open = false);
    });
  }
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
  li.append(head, element("p", "", ABOUT[ext.id] ?? TEXT.source), element("p", "sites", sites.join(", ")));
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
  // "Скопировать ссылку на каталог" copies the registry; a [data-copy-from] button copies the text
  // of the element it names, like the install command.
  const buttons = [
    ...[...document.querySelectorAll("[data-copy-registry]")].map((b) => [b, () => REGISTRY]),
    ...[...document.querySelectorAll("[data-copy-from]")].map((b) => [
      b,
      () => document.querySelector(b.dataset.copyFrom)?.textContent.trim() ?? "",
    ]),
  ];
  for (const button of document.querySelectorAll("[data-copy-code]")) {
    button.textContent = TEXT.copy;
    buttons.push([button, () => button.parentElement.querySelector("code").textContent]);
  }
  for (const [button, text] of buttons) {
    const label = button.textContent;
    button.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(text());
        button.textContent = TEXT.copied;
      } catch {
        button.textContent = TEXT.failed;
      }
      setTimeout(() => (button.textContent = label), 2000);
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
  const links = document.querySelectorAll("[data-release-link]");
  if (!links.length) return;
  try {
    const res = await fetch(RELEASES, { headers: { Accept: "application/vnd.github+json" } });
    if (!res.ok) return;
    const rel = await res.json();
    const assets = Array.isArray(rel.assets) ? rel.assets : [];
    const linux = assets.find((a) => /linux-x86_64\.tar\.gz$/i.test(a.name));
    const windows = assets.find((a) => /windows-x86_64-setup\.exe$/i.test(a.name));
    const version = String(rel.tag_name ?? "").replace(/^v/, "");
    if (windows) {
      for (const link of document.querySelectorAll("[data-release-windows]")) link.href = windows.browser_download_url;
    }
    if (!linux) return;
    const mb = Math.round(linux.size / 1048576);
    // On Windows the main button gives the installer.
    const onWindows = windows && /Windows/i.test(navigator.userAgent);
    for (const link of links) {
      link.href = onWindows ? windows.browser_download_url : linux.browser_download_url;
      if (onWindows && !link.hasAttribute("data-keep-label")) link.textContent = "Скачать для Windows";
    }
    const cmd = document.querySelector("[data-install-cmd]");
    if (cmd) cmd.textContent = `curl -L ${linux.browser_download_url} | tar xz && ./ninari-*/install.sh`;
    const meta = document.querySelector("[data-release-meta]");
    if (meta) meta.textContent = `Версия ${version} · Linux и Windows · бесплатно, открытый код`;
    const date = rel.published_at
      ? new Date(rel.published_at).toLocaleDateString("ru-RU", { day: "numeric", month: "long", year: "numeric" })
      : "";
    const line = document.querySelector("[data-release-line]");
    if (line) line.textContent = `Версия ${version}${date ? ` от ${date}` : ""}, архив ${mb} МБ.`;
    const notes = document.querySelector("[data-release-notes]");
    if (notes && rel.body) {
      notes.querySelector("[data-release-version]").textContent = version;
      notes.querySelector("[data-release-body]").textContent = rel.body;
      notes.hidden = false;
    }
  } catch {
    // The buttons keep pointing at the releases page.
  }
}

setupReveal();
setupTilt();
setupFaq();
setupNav();
setupToc();
setupCopy();
setupInstallAll();
loadPlugins();
loadRelease();

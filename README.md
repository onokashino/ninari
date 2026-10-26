<div align="center">

<img src="site/assets/mark.svg" width="96" height="96" alt="" />

# ninari

**Аниме из всех озвучек в одном окне**

Каталог Shikimori, серии из разных источников с лучшим качеством каждой озвучки,
плеер mpv с улучшением картинки Anime4K и ArtCNN, прогресс, который не теряется.

[![Релиз](https://img.shields.io/github/v/release/onokashino/ninari?label=%D1%80%D0%B5%D0%BB%D0%B8%D0%B7&color=b9a3ff)](https://github.com/onokashino/ninari/releases/latest)
[![Лицензия](https://img.shields.io/github/license/onokashino/ninari?label=%D0%BB%D0%B8%D1%86%D0%B5%D0%BD%D0%B7%D0%B8%D1%8F&color=e2475a)](LICENSE)
[![Сайт](https://img.shields.io/badge/%D1%81%D0%B0%D0%B9%D1%82-onokashino.github.io%2Fninari-101117)](https://onokashino.github.io/ninari/)

[Сайт](https://onokashino.github.io/ninari/) ·
[Скачать](https://onokashino.github.io/ninari/download/) ·
[Документация](https://onokashino.github.io/ninari/docs/) ·
[Плагины](https://onokashino.github.io/ninari/plugins/) ·
[English](README.en.md)

<img src="site/assets/shots/title.webp" alt="Страница тайтла в ninari: сезоны, карточка «Дальше» и серии" />

</div>

## Возможности

- **Все озвучки сразу.** Приложение спрашивает каждый подключённый источник и сводит одну команду озвучки с разных сайтов в одну строку с лучшим качеством.
- **Плеер mpv.** Улучшение картинки Anime4K и ArtCNN прямо во время просмотра, ровная громкость и чёткие голоса, пропуск опенингов, эндингов и филлеров.
- **Каталог Shikimori.** Подборки, жанры, студии и сезоны, сезоны франшизы карточками, поиск по кадру.
- **Ничего не теряется.** «Продолжить» с той же минуты, уведомления о новых сериях, синхронизация с Shikimori и между своими компьютерами в домашней сети.

## Установка

Linux (Arch, openSUSE Tumbleweed и другие системы со свежими библиотеками), mpv и всё нужное уже внутри:

```bash
curl -L https://github.com/onokashino/ninari/releases/download/v0.1.5/ninari-0.1.5-linux-x86_64.tar.gz | tar xz && ./ninari-*/install.sh
```

Для Windows 10 и 11 на [странице релиза](https://github.com/onokashino/ninari/releases/latest) есть установщик и portable-zip. Flatpak для любого дистрибутива и AUR скоро, macOS в работе. Подробнее: [установка](https://onokashino.github.io/ninari/docs/install/).

## Источники серий

ninari не хранит и не раздаёт видео. Серии находят плагины в папке [`extensions`](extensions): каждый работает в песочнице и ходит только на свои сайты. Написать свой несложно: [свой источник](https://onokashino.github.io/ninari/docs/custom-source/).

## Разработка

Rust и GPUI (фреймворк интерфейса редактора Zed), видео через libmpv.

```bash
bun shaders   # один раз: шейдеры улучшения качества
bun gpui      # собрать и запустить
```

| Папка | Что внутри |
| --- | --- |
| `crates/core` | Всё, кроме окна: каталог, библиотека, источники, плеер, синхронизация |
| `crates/app` | Приложение на GPUI |
| `extensions` | Официальные плагины-источники |
| `site` | Сайт и документация на русском и английском |

Подробнее: [сборка](https://onokashino.github.io/ninari/docs/build/), [устройство проекта](https://onokashino.github.io/ninari/docs/architecture/), [выпуск версии](https://onokashino.github.io/ninari/docs/releasing/).

## Лицензия

[GPL-3.0](LICENSE)

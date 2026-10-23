---
title: Сборка из исходников
description: Как собрать и запустить ninari из исходников на Linux.
section: dev
---
# Сборка из исходников

## Что нужно

- Rust (stable).
- mpv с libmpv из пакетов системы.
- bun, для коротких команд и плагинов.

## Запуск

```bash
git clone https://github.com/onokashino/ninari
cd ninari
bun shaders          # один раз: шейдеры улучшения качества
bun gpui             # собрать и запустить
```

| Команда | Что делает |
| --- | --- |
| `bun gpui` | Собрать и запустить. `--page title:<id>` откроет тайтл |
| `bun play` | Сразу в серию с карточки «Продолжить» |
| `bun shot out.png catalog` | Нарисовать страницу в PNG без окна |
| `bun plugins` | Собрать плагины в `extensions/dist` |
| `bun plugins:test` | Проверить плагины на живых источниках |

`scripts/dev` запускает всё с ограничением памяти и без свопа.

## Тесты

```bash
cd crates/core && cargo test
cd crates/app && cargo test
```

## Ключи Shikimori

Вход в Shikimori требует ключей приложения при сборке. Создай `.env.local` в корне проекта:

```bash
SHIKIMORI_CLIENT_ID=...
SHIKIMORI_CLIENT_SECRET=...
```

Файл в `.gitignore`. Приложение на Shikimori регистрируется с адресом возврата `http://127.0.0.1:47831/oauth/shikimori` и правом `user_rates`.

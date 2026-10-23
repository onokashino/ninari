---
title: Выпуск версии
description: Как выпустить новую версию ninari: тег, сборка на GitHub и обновление у пользователей.
section: dev
---
# Выпуск версии

1. Подними `version` в `crates/app/Cargo.toml` и закоммить.
2. Поставь тег с заметкой, которую увидят пользователи, и отправь его:

```bash
git tag -a v0.2.0 -m "Что нового, как это покажет приложение"
git push origin master v0.2.0
```

Workflow `.github/workflows/release.yml` соберёт архив в контейнере Arch Linux (`scripts/package.sh`) и опубликует релиз вместе с `latest.json`. Установленные приложения читают этот файл и предлагают обновиться.

Тег должен совпадать с версией в `Cargo.toml`, иначе сборка остановится. Ключи Shikimori берутся из секретов репозитория `SHIKIMORI_CLIENT_ID` и `SHIKIMORI_CLIENT_SECRET`.

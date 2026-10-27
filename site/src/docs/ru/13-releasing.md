---
title: Выпуск версии
description: Как выпустить новую версию ninari: тег, сборка на GitHub и обновление у пользователей.
section: dev
---
# Выпуск версии

Номер версии живёт в одном месте: `version` в `crates/app/Cargo.toml`. Команды установки на сайте и в README ведут на последний релиз и версии не содержат.

1. Подними `version` в `crates/app/Cargo.toml` и закоммить вместе с `Cargo.lock`.
2. Поставь тег с заметкой, которую увидят пользователи, и отправь его:

```bash
git tag -a v0.2.0 -m "Что нового, как это покажет приложение"
git push origin master v0.2.0
```

Workflow `.github/workflows/release.yml` соберёт архив для Linux в контейнере Arch Linux (`scripts/package.sh`) и zip с установщиком для Windows (`scripts/package.ps1`, Inno Setup), а затем опубликует релиз вместе с `latest.json`:

```json
{
  "version": "0.2.0",
  "notes": "сообщение тега",
  "platforms": {
    "linux-x86_64": { "url": "…/ninari-0.2.0-linux-x86_64.tar.gz", "sha256": "…" },
    "windows-x86_64": { "url": "…/ninari-0.2.0-windows-x86_64-setup.exe", "sha256": "…" }
  }
}
```

Установленные приложения читают этот файл, предлагают обновиться, скачивают пакет своей системы и сверяют контрольную сумму. На Linux они распаковывают архив и запускают его `install.sh`, на Windows тихо запускают установщик. Потом приложение перезапускается.

Каждый push в master, который трогает приложение, тоже собирает обе системы. Пакеты лежат в артефактах запуска, релиз при этом не создаётся.

Тег должен совпадать с версией в `Cargo.toml`, иначе сборка остановится. Ключи Shikimori берутся из секретов репозитория `SHIKIMORI_CLIENT_ID` и `SHIKIMORI_CLIENT_SECRET`.

После выхода релиза пакет AUR обновляется скриптом `packaging/aur/update.sh` (см. `packaging/aur/README.md`).

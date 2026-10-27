---
title: Установка
description: Как установить ninari на Linux и Windows, обновить и удалить.
section: guide
---
# Установка

ninari выходит для Linux и Windows 10 и 11. Версия для macOS в работе.

## Windows

На [странице релиза](https://github.com/onokashino/ninari/releases/latest) два файла:

- `ninari-<версия>-windows-x86_64-setup.exe`: установщик. Права администратора не нужны, ставится в `%LOCALAPPDATA%\Programs\ninari`, добавляет ярлык в меню «Пуск».
- `ninari-<версия>-windows-x86_64.zip`: portable-версия, распакуй и запусти `ninari.exe`.

Установщик пока не подписан, поэтому SmartScreen может предупредить: «Подробнее» → «Выполнить в любом случае».

## Linux одной командой

Архив собран на Arch и требует свежих системных библиотек (glibc 2.44 или новее): подойдут Arch, openSUSE Tumbleweed и похожие дистрибутивы.

```bash
curl -L https://github.com/onokashino/ninari/releases/download/v0.1.8/ninari-0.1.8-linux-x86_64.tar.gz | tar xz && ./ninari-*/install.sh
```

Команда скачивает последний релиз, распаковывает его в текущую папку и запускает установщик. Актуальная команда всегда есть на странице [Скачать](../download.html).

## Linux вручную

1. Скачай `ninari-<версия>-linux-x86_64.tar.gz` со [страницы релизов](https://github.com/onokashino/ninari/releases).
2. Распакуй и запусти `./install.sh` внутри.
3. ninari появится в меню приложений, а в терминале станет командой `ninari`.

Пароль не нужен: всё ставится в твою домашнюю папку.

## Что куда ставится

| Что | Где |
| --- | --- |
| Программа, свой mpv и библиотеки | `~/.local/share/ninari` |
| Команда `ninari` | `~/.local/bin/ninari` |
| Пункт в меню и иконки | `~/.local/share/applications`, `~/.local/share/icons` |
| Библиотека, прогресс, настройки | `~/.local/share/com.onokashino.ninari` |
| Постеры и превью | `~/.cache/com.onokashino.ninari` |

На Windows библиотека и настройки лежат в `%APPDATA%\com.onokashino.ninari`, постеры и превью в `%LOCALAPPDATA%\com.onokashino.ninari`.

Внутри архива уже есть mpv и все библиотеки, которые ему нужны. Системный mpv не требуется и не трогается. Из системы берутся только glibc, драйвер видеокарты, X11 или Wayland и звук.

## Обновление

Когда выходит новая версия, ninari покажет это в углу окна. Нажми «Обновить»: приложение скачает её, проверит контрольную сумму, установит поверх и перезапустится. Библиотека и настройки останутся. Проверить вручную можно в **Настройки → Приложение**.

## Удаление

На Windows: «Параметры → Приложения» или пункт удаления в меню «Пуск». На Linux:

```bash
./install.sh --remove
```

Удаляются программа, команда, пункт меню и иконки. Библиотеку и настройки в `~/.local/share/com.onokashino.ninari` это не трогает: удали папку сам, если она больше не нужна.

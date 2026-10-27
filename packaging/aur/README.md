# AUR

`ninari-bin` ставит готовый релиз из GitHub: программа в `/opt/ninari`, команда `ninari`, ярлык и
иконки. Приложение видит, что его поставил пакетный менеджер, и само не обновляется.

Первый раз: завести аккаунт на aur.archlinux.org, добавить туда SSH-ключ, затем

```bash
git clone ssh://aur@aur.archlinux.org/ninari-bin.git aur-ninari-bin
cp packaging/aur/ninari-bin/{PKGBUILD,.SRCINFO} aur-ninari-bin/
cd aur-ninari-bin && git add -A && git commit -m "ninari 0.1.8" && git push
```

На каждую версию: поменять `pkgver` и `sha256sums` (сумма есть в `latest.json` релиза), затем
`makepkg --printsrcinfo > .SRCINFO` и тот же push.

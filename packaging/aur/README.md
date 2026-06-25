# Publishing StorageSifter to the AUR

Two packages are provided:

- **`storagesifter-bin`** — installs the prebuilt binary from the GitHub
  Release. Fast to install; recommended for most users.
- **`storagesifter`** — builds from source with `cargo`. No binary trust
  needed, but pulls the full GUI dependency tree and takes a while to compile.

Publish whichever you want (you can publish both). You'll need an
[AUR account](https://aur.archlinux.org) with an SSH key registered.

## One-time per package

For **v0.1.0 the `sha256sums` and `.SRCINFO` are already filled in and committed**
(generated against the live release), so the only thing you *must* do before
pushing is set the real `Maintainer:` email. The full flow:

```sh
# 0. Set your Maintainer email at the top of the PKGBUILD (replaces CHANGE_ME).

# 1. (Optional) test the build locally:
cd packaging/aur/storagesifter-bin       # or .../storagesifter
makepkg -si                              # builds + installs to verify

# 2. Push to the AUR (the AUR repo name is the pkgname):
git clone ssh://aur@aur.archlinux.org/storagesifter-bin.git aur-bin
cp PKGBUILD .SRCINFO aur-bin/
cd aur-bin && git add PKGBUILD .SRCINFO && git commit -m "Initial import: 0.1.0" && git push
```

(If you edit the PKGBUILD after this, re-run `updpkgsums` and
`makepkg --printsrcinfo > .SRCINFO` so they stay in sync.)

## On each new release

1. Bump `pkgver` (and reset `pkgrel=1`) in the PKGBUILD.
2. `updpkgsums` to refresh `sha256sums`.
3. `makepkg --printsrcinfo > .SRCINFO`.
4. Commit + push to the AUR repo.

## Notes

- Set the `Maintainer:` line to your real name/email before publishing.
- `makedepends=('cargo')` is satisfied by either the `rust` or `rustup` package.
- If `namcap` complains about `sha256sums=('SKIP')`, that's expected until you
  run `updpkgsums`; the AUR prefers real checksums.

# Publishing StorageSifter to the AUR

Two packages are provided:

- **`storagesifter-bin`** — installs the prebuilt binary from the GitHub
  Release. Fast to install; recommended for most users.
- **`storagesifter`** — builds from source with `cargo`. No binary trust
  needed, but pulls the full GUI dependency tree and takes a while to compile.

Publish whichever you want (you can publish both). You'll need an
[AUR account](https://aur.archlinux.org) with an SSH key registered.

## One-time per package

```sh
# 1. Cut the GitHub release first (push a tag; CI builds the artifacts).
#    The -bin package downloads from that release, so it must exist.

# 2. Fill in the real checksum (replaces the SKIP placeholder):
cd packaging/aur/storagesifter-bin      # or .../storagesifter
updpkgsums

# 3. Sanity-check the build in a clean chroot if you can:
makepkg -si                              # builds + installs locally to test

# 4. Generate the AUR metadata file:
makepkg --printsrcinfo > .SRCINFO

# 5. Push to the AUR (the AUR repo name is the pkgname):
git clone ssh://aur@aur.archlinux.org/storagesifter-bin.git aur-bin
cp PKGBUILD .SRCINFO aur-bin/
cd aur-bin && git add PKGBUILD .SRCINFO && git commit -m "Initial import: 0.1.0" && git push
```

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

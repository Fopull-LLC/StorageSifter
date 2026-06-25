#!/usr/bin/env bash
# Build a StorageSifter AppImage from an already-compiled release binary.
#
# Usage:
#   cargo build --release -p storagesifter
#   packaging/appimage/build-appimage.sh
#
# Env overrides:
#   BINARY  path to the compiled binary (default: target/release/storagesifter)
#   OUTDIR  where to write the .AppImage    (default: dist)
#   VERSION version string baked into the filename (default: git describe)
#
# Relies on the host providing graphics libraries at runtime (Vulkan loader,
# Wayland/X11, libxkbcommon) — standard for AppImages. To maximize glibc
# compatibility, build on the oldest distro you intend to support (CI uses an
# older Ubuntu for exactly this reason).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"

BINARY="${BINARY:-$repo/target/release/storagesifter}"
OUTDIR="${OUTDIR:-$repo/dist}"
VERSION="${VERSION:-$(git -C "$repo" describe --tags --always 2>/dev/null || echo dev)}"
APPID="com.fopull.StorageSifter"
ARCH="${ARCH:-x86_64}"

if [[ ! -x "$BINARY" ]]; then
  echo "error: binary not found at $BINARY — run 'cargo build --release -p storagesifter' first" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
appdir="$work/StorageSifter.AppDir"

# Standard AppDir layout.
install -Dm755 "$BINARY"                                   "$appdir/usr/bin/storagesifter"
install -Dm644 "$repo/packaging/$APPID.desktop"            "$appdir/usr/share/applications/$APPID.desktop"
install -Dm644 "$repo/packaging/$APPID.metainfo.xml"       "$appdir/usr/share/metainfo/$APPID.metainfo.xml"
install -Dm644 "$repo/assets/icons/storagesifter.png"      "$appdir/usr/share/icons/hicolor/256x256/apps/$APPID.png"

# appimagetool expects a desktop file and icon at the AppDir root too.
cp "$appdir/usr/share/applications/$APPID.desktop" "$appdir/$APPID.desktop"
cp "$repo/assets/icons/storagesifter.png"          "$appdir/$APPID.png"

# Entry point.
cat > "$appdir/AppRun" <<'EOF'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "${0}")")"
exec "$HERE/usr/bin/storagesifter" "$@"
EOF
chmod +x "$appdir/AppRun"

# Fetch appimagetool if it isn't already available.
tool="$(command -v appimagetool || true)"
if [[ -z "$tool" ]]; then
  tool="$work/appimagetool"
  echo "Fetching appimagetool…"
  curl -fsSL -o "$tool" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${ARCH}.AppImage"
  chmod +x "$tool"
fi

mkdir -p "$OUTDIR"
out="$OUTDIR/StorageSifter-${VERSION}-${ARCH}.AppImage"

# --appimage-extract-and-run lets appimagetool work without FUSE (e.g. in CI).
ARCH="$ARCH" "$tool" --appimage-extract-and-run "$appdir" "$out"
echo "Built $out"

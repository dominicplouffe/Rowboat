#!/usr/bin/env bash
# Validates packaging/arch/PKGBUILD the way an Arch user would experience it:
# build the package with makepkg from a real .deb, install it with pacman
# (which proves every dependency resolves), and check the installed binary.
#
#   packaging/arch/validate.sh path/to/Rowboat_<version>_amd64.deb [out-dir]
#
# Meant for an `archlinux:base-devel` container, as root (see ci.yml and
# release.yml). Writes the PKGBUILD with real checksums and its .SRCINFO, i.e.
# exactly what gets pushed to the AUR, to out-dir.
set -euo pipefail

deb=$(realpath "${1:?usage: validate.sh <deb> [out-dir]}")
out=$(realpath -m "${2:-arch-out}")
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)

version=$(sed -n 's/^_upstreamver=//p' "$here/PKGBUILD")
echo "==> Validating rowboat-sql-bin $version against $(basename "$deb")"

pacman -Syu --noconfirm --needed namcap >/dev/null

# makepkg refuses to run as root.
id builder >/dev/null 2>&1 || useradd --create-home builder
work=$(mktemp -d)
cp "$here/PKGBUILD" "$work/PKGBUILD"
# Sources already present next to the PKGBUILD are used instead of downloaded,
# so this works before the release exists.
cp "$deb" "$work/Rowboat_${version}_amd64.deb"
cp "$repo/LICENSE" "$work/LICENSE-${version}"

deb_sum=$(sha256sum "$work/Rowboat_${version}_amd64.deb" | cut -d' ' -f1)
license_sum=$(sha256sum "$work/LICENSE-${version}" | cut -d' ' -f1)
sed -i "/^sha256sums=/,/)/c\\sha256sums=('${deb_sum}'\n            '${license_sum}')" "$work/PKGBUILD"
grep -q "$deb_sum" "$work/PKGBUILD"

chown -R builder "$work"
echo "==> makepkg"
su builder -c "cd '$work' && makepkg --nodeps --noconfirm && makepkg --printsrcinfo > .SRCINFO"

echo "==> pacman -U (resolves and installs dependencies)"
pacman -U --noconfirm "$work"/rowboat-sql-bin-*.pkg.tar.*

echo "==> Checking the installed package"
test -x /usr/bin/rowboat
test -f /usr/share/applications/Rowboat.desktop
test -f /usr/share/licenses/rowboat-sql-bin/LICENSE
if ldd /usr/bin/rowboat | grep 'not found'; then
  echo 'FAIL: the binary has unresolved shared libraries; add them to depends=().' >&2
  exit 1
fi

# Advisory only. namcap flags every library the binary links that is not named
# in depends=(), even when a listed package already pulls it in (gtk3 brings
# pango, for example). What must hold is checked above: the package builds,
# pacman resolves its dependencies, and nothing is left unresolved.
echo "==> namcap (advisory)"
namcap "$work/PKGBUILD" || true
namcap "$work"/rowboat-sql-bin-*.pkg.tar.* || true

mkdir -p "$out"
cp "$work/PKGBUILD" "$work/.SRCINFO" "$out/"
echo "==> OK: PKGBUILD and .SRCINFO written to $out"

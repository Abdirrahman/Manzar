#!/bin/sh
set -eu

if ! command -v pacman >/dev/null 2>&1; then
  echo "error: pacman was not found. This installer is only for Arch Linux or Arch-based systems." >&2
  exit 1
fi

if [ "$(id -u)" -eq 0 ]; then
  echo "error: do not run this script as root. It uses sudo for pacman and runs makepkg as your user." >&2
  exit 1
fi

if [ -r /etc/os-release ] && ! grep -Eq '^(ID|ID_LIKE)=.*arch' /etc/os-release; then
  echo "warning: /etc/os-release does not look Arch-based, but pacman exists; continuing." >&2
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

if [ ! -f "$REPO_ROOT/PKGBUILD" ]; then
  echo "error: PKGBUILD not found at repository root: $REPO_ROOT" >&2
  exit 1
fi

if ! command -v sudo >/dev/null 2>&1; then
  echo "error: sudo was not found. Install base-devel and git manually, then run makepkg -si as a non-root user." >&2
  exit 1
fi

sudo pacman -S --needed base-devel git

mkdir -p "$REPO_ROOT/.makepkg/build"
mkdir -p "$REPO_ROOT/.makepkg/packages"
mkdir -p "$REPO_ROOT/.makepkg/sources"
mkdir -p "$REPO_ROOT/.makepkg/srcpackages"

cd "$REPO_ROOT"
BUILDDIR="$REPO_ROOT/.makepkg/build" \
PKGDEST="$REPO_ROOT/.makepkg/packages" \
SRCDEST="$REPO_ROOT/.makepkg/sources" \
SRCPKGDEST="$REPO_ROOT/.makepkg/srcpackages" \
makepkg -si

echo "Manzar installed through pacman."
echo "To uninstall: sudo pacman -Rns manzar"

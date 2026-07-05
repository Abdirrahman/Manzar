# Maintainer: Manzar contributors

pkgname=manzar
pkgver=0.1.1
pkgrel=1
pkgdesc='A simple local image viewer'
arch=('x86_64')
url='https://github.com/Abdirrahman/Manzar'
license=('MIT')
depends=(
  'webkit2gtk-4.1'
  'gtk3'
  'openssl'
  'appmenu-gtk-module'
  'libappindicator'
  'librsvg'
  'xdotool'
  'hicolor-icon-theme'
)
makedepends=(
  'bun'
  'rust'
  'cargo'
  'curl'
  'wget'
  'file'
  'git'
)
source=("$pkgname-$pkgver.tar.gz::https://github.com/Abdirrahman/Manzar/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('16af1b84e63e61626a1809b26d22b1a579ca0b31dcab228bc569fa2f32756d62')

build() {
  cd "Manzar-$pkgver"

  bun install --frozen-lockfile
  bun run tauri build --no-bundle
}

package() {
  cd "Manzar-$pkgver"

  install -Dm755 "src-tauri/target/release/manzar" "$pkgdir/usr/bin/manzar"
  install -Dm644 /dev/stdin "$pkgdir/usr/share/applications/manzar.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Manzar
Comment=A simple local image viewer
Exec=manzar %F
Icon=manzar
Terminal=false
Categories=Graphics;Viewer;
MimeType=image/png;image/jpeg;image/webp;image/gif;image/bmp;
EOF
  install -Dm644 "src-tauri/icons/32x32.png" "$pkgdir/usr/share/icons/hicolor/32x32/apps/manzar.png"
  install -Dm644 "src-tauri/icons/128x128.png" "$pkgdir/usr/share/icons/hicolor/128x128/apps/manzar.png"
  install -Dm644 "src-tauri/icons/128x128@2x.png" "$pkgdir/usr/share/icons/hicolor/256x256/apps/manzar.png"
  install -Dm644 "src-tauri/icons/icon.png" "$pkgdir/usr/share/icons/hicolor/512x512/apps/manzar.png"
  install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}

#!/usr/bin/env bash
# Builds the standalone releases into release/:
#   release/Aetherhub-<version>-x86_64.AppImage   (Linux)
#   release/Aetherhub-<version>-windows-x64.zip    (Windows: unzip, run Aetherhub.exe)
#
# Needs: Node/npm (the repo's dev dependencies installed with `npm ci`), Docker or Podman, curl, unzip, zip.
# Usage: packaging/build.sh [linux|windows|all]   (default: all)
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$PWD
TARGET=${1:-all}
VERSION=$(node -p "require('./package.json').version")
NODE_MAJOR=${NODE_MAJOR:-24}
OUT=$ROOT/release
CACHE=$OUT/.cache
STAGE=$OUT/.stage
mkdir -p "$CACHE"
rm -rf "$STAGE"
mkdir -p "$STAGE"

echo "== Building the web UI and the server"
npm run build

echo "== Node.js $NODE_MAJOR runtime"
NODE_VERSION=$(curl -fsSL https://nodejs.org/dist/index.json | node -e "
  let s='';process.stdin.on('data',d=>s+=d).on('end',()=>{
    const v=JSON.parse(s).find(r=>r.version.startsWith('v$NODE_MAJOR.'));process.stdout.write(v.version)})")
echo "   $NODE_VERSION"
curl -fsSL "https://nodejs.org/dist/$NODE_VERSION/SHASUMS256.txt" -o "$CACHE/SHASUMS256-$NODE_VERSION.txt"
echo "== Building the rules engine and the launcher (Linux + Windows) in a container"
docker build -f packaging/Dockerfile --build-arg "NODE_VERSION=$NODE_VERSION" --output "type=local,dest=$STAGE/native" .

fetch_node() { # <file name>
  local f=$1
  if [ ! -f "$CACHE/$f" ]; then curl -fL "https://nodejs.org/dist/$NODE_VERSION/$f" -o "$CACHE/$f.part" && mv "$CACHE/$f.part" "$CACHE/$f"; fi
  (cd "$CACHE" && grep " $f\$" "SHASUMS256-$NODE_VERSION.txt" | sha256sum -c --quiet -)
}

# The app folder shared by both platforms: bundled server, built client, card definitions (scenario tests left out).
stage_app() { # <dest>
  local d=$1
  mkdir -p "$d/server" "$d/native"
  cp packages/server/dist/index.js packages/server/dist/cli.js "$d/server/"
  cp -r packages/client/dist "$d/client"
  mkdir -p "$d/cards"
  (cd cards && find . -mindepth 1 -maxdepth 1 -type d ! -name tests -exec cp -r {} "$d/cards/" \;)
  # runtime dependencies at the exact versions the repo is built and tested with
  node -e "
    const srv=require('./packages/server/package.json');
    const lock=require('./package-lock.json').packages;
    const deps={};
    for (const n of Object.keys(srv.dependencies)) {
      if (n.startsWith('@aetherhub/')) continue;
      const e=lock['packages/server/node_modules/'+n]||lock['node_modules/'+n];
      if (!e) throw new Error('not in package-lock.json: '+n);
      deps[n]=e.version;
    }
    require('fs').writeFileSync('$d/package.json', JSON.stringify({name:'aetherhub',version:'$VERSION',private:true,type:'module',dependencies:deps},null,2));"
}

install_deps() { # <dest> <npm platform flags...>
  local d=$1; shift
  (cd "$d" && npm install --omit=dev --no-audit --no-fund --no-package-lock "$@" >/dev/null)
}

build_linux() {
  echo "== Linux AppImage"
  local dir=$STAGE/Aetherhub.AppDir
  stage_app "$dir/app"
  install_deps "$dir/app" --os=linux --cpu=x64 --libc=glibc
  cp "$STAGE/native/linux/libmtg_node.so" "$dir/app/native/"
  local nf=node-$NODE_VERSION-linux-x64.tar.xz
  fetch_node "$nf"
  mkdir -p "$dir/node"
  tar -xJf "$CACHE/$nf" -C "$dir/node" --strip-components=1 "node-$NODE_VERSION-linux-x64/bin/node" "node-$NODE_VERSION-linux-x64/LICENSE"
  cp "$STAGE/native/linux/aetherhub" "$dir/AppRun"
  chmod +x "$dir/AppRun"
  cp LICENSE "$dir/LICENSE"
  cp packages/client/public/icon-512.png "$dir/aetherhub.png"
  cat > "$dir/aetherhub.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Aetherhub
Comment=Self-hosted Magic: The Gathering deckbuilder and tabletop
Exec=AppRun
Icon=aetherhub
Terminal=true
Categories=Game;CardGame;
EOF
  local tool=$CACHE/appimagetool-x86_64.AppImage
  if [ ! -f "$tool" ]; then
    curl -fL https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage -o "$tool.part" && mv "$tool.part" "$tool"
    chmod +x "$tool"
  fi
  ARCH=x86_64 "$tool" --appimage-extract-and-run --no-appstream "$dir" "$OUT/Aetherhub-$VERSION-x86_64.AppImage" >/dev/null
  echo "   $OUT/Aetherhub-$VERSION-x86_64.AppImage"
}

build_windows() {
  echo "== Windows zip"
  local dir=$STAGE/Aetherhub
  stage_app "$dir/app"
  install_deps "$dir/app" --os=win32 --cpu=x64
  cp "$STAGE/native/windows/mtg_node.dll" "$dir/app/native/"
  local nf=node-$NODE_VERSION-win-x64.zip
  fetch_node "$nf"
  mkdir -p "$dir/node"
  unzip -q -j "$CACHE/$nf" "node-$NODE_VERSION-win-x64/node.exe" "node-$NODE_VERSION-win-x64/LICENSE" -d "$dir/node"
  cp "$STAGE/native/windows/aetherhub.exe" "$dir/Aetherhub.exe"
  cp LICENSE "$dir/LICENSE.txt"
  local zipf=$OUT/Aetherhub-$VERSION-windows-x64.zip
  rm -f "$zipf"
  (cd "$STAGE" && zip -qr "$zipf" Aetherhub)
  echo "   $zipf"
}

case $TARGET in
  linux) build_linux ;;
  windows) build_windows ;;
  all) build_linux; build_windows ;;
  *) echo "usage: $0 [linux|windows|all]" >&2; exit 2 ;;
esac
rm -rf "$STAGE"

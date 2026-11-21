#!/bin/sh
# Builds the Linux release: Wraith-Linux-<version>-x64.zip (wraith + Xray core + geo files) and
# wraith-setup-<version>-x64 (the installer with that archive embedded).
#   ./package.sh            uses the Xray version below
#   XRAY_VERSION=v26.11.9 ./package.sh
set -eu
cd "$(dirname "$0")"
XRAY_VERSION="${XRAY_VERSION:-v26.11.9}"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
NAME="Wraith-Linux-$VERSION-x64"
OUT=dist; WORK="$OUT/work"
rm -rf "$WORK"; mkdir -p "$WORK/$NAME" "$WORK/xray"

echo "== Xray $XRAY_VERSION"
base="https://github.com/XTLS/Xray-core/releases/download/$XRAY_VERSION"
curl -fsSL -o "$WORK/xray.zip" "$base/Xray-linux-64.zip"
curl -fsSL -o "$WORK/xray.dgst" "$base/Xray-linux-64.zip.dgst"
want=$(sed -n 's/^SHA2-256= *//p' "$WORK/xray.dgst")
have=$(sha256sum "$WORK/xray.zip" | cut -d' ' -f1)
[ -n "$want" ] && [ "$want" = "$have" ] || { echo "Xray SHA-256 mismatch" >&2; exit 1; }
python3 -I -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$WORK/xray.zip" "$WORK/xray"

echo "== wraith $VERSION"
cargo build --release
cp target/release/wraith "$WORK/$NAME/wraith"
cp "$WORK/xray/xray" "$WORK/xray/geoip.dat" "$WORK/xray/geosite.dat" "$WORK/$NAME/"
cp "$WORK/xray/LICENSE" "$WORK/$NAME/LICENSE-Xray.txt"
cp LICENSE THIRD_PARTY_NOTICES.md README.md packaging/wraith.desktop packaging/wraith.png "$WORK/$NAME/"
chmod 755 "$WORK/$NAME/wraith" "$WORK/$NAME/xray"

# zip with Unix permissions (the updater and the installer keep the executable bits).
python3 -I - "$WORK" "$NAME" "$OUT/$NAME.zip" <<'EOF'
import os, sys, zipfile
work, name, out = sys.argv[1:]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for root, _, files in os.walk(os.path.join(work, name)):
        for f in sorted(files):
            path = os.path.join(root, f)
            z.write(path, os.path.relpath(path, work))
EOF

echo "== wraith-setup"
WRAITH_PAYLOAD="$PWD/$OUT/$NAME.zip" cargo build --release --features setup --target-dir target/setup
cp target/setup/release/wraith "$OUT/wraith-setup-$VERSION-x64"
chmod 755 "$OUT/wraith-setup-$VERSION-x64"
rm -rf "$WORK"
( cd "$OUT" && sha256sum "$NAME.zip" "wraith-setup-$VERSION-x64" )

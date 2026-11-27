#!/bin/bash
set -euo pipefail

if (( $# > 1 )); then
    echo "Usage: $0 [release-tag]" >&2
    exit 1
fi

cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=14.0
target=aarch64-apple-darwin
metadata=$(cargo metadata --locked --no-deps --format-version 1 | python3 -c '
import json
import sys

metadata = json.load(sys.stdin)
print(next(package["version"] for package in metadata["packages"] if package["name"] == "rshot-cli"))
print(metadata["target_directory"])
')
version=${metadata%%$'\n'*}
target_dir=${metadata#*$'\n'}

if [[ ${1:-v$version} != "v$version" ]]; then
    echo "Release tag $1 does not match Cargo version v$version." >&2
    exit 1
fi

for license in LICENSE-MIT LICENSE-APACHE; do
    if [[ ! -f $license ]]; then
        echo "Missing required license: $license" >&2
        exit 1
    fi
done
licenses=(LICENSE-*)

cargo build --release --locked --target "$target" -p rshot-cli -p rshot-gui
build_dir="$target_dir/$target/release"
for binary in rshot-cli rshot; do
    minimum_version=$(otool -l "$build_dir/$binary" | awk '
        $1 == "cmd" { build_version = ($2 == "LC_BUILD_VERSION") }
        build_version && $1 == "minos" { print $2 }
    ')
    if [[ $minimum_version != "$MACOSX_DEPLOYMENT_TARGET" ]]; then
        echo "$binary must target macOS $MACOSX_DEPLOYMENT_TARGET (found: ${minimum_version:-none})." >&2
        exit 1
    fi
done

mkdir -p dist
staging=$(mktemp -d "$PWD/dist/.package.XXXXXX")
trap 'rm -rf -- "$staging"' EXIT
app="$staging/Rshot.app"
cli="$staging/cli"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$cli"
install -m 755 "$build_dir/rshot" "$app/Contents/MacOS/rshot"
install -m 755 "$build_dir/rshot-cli" "$cli/rshot-cli"
cp "${licenses[@]}" "$app/Contents/Resources/"
cp README.md "${licenses[@]}" "$cli/"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>dev.rshot.Rshot</string>
    <key>CFBundleName</key>
    <string>Rshot</string>
    <key>CFBundleExecutable</key>
    <string>rshot</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>LSMinimumSystemVersion</key>
    <string>$MACOSX_DEPLOYMENT_TARGET</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

codesign --force --deep -s - "$app"
codesign --force -s - "$cli/rshot-cli"
codesign --verify --deep --strict "$app"
codesign --verify --strict "$cli/rshot-cli"

cli_archive="rshot-cli-$version-$target.tar.gz"
app_archive="Rshot-$version-$target.zip"
tar -czf "$staging/$cli_archive" -C "$cli" rshot-cli README.md "${licenses[@]}"
ditto -c -k --keepParent "$app" "$staging/$app_archive"
(
    cd "$staging"
    shasum -a 256 "$cli_archive" "$app_archive" > SHA256SUMS
)
mv "$staging/$cli_archive" "$staging/$app_archive" "$staging/SHA256SUMS" dist/
printf 'Packaged %s and %s in dist/\n' "$cli_archive" "$app_archive"

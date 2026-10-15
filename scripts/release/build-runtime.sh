#!/usr/bin/env bash
# Build the relocatable macOS runtime. Run only on an Apple Silicon Mac.
set -euo pipefail
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]]
APP="dist/ytb2pdf-app.app"
BIN="$APP/Contents/Resources/bin"
mkdir -p "$APP/Contents/MacOS" "$BIN" "$APP/Contents/Frameworks"
cp target/release/ytb2pdf-app target/release/ytb2pdf-cli "$APP/Contents/MacOS/"

# yt-dlp's standalone build includes its Python runtime.
curl --fail --location --retry 3 -o "$BIN/yt-dlp" \
  https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos
for tool in ffmpeg ffprobe pdfunite pdfinfo deno; do
  cp -L "$(command -v "$tool")" "$BIN/$tool"
done

WHISPER_SOURCE="$RUNNER_TEMP/ytb2pdf-whisper"
git clone --depth 1 --branch v1.8.2 https://github.com/ggml-org/whisper.cpp "$WHISPER_SOURCE"
cmake -S "$WHISPER_SOURCE" -B "$WHISPER_SOURCE/build" \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0 \
  -DBUILD_SHARED_LIBS=OFF -DGGML_METAL_EMBED_LIBRARY=ON -DWHISPER_BUILD_TESTS=OFF
cmake --build "$WHISPER_SOURCE/build" --parallel 3 --target whisper-cli
cp "$WHISPER_SOURCE/build/bin/whisper-cli" "$BIN/whisper-cli"
chmod +x "$BIN/"*

# Copy Homebrew dylibs transitively and rewrite their references. Every media
# tool is in Resources/bin, so this relative executable path works after moving
# the app and also in the CLI archive's matching Contents layout.
dylibbundler -od -b \
  -x "$BIN/ffmpeg" -x "$BIN/ffprobe" -x "$BIN/pdfunite" -x "$BIN/pdfinfo" \
  -x "$BIN/deno" -x "$BIN/whisper-cli" \
  -d "$APP/Contents/Frameworks" -p '@executable_path/../../Frameworks/'

# Check that no shipped Mach-O still requires a build-runner/Homebrew path.
python3 scripts/release/verify-runtime.py "$APP"
"$BIN/yt-dlp" --version
"$BIN/ffmpeg" -version
"$BIN/ffprobe" -version
"$BIN/pdfunite" -v
"$BIN/pdfinfo" -v
"$BIN/deno" --version
"$BIN/whisper-cli" --help

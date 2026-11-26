#!/usr/bin/env bash
# Compiles crates/mobile-ffi for Android (arm64-v8a + x86_64) via cargo-ndk
# and generates the Kotlin UniFFI bindings into apps/android.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

detect_ndk() {
  if [ -n "${ANDROID_NDK_HOME:-}" ] && [ -d "${ANDROID_NDK_HOME}" ]; then
    echo "$ANDROID_NDK_HOME"
    return
  fi
  for base in \
    "${ANDROID_HOME:-}/ndk" \
    "${ANDROID_SDK_ROOT:-}/ndk" \
    "$HOME/Library/Android/sdk/ndk" \
    "$HOME/Android/Sdk/ndk"; do
    if [ -d "$base" ]; then
      local found
      found=$(find "$base" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -n 1 || true)
      if [ -n "$found" ]; then
        echo "$found"
        return
      fi
    fi
  done
}

NDK_DIR="$(detect_ndk)"
if [ -z "$NDK_DIR" ]; then
  echo "error: ANDROID_NDK_HOME not set and no Android NDK found" >&2
  exit 1
fi
export ANDROID_NDK_HOME="$NDK_DIR"

PROFILE="${1:-debug}"
CARGO_ARGS=()
TARGET_SUBDIR="debug"
if [ "$PROFILE" = "--release" ] || [ "$PROFILE" = "release" ]; then
  CARGO_ARGS+=(--release)
  TARGET_SUBDIR="release"
fi

echo "==> Building libottero_mobile_ffi.so ($TARGET_SUBDIR) with NDK $NDK_DIR"
cargo ndk -t arm64-v8a -t x86_64 --platform 28 \
  -o apps/android/app/src/main/jniLibs \
  build -p ottero-mobile-ffi ${CARGO_ARGS[@]+"${CARGO_ARGS[@]}"}

echo "==> Generating Kotlin UniFFI bindings"
cargo run -p ottero-mobile-ffi --features cli --bin uniffi-bindgen -- \
  generate \
  --library "target/aarch64-linux-android/${TARGET_SUBDIR}/libottero_mobile_ffi.so" \
  --language kotlin \
  --out-dir apps/android/app/src/main/kotlin

echo "==> Done: jniLibs + Kotlin UniFFI bindings updated."

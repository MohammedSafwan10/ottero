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
# Release Android `.so` binaries strip symbols, so extract UniFFI metadata from
# an unstripped host debug cdylib (works identically on macOS and Linux CI).
cargo build -p ottero-mobile-ffi --lib
HOST_LIB=""
for candidate in target/debug/libottero_mobile_ffi.dylib target/debug/libottero_mobile_ffi.so; do
  if [ -f "$candidate" ]; then
    HOST_LIB="$candidate"
    break
  fi
done
if [ -z "$HOST_LIB" ]; then
  echo "error: could not find host libottero_mobile_ffi.{dylib,so} for uniffi-bindgen" >&2
  exit 1
fi
cargo run -p ottero-mobile-ffi --features cli --bin uniffi-bindgen -- \
  generate \
  --library "$HOST_LIB" \
  --language kotlin \
  --out-dir apps/android/app/src/main/kotlin

if [ ! -f apps/android/app/src/main/kotlin/uniffi/ottero_mobile_ffi/ottero_mobile_ffi.kt ]; then
  echo "error: uniffi-bindgen did not generate ottero_mobile_ffi.kt" >&2
  exit 1
fi

echo "==> Done: jniLibs + Kotlin UniFFI bindings updated."

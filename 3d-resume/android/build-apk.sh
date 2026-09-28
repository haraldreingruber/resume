#!/usr/bin/env bash
# Builds the Android development APK: the app library for arm64 phones and
# x86_64 emulators, packaged with the SDK build tools (no Gradle) and signed
# with the keystore in $ANDROID_KEYSTORE (password: $ANDROID_KEYSTORE_PASSWORD,
# key alias "resume").
#
# Needs: $ANDROID_HOME (SDK with build-tools and a platform), the NDK
# ($ANDROID_NDK_HOME or $ANDROID_NDK_LATEST_HOME), and the Rust targets
# aarch64-linux-android and x86_64-linux-android. Runs on Linux.
#
# Usage: build-apk.sh <output.apk>
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
out=$(realpath -m "$1")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$here/../.."

min_sdk=26
ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_LATEST_HOME:?set ANDROID_NDK_HOME}}
toolchain="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
build_tools=$(ls -d "$ANDROID_HOME"/build-tools/* | sort -V | tail -1)
platform=$(ls -d "$ANDROID_HOME"/platforms/android-* | sort -V | tail -1)
target_sdk=${platform##*-}

# The library (loaded by NativeActivity) per ABI. Only Android builds it as
# a cdylib, so desktop and web builds don't pay for it.
declare -A triples=([arm64-v8a]=aarch64-linux-android [x86_64]=x86_64-linux-android)
for abi in "${!triples[@]}"; do
  triple=${triples[$abi]}
  env_triple=$(echo "$triple" | tr 'a-z-' 'A-Z_')
  export "CARGO_TARGET_${env_triple}_LINKER=$toolchain/${triple}${min_sdk}-clang"
  export "CC_${triple//-/_}=$toolchain/${triple}${min_sdk}-clang"
  export "AR_${triple//-/_}=$toolchain/llvm-ar"
  cargo rustc -p resume-3d --lib --crate-type cdylib --release --target "$triple"
  mkdir -p "$work/apk/lib/$abi"
  cp "target/$triple/release/libresume_3d.so" "$work/apk/lib/$abi/"
done

version=$(sed -n 's/^version = "\(.*\)"/\1/p' 3d-resume/Cargo.toml | head -1)
"$build_tools/aapt2" compile --dir "$here/res" -o "$work/res.zip"
"$build_tools/aapt2" link -o "$work/unsigned.apk" \
  --manifest "$here/AndroidManifest.xml" -I "$platform/android.jar" \
  --min-sdk-version "$min_sdk" --target-sdk-version "$target_sdk" \
  --version-code "${GITHUB_RUN_NUMBER:-1}" --version-name "$version" \
  "$work/res.zip"
(cd "$work/apk" && zip -qr "$work/unsigned.apk" lib)
"$build_tools/zipalign" -f -p 4 "$work/unsigned.apk" "$work/aligned.apk"
"$build_tools/apksigner" sign --ks "$ANDROID_KEYSTORE" --ks-key-alias resume \
  --ks-pass env:ANDROID_KEYSTORE_PASSWORD --out "$out" "$work/aligned.apk"
"$build_tools/apksigner" verify "$out"
echo "Built $out ($(wc -c < "$out") bytes, target SDK $target_sdk)"

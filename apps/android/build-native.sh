#!/usr/bin/env bash
# Compile le moteur Rust (crates/vtt-android) pour les téléphones ARM64 et le
# place dans app/src/main/jniLibs, avec la bibliothèque C++ du NDK dont
# whisper.cpp a besoin, et les bibliothèques officielles d'ONNX Runtime
# (Parakeet) et de Vosk, tirées de leurs paquets Maven (empreintes vérifiées).
#
# Prérequis : ANDROID_HOME (SDK avec ndk;29.0.14206865 et cmake;3.31.6),
# cible Rust aarch64-linux-android, cargo-ndk.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
ndk_version="29.0.14206865"
cmake_version="3.31.6"
: "${ANDROID_HOME:?ANDROID_HOME doit pointer vers le SDK Android}"
ndk="$ANDROID_HOME/ndk/$ndk_version"
abi="arm64-v8a"
api=29

export ANDROID_NDK_HOME="$ndk"
export PATH="$ANDROID_HOME/cmake/$cmake_version/bin:$PATH"
# whisper.cpp (CMake) : support Android natif de CMake, construit avec Ninja.
# whisper-rs-sys transmet à CMake les variables CMAKE_*.
export CMAKE_GENERATOR=Ninja
export CMAKE_ANDROID_NDK="$ndk"
export CMAKE_ANDROID_ARCH_ABI="$abi"
export CMAKE_SYSTEM_VERSION="$api"
# Instructions des processeurs de téléphone depuis 2018 (dotprod, fp16) :
# sans elles, ggml vise l'ARM de base et les modèles 8 bits sont plusieurs
# fois plus lents.
export GGML_NATIVE=OFF
export GGML_CPU_ARM_ARCH="armv8.2-a+dotprod+fp16"

out="$here/app/src/main/jniLibs"
cd "$here/../.."
cargo ndk -t "$abi" --platform "$api" -o "$out" build --release -p vtt-android
cp "$ndk/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so" \
  "$out/$abi/"

# Paquet Maven (.aar) téléchargé une fois, vérifié, puis la bibliothèque
# ARM64 extraite.
cache="$here/.cache"
mkdir -p "$cache"
native_lib() {
  local url="$1" sha256="$2" lib="$3"
  local aar="$cache/$(basename "$url")"
  if ! echo "$sha256  $aar" | sha256sum -c --quiet >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$aar.part" "$url"
    echo "$sha256  $aar.part" | sha256sum -c --quiet
    mv "$aar.part" "$aar"
  fi
  unzip -o -q -j "$aar" "jni/$abi/$lib" -d "$out/$abi"
}
maven="https://repo1.maven.org/maven2"
native_lib "$maven/com/microsoft/onnxruntime/onnxruntime-android/1.31.1/onnxruntime-android-1.31.1.aar" \
  d4d50660300ed2c4f38ffdfd5ab5bfa3a22f995f282dda24f42ea3ea55467b49 libonnxruntime.so
native_lib "$maven/com/alphacephei/vosk-android/0.3.75/vosk-android-0.3.75.aar" \
  ab2f8b91ac8051561aa325546b35fed9a68b36b8121bac5c6fb927525c4adfad libvosk.so
ls -la "$out/$abi"

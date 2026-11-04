#!/usr/bin/env bash
# Compile le moteur Rust (crates/vtt-android) pour les téléphones ARM64 et le
# place dans app/src/main/jniLibs, avec la bibliothèque C++ du NDK dont
# whisper.cpp a besoin.
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
ls -la "$out/$abi"

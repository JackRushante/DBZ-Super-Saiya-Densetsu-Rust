#!/usr/bin/env bash
# Builds the Android app: libmain.so (the game) and libSDL3.so for the ABIs given, SDL's Java glue, then the APK.
#   android/build-apk.sh [--release] [abi ...]        default ABI: x86_64 (the emulator); use arm64-v8a for a phone
# Needs: Rust with the Android targets, cargo-ndk, the Android SDK and NDK (ANDROID_HOME, ANDROID_NDK_HOME),
# a JDK 17+, cmake. The APK lands in android/app/build/outputs/apk/.
set -euo pipefail
cd "$(dirname "$0")/.."
# .cargo/config.toml points CMAKE at a Windows path; an environment value wins over it.
export CMAKE="${CMAKE_ANDROID:-$(command -v cmake)}"
profile=debug; cargo_profile=(); gradle_task=assembleDebug
if [ "${1:-}" = "--release" ]; then profile=release; cargo_profile=(--release); gradle_task=assembleRelease; shift; fi
abis=("$@"); [ ${#abis[@]} -eq 0 ] && abis=(x86_64)

jni=android/app/src/main/jniLibs
rm -rf "$jni"
for abi in "${abis[@]}"; do
    case $abi in
        arm64-v8a) triple=aarch64-linux-android ;;
        x86_64) triple=x86_64-linux-android ;;
        *) echo "unsupported ABI $abi" >&2; exit 2 ;;
    esac
    export CMAKE_TOOLCHAIN_FILE="${ANDROID_NDK_HOME:?}/build/cmake/android.toolchain.cmake" ANDROID_ABI="$abi" ANDROID_PLATFORM=android-26
    cargo ndk -t "$abi" -o "$jni" build -p ssd-android "${cargo_profile[@]}"
    # cargo-ndk copies libmain.so only; SDL's shared library is built by its build script.
    sdl=$(ls -t target/"$triple"/"$profile"/build/sdl3-sys-*/out/lib/libSDL3.so 2>/dev/null | head -1 || true)
    [ -n "$sdl" ] || sdl=$(find target/"$triple"/"$profile"/build -name libSDL3.so | head -1)
    cp "$sdl" "$jni/$abi/libSDL3.so"
done

# SDL's Java glue (zlib licence) comes from the SDL sources cargo already fetched; it is not tracked.
sdl_java=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/sdl3-src-*/SDL/android-project/app/src/main/java/org/libsdl/app | head -1)
rm -rf android/app/src/main/java/org
mkdir -p android/app/src/main/java/org/libsdl
cp -r "$sdl_java" android/app/src/main/java/org/libsdl/app

(cd android && ./gradlew --no-daemon "$gradle_task")

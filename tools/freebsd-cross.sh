#!/bin/sh
# The FreeBSD release build's cross toolchain: GitHub has no FreeBSD runners, so a Linux runner builds the x86_64
# FreeBSD download with clang and lld against a pinned FreeBSD base system (the oldest supported release, so the
# download runs on it and every later one) plus FreeBSD's alsa-lib package (cpal's and so Honeydew's audio is ALSA
# there, which base lacks), both checked by SHA-256.
# Usage: tools/freebsd-cross.sh <work dir>. It leaves the sysroot in <work dir>/freebsd, the compiler wrappers (cc,
# c++, and lld-clang, which links with lld) in <work dir>/freebsd-bin, and points cargo, cc, bindgen and pkg-config
# at them: in GitHub Actions through GITHUB_ENV, else in <work dir>/freebsd.env.
set -eu
[ "$#" -eq 1 ] || { echo 'usage: freebsd-cross.sh <work dir>' >&2; exit 2; }
version=14.4
base_sha256=769f60a6eea2938ad6b7943cfbcc17dfaf7d1f7ba59a32b869a00349302df853
# The 14.4 release's package set (release_4), which pkg.freebsd.org keeps unchanged for the branch's life.
alsa_url=https://pkg.freebsd.org/FreeBSD:14:amd64/release_4/All/Hashed/alsa-lib-1.2.14~26607f7acd.pkg
alsa_sha256=26607f7acd0a6efb523d2c4273dad9d1f1614776738d7cbd276130e53f0d7772
triple=x86_64-unknown-freebsd$version
work=$(cd -- "$1" && pwd)
sysroot=$work/freebsd
bin=$work/freebsd-bin
mkdir -p "$sysroot" "$bin"
curl --retry 3 -sSfLo "$work/base.txz" "https://download.freebsd.org/releases/amd64/$version-RELEASE/base.txz"
curl --retry 3 -sSfLo "$work/alsa-lib.pkg" "$alsa_url"
printf '%s  %s\n' "$base_sha256" "$work/base.txz" "$alsa_sha256" "$work/alsa-lib.pkg" | sha256sum -c
# Headers, libraries and the C runtime objects, without the debug files; ALSA's headers, library and .pc file.
tar -xJf "$work/base.txz" -C "$sysroot" --exclude ./usr/lib/debug ./lib ./usr/lib ./usr/include
tar --zstd --warning=no-unknown-keyword -xf "$work/alsa-lib.pkg" -C "$sysroot" /usr/local/include /usr/local/lib /usr/local/libdata/pkgconfig
# rustc reads a linker named *-clang as a compiler driver.
for tool in cc:clang c++:clang++ lld-clang:'clang -fuse-ld=lld'; do
    printf '#!/bin/sh\nexec %s --target=%s --sysroot=%s "$@"\n' "${tool#*:}" "$triple" "$sysroot" > "$bin/${tool%%:*}"
    chmod +x "$bin/${tool%%:*}"
done
# pkg-config reads only the sysroot's .pc files (alsa.pc) and prefixes their paths with the sysroot.
cat >> "${GITHUB_ENV:-$work/freebsd.env}" <<EOF
CC_x86_64_unknown_freebsd=$bin/cc
CXX_x86_64_unknown_freebsd=$bin/c++
AR_x86_64_unknown_freebsd=llvm-ar
CARGO_TARGET_X86_64_UNKNOWN_FREEBSD_LINKER=$bin/lld-clang
BINDGEN_EXTRA_CLANG_ARGS_x86_64_unknown_freebsd=--sysroot=$sysroot
PKG_CONFIG_ALLOW_CROSS=1
PKG_CONFIG_LIBDIR_x86_64_unknown_freebsd=$sysroot/usr/local/libdata/pkgconfig
PKG_CONFIG_SYSROOT_DIR_x86_64_unknown_freebsd=$sysroot
EOF

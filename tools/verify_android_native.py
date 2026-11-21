#!/usr/bin/env python3
"""Fail closed on APK native ABI, dependency and Activity/JNI entry-point mismatches.

This is static validation, not a substitute for Android linker/device execution.
"""
import argparse
import pathlib
import re
import struct
import subprocess
import tempfile
import zipfile

ABIS = {"armeabi-v7a": (1, 40), "arm64-v8a": (2, 183)}
SYSTEM = {"libc.so", "libm.so", "libdl.so", "liblog.so", "libandroid.so", "libEGL.so",
          "libGLESv2.so", "libGLESv3.so", "libOpenSLES.so", "libaaudio.so", "libz.so",
          "libjnigraphics.so", "libmediandk.so", "libvulkan.so"}


def check_identity(data, abi):
    if len(data) < 52 or data[:4] != b"\x7fELF" or data[5] != 1:
        raise ValueError(f"{abi}: expected little-endian ELF")
    identity = (data[4], struct.unpack_from("<H", data, 18)[0])
    if identity != ABIS[abi]:
        raise ValueError(f"{abi}: incorrect ELF class/machine {identity}, expected {ABIS[abi]}")


def check_dependencies(needed, packaged):
    missing = set(needed) - SYSTEM - set(packaged)
    if missing:
        raise ValueError(f"Unbundled non-system dependencies: {sorted(missing)}")


def check_exports(symbols, java):
    required = {"ANativeActivity_onCreate"}
    # Methods are all static, non-overloaded JNI callbacks in this Activity.
    required.update("Java_dev_daijro_banaforge_BanaForgeActivity_" + method
                    for method in re.findall(r"\bnative\s+\w+\s+(\w+)\s*\(", java))
    missing = required - set(symbols)
    if missing:
        raise ValueError(f"Missing exported Activity/JNI entry points: {sorted(missing)}")


def verify(apk_path, abis, readelf, java_path):
    java = pathlib.Path(java_path).read_text()
    with zipfile.ZipFile(apk_path) as apk, tempfile.TemporaryDirectory() as temp:
        native = [n for n in apk.namelist() if n.startswith("lib/") and n.endswith(".so")]
        actual = {n.split("/")[1] for n in native}
        if actual != set(abis):
            raise ValueError(f"APK ABIs {sorted(actual)} != requested {sorted(abis)}")
        for abi in abis:
            names = [n for n in native if n.startswith(f"lib/{abi}/")]
            packaged = {pathlib.PurePosixPath(n).name for n in names}
            if not {"libbanaforge.so", "libc++_shared.so"} <= packaged:
                raise ValueError(f"{abi}: missing engine or C++ runtime")
            for name in names:
                data = apk.read(name)
                check_identity(data, abi)
                library = pathlib.Path(temp) / pathlib.PurePosixPath(name).name
                library.write_bytes(data)
                dynamic = subprocess.check_output([readelf, "--dynamic", str(library)], text=True)
                needed = re.findall(r"\(NEEDED\).*?\[(.*?)\]", dynamic)
                check_dependencies(needed, packaged)
                if library.name == "libbanaforge.so":
                    table = subprocess.check_output([readelf, "--dyn-syms", "--wide", str(library)], text=True)
                    symbols = {line.split()[-1] for line in table.splitlines()
                               if " UND " not in line and line.split()}
                    check_exports(symbols, java)
                print(f"Verified {name}: ELF identity, DT_NEEDED {needed}")
            print(f"Verified {abi}: actual engine, C++ runtime and Activity JNI exports")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk")
    parser.add_argument("--abis", required=True, help="comma-separated expected ABIs")
    parser.add_argument("--readelf", required=True)
    parser.add_argument("--java", default="android/app/src/main/java/dev/daijro/banaforge/BanaForgeActivity.java")
    args = parser.parse_args()
    verify(args.apk, args.abis.split(","), args.readelf, args.java)

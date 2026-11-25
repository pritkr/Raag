#!/usr/bin/env python3
"""Shrink an NSH-packaged Android APK under 15 MB by enabling
android:extractNativeLibs="true", DEFLATE-compressing native .so libraries,
re-aligning with zipalign, and re-signing with apksigner.
"""

from __future__ import annotations

import argparse
import glob
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zipfile


ATTR_EXTRACT_NATIVE_LIBS_RES_ID = 0x010104EA
RES_XML_RESOURCE_MAP_TYPE = 0x0180
RES_XML_START_ELEMENT_TYPE = 0x0102
TYPE_INT_BOOLEAN = 0x12


def enable_extract_native_libs(axml: bytes) -> bytes:
    """Flip android:extractNativeLibs from false (0) to true (-1) in binary AXML."""
    if len(axml) < 8:
        raise ValueError("Invalid AndroidManifest.xml: too short")

    buf = bytearray(axml)
    pos = 8  # Skip ResXMLTree_header (type=0x0003, headerSize=8, size)
    attr_name_idx = None

    while pos + 8 <= len(buf):
        chunk_type, header_size, chunk_size = struct.unpack_from("<HHI", buf, pos)
        if chunk_size < 8 or pos + chunk_size > len(buf):
            break

        if chunk_type == RES_XML_RESOURCE_MAP_TYPE:
            count = (chunk_size - header_size) // 4
            for i in range(count):
                res_id = struct.unpack_from("<I", buf, pos + header_size + i * 4)[0]
                if res_id == ATTR_EXTRACT_NATIVE_LIBS_RES_ID:
                    attr_name_idx = i
                    break

        elif chunk_type == RES_XML_START_ELEMENT_TYPE and attr_name_idx is not None:
            # ResXMLTree_node (16 bytes) + ResXMLTree_attrExt (ns u32, name u32,
            # attrStart u16 at +24, attrSize u16 at +26, attrCount u16 at +28)
            attr_start, attr_size, attr_count = struct.unpack_from("<HHH", buf, pos + 24)
            attrs_base = pos + 16 + attr_start
            for i in range(attr_count):
                a_off = attrs_base + i * attr_size
                _ns, name_idx, _raw_val, _tv_size, _res0, data_type, data = struct.unpack_from(
                    "<IIIHBBI", buf, a_off
                )
                if name_idx == attr_name_idx:
                    if data_type != TYPE_INT_BOOLEAN:
                        raise ValueError(
                            f"extractNativeLibs has unexpected dataType=0x{data_type:02x}"
                        )
                    struct.pack_into("<I", buf, a_off + 16, 0xFFFFFFFF)
                    return bytes(buf)

        pos += chunk_size

    raise ValueError("android:extractNativeLibs attribute not found in AndroidManifest.xml")


def find_build_tool(name: str) -> str:
    candidates: list[str] = []
    for env_var in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        sdk = os.environ.get(env_var)
        if sdk:
            candidates.extend(sorted(glob.glob(os.path.join(sdk, "build-tools", "*", name))))
    home_sdk = os.path.expanduser(os.path.join("~", "Library", "Android", "sdk", "build-tools", "*", name))
    candidates.extend(sorted(glob.glob(home_sdk)))
    if candidates:
        return candidates[-1]
    found = shutil.which(name)
    if found:
        return found
    raise FileNotFoundError(f"Could not locate Android SDK tool '{name}'")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", help="Path to the APK to shrink in-place")
    parser.add_argument("--keystore", default=os.path.expanduser("~/.android/debug.keystore"))
    parser.add_argument("--key-alias", default="androiddebugkey")
    parser.add_argument(
        "--keystore-password",
        default=os.environ.get("NSH_KEYSTORE_PASSWORD", "android"),
    )
    args = parser.parse_args()

    apk_path = os.path.abspath(args.apk)
    if not os.path.isfile(apk_path):
        print(f"error: APK not found: {apk_path}", file=sys.stderr)
        return 1
    if not os.path.isfile(args.keystore):
        print(f"error: keystore not found: {args.keystore}", file=sys.stderr)
        return 1

    zipalign = find_build_tool("zipalign")
    apksigner = find_build_tool("apksigner")

    before_bytes = os.path.getsize(apk_path)

    with tempfile.TemporaryDirectory() as tmp:
        repacked = os.path.join(tmp, "repacked.apk")
        aligned = os.path.join(tmp, "aligned.apk")

        with zipfile.ZipFile(apk_path, "r") as zin, zipfile.ZipFile(repacked, "w") as zout:
            for info in zin.infolist():
                # Strip old signature files; apksigner will re-sign cleanly.
                if info.filename.startswith("META-INF/") and info.filename.endswith(
                    (".SF", ".RSA", ".DSA", ".EC", "MANIFEST.MF")
                ):
                    continue

                payload = zin.read(info.filename)
                if info.filename == "AndroidManifest.xml":
                    payload = enable_extract_native_libs(payload)

                # Keep resources.arsc uncompressed (Android R+ requirement);
                # compress native .so libraries and everything else at max DEFLATE.
                if info.filename == "resources.arsc":
                    compress_type = zipfile.ZIP_STORED
                else:
                    compress_type = zipfile.ZIP_DEFLATED

                out_info = zipfile.ZipInfo(info.filename, date_time=info.date_time)
                out_info.external_attr = info.external_attr
                out_info.compress_type = compress_type
                zout.writestr(out_info, payload, compress_type=compress_type, compresslevel=9)

        subprocess.run([zipalign, "-p", "-f", "4", repacked, aligned], check=True)
        subprocess.run(
            [
                apksigner,
                "sign",
                "--ks",
                args.keystore,
                "--ks-key-alias",
                args.key_alias,
                "--ks-pass",
                f"pass:{args.keystore_password}",
                "--out",
                apk_path,
                aligned,
            ],
            check=True,
        )
        subprocess.run([apksigner, "verify", apk_path], check=True)

    after_bytes = os.path.getsize(apk_path)
    print(
        f"Shrunk {os.path.basename(apk_path)}: "
        f"{before_bytes / (1024 * 1024):.2f} MB -> {after_bytes / (1024 * 1024):.2f} MB"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

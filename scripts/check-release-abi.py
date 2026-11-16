#!/usr/bin/env python3
"""Fail release builds when ELF linkage or Windows DLL imports drift."""

import argparse
import re
import struct
import subprocess
import sys
from pathlib import Path


MAX_GLIBC = (2, 34)
GNU_LIBRARIES = {"libc.so.6", "libgcc_s.so.1", "ld-linux-x86-64.so.2"}
WINDOWS_LIBRARIES = {
    "kernel32.dll",
    "bcryptprimitives.dll",
    "ntdll.dll",
    "userenv.dll",
    "api-ms-win-core-synch-l1-2-0.dll",
}


def readelf(path: Path, *args: str) -> str:
    return subprocess.check_output(["readelf", "--wide", *args, str(path)], text=True)


def check_elf(path: Path, target: str) -> None:
    headers = readelf(path, "--program-headers")
    dynamic = readelf(path, "--dynamic")
    versions = readelf(path, "--version-info")
    libraries = set(re.findall(r"Shared library: \[([^]]+)\]", dynamic))
    interpreters = re.findall(r"Requesting program interpreter: ([^]]+)", headers)
    print(f"ELF interpreter: {interpreters or 'none'}")
    print(f"ELF needed libraries: {sorted(libraries)}")

    if target.endswith("-musl"):
        if interpreters or libraries or re.search(r"GLIBC_\d", versions):
            raise ValueError("musl release must be static, without an interpreter, shared libraries, or GLIBC symbols")
        return

    if not interpreters or not interpreters[0].endswith("/ld-linux-x86-64.so.2"):
        raise ValueError("unexpected GNU Linux ELF interpreter")
    if not {"libc.so.6"}.issubset(libraries) or libraries - GNU_LIBRARIES:
        raise ValueError(f"unexpected GNU Linux shared libraries: {sorted(libraries - GNU_LIBRARIES)}")

    # A weak version requirement indicates an optional symbol probe; it does
    # not establish the minimum glibc version needed to load the executable.
    required = []
    for line in versions.splitlines():
        match = re.search(r"Name: GLIBC_(\d+)\.(\d+)\s+Flags: (\S+)", line)
        if match and match.group(3) != "WEAK":
            required.append((int(match.group(1)), int(match.group(2))))
    if not required:
        raise ValueError("GNU Linux release has no readable GLIBC version requirements")
    ceiling = max(required)
    print(f"Highest required non-weak glibc: {ceiling[0]}.{ceiling[1]} (limit {MAX_GLIBC[0]}.{MAX_GLIBC[1]})")
    if ceiling > MAX_GLIBC:
        raise ValueError("glibc minimum exceeds the release compatibility limit")


def pe_imports(path: Path) -> set[str]:
    data = path.read_bytes()

    def u16(offset: int) -> int:
        return struct.unpack_from("<H", data, offset)[0]

    def u32(offset: int) -> int:
        return struct.unpack_from("<I", data, offset)[0]

    if data[:2] != b"MZ":
        raise ValueError("not a Windows PE executable")
    pe = u32(0x3C)
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("invalid PE signature")
    sections = u16(pe + 6)
    optional_size = u16(pe + 20)
    optional = pe + 24
    if u16(optional) != 0x20B:
        raise ValueError("expected PE32+ 64-bit executable")
    import_rva = u32(optional + 112 + 8)  # DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT]
    if import_rva == 0:
        raise ValueError("PE has no DLL import table")

    def offset_for_rva(rva: int) -> int:
        for section in range(sections):
            start = optional + optional_size + section * 40
            virtual_size = u32(start + 8)
            virtual_address = u32(start + 12)
            raw_size = u32(start + 16)
            raw_offset = u32(start + 20)
            if virtual_address <= rva < virtual_address + max(virtual_size, raw_size):
                return raw_offset + rva - virtual_address
        raise ValueError(f"PE RVA 0x{rva:x} has no file section")

    descriptor = offset_for_rva(import_rva)
    imports = set()
    for _ in range(1024):
        fields = struct.unpack_from("<IIIII", data, descriptor)
        if fields == (0, 0, 0, 0, 0):
            return imports
        name_offset = offset_for_rva(fields[3])
        end = data.index(b"\0", name_offset)
        imports.add(data[name_offset:end].decode("ascii").lower())
        descriptor += 20
    raise ValueError("unterminated PE DLL import table")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()

    try:
        if args.target in ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"):
            check_elf(args.binary, args.target)
        elif args.target == "x86_64-pc-windows-msvc":
            imports = pe_imports(args.binary)
            print(f"Windows DLL imports: {sorted(imports)}")
            runtime_imports = {
                name for name in imports
                if name.startswith(("vcruntime", "msvcp", "msvcr", "ucrtbase", "api-ms-win-crt-"))
            }
            if runtime_imports:
                raise ValueError(f"dynamically linked Windows CRT libraries: {sorted(runtime_imports)}")
            unexpected = imports - WINDOWS_LIBRARIES
            if not imports or unexpected:
                raise ValueError(f"unexpected Windows DLL imports: {sorted(unexpected)}")
        else:
            raise ValueError(f"unsupported ABI inspection target: {args.target}")
    except (OSError, ValueError, subprocess.CalledProcessError, struct.error) as exc:
        print(f"release ABI inspection failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

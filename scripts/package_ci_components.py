#!/usr/bin/env python3
"""Stage/assemble same-source Unix shipping profiles from separate CI jobs.

The test-release workflow consumes these manifests before making the installer
archive. They bind the source, target, profile, exact member set and bytes; they
are not signatures or proof that synthetic self-test binaries execute natively.
Existing compiler profiles and native --version checks stay in the workflow.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import stat
import sys
import tarfile
import tempfile
from typing import BinaryIO

PROFILES = {"wrapper-release": ("rch",), "daemon-release": ("rchd", "rch-wkr")}
MANIFEST = "component.json"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def digest_stream(stream: BinaryIO) -> str:
    digest = hashlib.sha256()
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        digest.update(chunk)
    return digest.hexdigest()


def regular(path: Path) -> None:
    require(stat.S_ISREG(path.lstat().st_mode), f"not a regular component file: {path}")


def digest_file(path: Path) -> str:
    regular(path)
    with path.open("rb") as stream:
        return digest_stream(stream)


def identity(source: str, target: str) -> None:
    require(re.fullmatch(r"[0-9a-f]{40}", source) is not None, "source must be an exact commit SHA")
    require(re.fullmatch(r"[a-zA-Z0-9_-]+", target) is not None, "invalid target triple")


def stage(profile: str, source: str, target: str, build_root: Path, destination: Path) -> None:
    identity(source, target)
    require(profile in PROFILES, "unknown shipping profile")
    files = {name: build_root / target / profile / name for name in PROFILES[profile]}
    for path in files.values():
        regular(path)
        require(bool(path.stat().st_mode & 0o111), f"build output is not executable: {path}")
    # Exclusive directory creation prevents a stale earlier component from
    # surviving a failed preparation. Failed preparation retains its own bytes.
    destination.mkdir(parents=True, exist_ok=False)
    hashes = {}
    for name, path in files.items():
        expected = digest_file(path)
        shutil.copyfile(path, destination / name)
        require(digest_file(destination / name) == expected, "component changed during staging")
        hashes[name] = expected
    receipt = {"version": 1, "source": source, "target": target,
               "profile": profile, "files": hashes}
    with (destination / MANIFEST).open("x", encoding="utf-8") as stream:
        json.dump(receipt, stream, sort_keys=True)
        stream.write("\n")


def components(parts: Path, source: str, target: str) -> dict[str, tuple[Path, str]]:
    identity(source, target)
    require(parts.is_dir() and not parts.is_symlink(), "missing/unsafe component root")
    require({path.name for path in parts.iterdir()} == set(PROFILES), "both shipping profiles are required")
    members = {}
    for profile, names in PROFILES.items():
        directory = parts / profile
        require(directory.is_dir() and not directory.is_symlink(), "unsafe component directory")
        require({path.name for path in directory.iterdir()} == {*names, MANIFEST},
                f"missing or unexpected files in {profile}")
        manifest = directory / MANIFEST
        regular(manifest)
        require(manifest.stat().st_size <= 64 * 1024, "component manifest too large")
        receipt = json.loads(manifest.read_text(encoding="utf-8"))
        require(isinstance(receipt, dict) and set(receipt) ==
                {"version", "source", "target", "profile", "files"}, "invalid component manifest")
        require(type(receipt["version"]) is int and receipt["version"] == 1,
                "unsupported component version")
        require((receipt["source"], receipt["target"], receipt["profile"]) ==
                (source, target, profile), "component source/target/profile mismatch")
        require(isinstance(receipt["files"], dict) and set(receipt["files"]) == set(names),
                "component member set mismatch")
        for name in names:
            expected = receipt["files"][name]
            require(isinstance(expected, str) and re.fullmatch(r"[0-9a-f]{64}", expected) is not None,
                    "invalid component checksum")
            path = directory / name
            require(digest_file(path) == expected, f"component checksum mismatch: {name}")
            require(name not in members, "duplicate component member")
            members[name] = (path, expected)
    return members


def assemble(parts: Path, source: str, target: str, tag: str, output: Path) -> Path:
    require(re.fullmatch(r"[A-Za-z0-9._-]{1,100}", tag) is not None, "invalid archive tag")
    members = components(parts, source, target)
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"rch-{tag}-{target}.tar.gz"
    checksum = archive.with_name(archive.name + ".sha256")
    if archive.exists() or archive.is_symlink() or checksum.exists() or checksum.is_symlink():
        raise FileExistsError("refusing to overwrite a shipping archive or checksum")
    with archive.open("xb") as destination, tarfile.open(fileobj=destination, mode="w:gz", format=tarfile.USTAR_FORMAT) as bundle:
        for name, (path, _) in sorted(members.items()):
            with path.open("rb") as source_file:
                info = tarfile.TarInfo(name)
                info.size = path.stat().st_size
                info.mode = 0o755
                bundle.addfile(info, source_file)
    # Verify the actual archived bytes too. Mutation during assembly cannot
    # acquire a checksum sidecar and then silently pass the installer gate.
    with tarfile.open(archive, "r:gz") as bundle:
        entries = bundle.getmembers()
        require(len(entries) == len(members) and {entry.name for entry in entries} == set(members),
                "shipping archive member mismatch")
        for entry in entries:
            require(entry.isfile() and entry.mode == 0o755, "unsafe shipping member")
            stream = bundle.extractfile(entry)
            require(stream is not None, "missing shipping payload")
            with stream:
                require(digest_stream(stream) == members[entry.name][1], "archived component changed")
    with checksum.open("x", encoding="utf-8") as stream:
        stream.write(f"{digest_file(archive)}  {archive.name}\n")
    return archive


def self_test() -> None:
    # Deliberately synthetic payloads; exercise real files/tar/checksums only.
    # Retain the owned temporary root for diagnosis rather than deleting it.
    root = Path(tempfile.mkdtemp(prefix="rch-ci-components-"))
    source, target, tag = "a" * 40, "x86_64-apple-darwin", "pr-0-test"

    def fixture(name: str) -> tuple[Path, Path]:
        case = root / name
        build = case / "build"
        parts = case / "parts"
        for profile, names in PROFILES.items():
            directory = build / target / profile
            directory.mkdir(parents=True)
            for member in names:
                path = directory / member
                path.write_bytes(f"synthetic-{member}\n".encode())
                path.chmod(0o755)
            stage(profile, source, target, build, parts / profile)
        return case, parts

    case, parts = fixture("positive")
    archive = assemble(parts, source, target, tag, case / "dist")
    with tarfile.open(archive) as bundle:
        assert sorted(bundle.getnames()) == ["rch", "rch-wkr", "rchd"]
        for member in bundle:
            assert bundle.extractfile(member).read() == f"synthetic-{member.name}\n".encode()
    before = archive.read_bytes()
    try:
        assemble(parts, source, target, tag, case / "dist")
    except FileExistsError:
        assert archive.read_bytes() == before
    else:
        raise AssertionError("archive overwrite accepted")
    try:
        stage("wrapper-release", source, target, case / "build", parts / "wrapper-release")
    except FileExistsError:
        pass
    else:
        raise AssertionError("component overwrite accepted")

    mutations = ("missing-profile", "missing-binary", "extra-file", "symlink",
                 "bytes", "source", "target", "profile", "schema", "members")
    for fault in mutations:
        case, parts = fixture(fault)
        directory = parts / "daemon-release"
        manifest = directory / MANIFEST
        if fault == "missing-profile":
            directory.rename(case / "retained-profile")
        elif fault == "missing-binary":
            (directory / "rchd").rename(case / "retained-rchd")
        elif fault == "extra-file":
            (directory / "unexpected").write_text("unexpected")
        elif fault == "symlink":
            (directory / "rchd").rename(case / "retained-rchd")
            (directory / "rchd").symlink_to(case / "retained-rchd")
        elif fault == "bytes":
            (directory / "rchd").write_bytes(b"changed")
        else:
            receipt = json.loads(manifest.read_text())
            if fault == "source": receipt["source"] = "b" * 40
            elif fault == "target": receipt["target"] = "aarch64-apple-darwin"
            elif fault == "profile": receipt["profile"] = "wrapper-release"
            elif fault == "schema": receipt["version"] = True
            elif fault == "members": receipt["files"]["extra"] = "0" * 64
            manifest.write_text(json.dumps(receipt))
        try:
            assemble(parts, source, target, tag, case / "dist")
        except (OSError, ValueError):
            assert not list((case / "dist").glob("*.tar.gz")), fault
        else:
            raise AssertionError(f"invalid components accepted: {fault}")
    print(f"component packaging: 1 positive, 2 no-clobber and {len(mutations)} negative cases passed; synthetic binaries; retained {root}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("self-test")
    for action in ("stage", "assemble"):
        command = sub.add_parser(action)
        command.add_argument("--source", required=True)
        command.add_argument("--target", required=True)
        command.add_argument("--output", required=True, type=Path)
        if action == "stage":
            command.add_argument("--profile", required=True, choices=PROFILES)
            command.add_argument("--build-root", type=Path, default=Path("target"))
        else:
            command.add_argument("--parts", required=True, type=Path)
            command.add_argument("--tag", required=True)
    args = parser.parse_args()
    try:
        if args.action == "self-test":
            self_test()
        elif args.action == "stage":
            stage(args.profile, args.source, args.target, args.build_root, args.output)
        else:
            print(assemble(args.parts, args.source, args.target, args.tag, args.output))
    except (OSError, ValueError, tarfile.TarError) as error:
        print(f"component packaging refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Stage/assemble same-source Unix shipping profiles from separate CI jobs.

The test-release workflow consumes these manifests before making the installer
archive. They bind the source, target, profile, exact member set and bytes; they
are not signatures or proof that synthetic self-test binaries execute natively.
The E2E workflow also hands off its unchanged release-profile applications to
its test job. Both consumers retain native --version checks; this helper only
binds the same-revision/profile/target files and their bytes.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys
import tarfile
import tempfile
from typing import BinaryIO

PROFILES = {"wrapper-release": ("rch",), "daemon-release": ("rchd", "rch-wkr")}
STAGE_PROFILES = {**PROFILES, "release": ("rch", "rchd", "rch-wkr")}
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
    require(profile in STAGE_PROFILES, "unknown component profile")
    # E2E retains its native `cargo build --release` layout. Shipping retains
    # explicit target triples and the two existing custom profile layouts.
    directory = build_root / profile if profile == "release" else build_root / target / profile
    files = {name: directory / name for name in STAGE_PROFILES[profile]}
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


def component(directory: Path, source: str, target: str, profile: str) -> dict[str, tuple[Path, str]]:
    identity(source, target)
    require(profile in STAGE_PROFILES, "unknown component profile")
    names = STAGE_PROFILES[profile]
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
    members = {}
    for name in names:
        expected = receipt["files"][name]
        require(isinstance(expected, str) and re.fullmatch(r"[0-9a-f]{64}", expected) is not None,
                "invalid component checksum")
        path = directory / name
        require(digest_file(path) == expected, f"component checksum mismatch: {name}")
        members[name] = (path, expected)
    return members


def components(parts: Path, source: str, target: str) -> dict[str, tuple[Path, str]]:
    identity(source, target)
    require(parts.is_dir() and not parts.is_symlink(), "missing/unsafe component root")
    require({path.name for path in parts.iterdir()} == set(PROFILES), "both shipping profiles are required")
    members = {}
    for profile in PROFILES:
        for name, value in component(parts / profile, source, target, profile).items():
            require(name not in members, "duplicate component member")
            members[name] = value
    return members


def restore_e2e(parts: Path, source: str, target: str, output: Path) -> None:
    """Replace only the three test applications, never a whole cached target.

    Downloaded artifacts lose executable bits. Validate all inputs first,
    restore those bits and atomically replace each named build output. The
    workflow must require this entire step before allowing any test to run.
    """
    members = component(parts, source, target, "release")
    require(not output.is_symlink(), "unsafe E2E output directory")
    for name in members:
        path = output / name
        if path.exists() or path.is_symlink():
            regular(path)
    output.mkdir(parents=True, exist_ok=True)
    for name, (path, expected) in members.items():
        with tempfile.NamedTemporaryFile(prefix=".e2e-", dir=output, delete=False) as stream:
            temporary = Path(stream.name)
            with path.open("rb") as payload:
                shutil.copyfileobj(payload, stream)
        require(digest_file(temporary) == expected, "E2E component changed during restoration")
        temporary.chmod(0o755)
        os.replace(temporary, output / name)
        require(digest_file(output / name) == expected, "restored E2E bytes changed")


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

    # E2E uses one normal release profile, not the shipping profile pair.
    release_names = STAGE_PROFILES["release"]
    build = root / "e2e-build"
    release = build / "release"
    release.mkdir(parents=True)
    for name in release_names:
        path = release / name
        path.write_bytes(f"synthetic-release-{name}\n".encode())
        path.chmod(0o755)
    for fault in (None, "source", "target", "profile", "missing", "extra", "tampered", "symlink", "output-symlink"):
        case = root / ("e2e-" + str(fault))
        parts, output = case / "parts", case / "restored"
        stage("release", source, target, build, parts)
        output.mkdir()
        for name in release_names:
            (output / name).write_bytes(b"stale cached application")
        (output / "unrelated-cache").write_bytes(b"preserve")
        if fault in {"source", "target", "profile"}:
            receipt = json.loads((parts / MANIFEST).read_text())
            receipt[fault] = {"source": "b" * 40, "target": "wrong-target", "profile": "daemon-release"}[fault]
            (parts / MANIFEST).write_text(json.dumps(receipt))
        elif fault == "missing":
            (parts / "rchd").rename(case / "retained-rchd")
        elif fault == "extra":
            (parts / "unexpected").write_bytes(b"extra")
        elif fault == "tampered":
            (parts / "rch-wkr").write_bytes(b"changed")
        elif fault == "symlink":
            (parts / "rchd").rename(case / "retained-rchd")
            (parts / "rchd").symlink_to(case / "retained-rchd")
        elif fault == "output-symlink":
            (output / "rchd").rename(case / "retained-output")
            (output / "rchd").symlink_to(case / "retained-output")
        try:
            restore_e2e(parts, source, target, output)
        except (OSError, ValueError):
            assert fault is not None, fault
            for name in release_names:
                assert (output / name).read_bytes() == b"stale cached application", fault
        else:
            assert fault is None, fault
            for name in release_names:
                assert (output / name).read_bytes() == (release / name).read_bytes()
                assert stat.S_IMODE((output / name).stat().st_mode) == 0o755
        assert (output / "unrelated-cache").read_bytes() == b"preserve"
    print("E2E handoff: 1 positive and 8 negative cases passed; synthetic binaries")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("self-test")
    for action in ("stage", "assemble", "restore-e2e"):
        command = sub.add_parser(action)
        command.add_argument("--source", required=True)
        command.add_argument("--target", required=True)
        command.add_argument("--output", required=True, type=Path)
        if action == "stage":
            command.add_argument("--profile", required=True, choices=STAGE_PROFILES)
            command.add_argument("--build-root", type=Path, default=Path("target"))
        else:
            command.add_argument("--parts", required=True, type=Path)
            if action == "assemble":
                command.add_argument("--tag", required=True)
    args = parser.parse_args()
    try:
        if args.action == "self-test":
            self_test()
        elif args.action == "stage":
            stage(args.profile, args.source, args.target, args.build_root, args.output)
        elif args.action == "restore-e2e":
            restore_e2e(args.parts, args.source, args.target, args.output)
        else:
            print(assemble(args.parts, args.source, args.target, args.tag, args.output))
    except (OSError, ValueError, tarfile.TarError) as error:
        print(f"component packaging refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

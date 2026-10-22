#!/usr/bin/env bash
# Core-only, unsigned Linux candidate; never installs, deploys, or deletes files.
# Python 3.11+ supplies TOML/JSON and exclusive, atomic publication without jq.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
exec python3 -I - "$script_dir/.." "$@" <<'PY'
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import stat
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from datetime import datetime, timezone

CORE = ("rch", "rchd", "rch-wkr")
DEFAULT_ROOT = Path("/mnt/hdd-vibe-coding-share/rch-candidates")


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def file_sha(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def encoded(value):
    return (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True) + "\n").encode()


def run(args, *, cwd, env=None, capture=True, timeout=None):
    return subprocess.run(args, cwd=cwd, env=env, check=True,
                          stdin=subprocess.DEVNULL,
                          stdout=subprocess.PIPE if capture else None,
                          timeout=timeout).stdout


def git(repo, *args):
    environment = {k: os.environ[k] for k in ("PATH", "HOME") if k in os.environ}
    environment.update({"LC_ALL": "C", "GIT_CONFIG_NOSYSTEM": "1",
                        "GIT_CONFIG_GLOBAL": "/dev/null"})
    return run(["git", "--no-optional-locks", "-C", str(repo),
                "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null", *args],
               cwd=repo, env=environment)


def manifest(repo):
    """No patch bodies, config contents, environment, or untracked dumps escape."""
    head = git(repo, "rev-parse", "HEAD").decode().strip()
    require(re.fullmatch(r"[0-9a-f]{40,64}", head), "Invalid Git HEAD")
    workspace = tomllib.loads((repo / "Cargo.toml").read_text())
    members = workspace["workspace"]["members"]
    require(all(re.fullmatch(r"[A-Za-z0-9_-]+", m) for m in members),
            "Source manifest requires literal, top-level workspace members")
    tracked = set(git(repo, "ls-files", "--cached", "-z").split(b"\0")) - {b""}
    # Include ignored source too, but never walk unrelated target/node_modules
    # trees. These literal pathspecs prune traversal before Git enumerates files.
    roots = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config",
             ".cargo/config.toml", "install.sh", "scripts"]
    roots += [f"{m}/{p}" for m in members for p in ("src", "Cargo.toml", "build.rs")]
    others = set(git(repo, "ls-files", "--others", "-z", "--", *roots).split(b"\0")) - {b""}
    files = []
    for raw in sorted(tracked | others):
        name = os.fsdecode(raw)
        path = Path(name)
        parts = path.parts
        # Include all crate src assets, manifests/build scripts, and packaging
        # scripts. Other tracked edits remain covered by the full diff digest.
        relevant = (name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml",
                             ".cargo/config", ".cargo/config.toml", "install.sh")
                    or (len(parts) >= 2 and parts[0] in members
                        and (parts[1] == "src" or name.endswith("/Cargo.toml")
                             or name == parts[0] + "/build.rs"))
                    or (parts[0] == "scripts" and path.suffix in (".sh", ".py")))
        if not relevant:
            continue
        # Hash source, not local credentials, user config or generated dumps.
        if (any(p in (".git", "target", "node_modules", "__pycache__") for p in parts)
                or path.name.startswith(".env") or path.suffix in
                (".pem", ".key", ".p12", ".log", ".db", ".sqlite", ".pyc")):
            continue
        source = repo / path
        require(source.resolve().is_relative_to(repo), f"Source escapes repository: {name}")
        if not source.exists():
            require(raw in tracked, f"Source disappeared: {name}")
            files.append({"path": name, "state": "deleted", "tracked": True})
            continue
        mode = source.lstat().st_mode
        require(stat.S_ISREG(mode), f"Source must be a regular file: {name}")
        files.append({"path": name, "sha256": file_sha(source),
                      "mode": stat.S_IMODE(mode), "tracked": raw in tracked})
    diff = git(repo, "diff", "--binary", "--full-index", "--no-ext-diff",
               "--no-textconv", "--ignore-submodules=none", "HEAD", "--")
    require(not git(repo, "ls-files", "--unmerged", "-z"), "Unmerged source is not packageable")
    return {"schema": "rch-linux-candidate-source-v1", "head": head,
            "tracked_diff_sha256": sha(diff),
            "dirty": bool(diff) or any(not f["tracked"] for f in files),
            "scope": "full tracked diff digest plus workspace source/build and script file hashes; no source bodies",
            "files": files}


def stable(repo, original):
    require(manifest(repo) == original,
            "Source changed during packaging; candidate NOT published. Retained workspace is diagnostic only.")


def exclusive_file(path, data, mode=0o600):
    with path.open("xb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())
    path.chmod(mode)


def publish(path, data):
    # Same-directory staging + link gives atomic visibility and EEXIST instead
    # of replacement. Leave staging links intact; this script never deletes.
    staged = path.with_name("." + path.name + ".staged")
    exclusive_file(staged, data)
    os.link(staged, path)


def package(repo, output, runner=run, synthetic=False):
    require(sys.platform == "linux", "Only native Linux candidates are supported")
    repo = repo.resolve()
    require(git(repo, "rev-parse", "--show-toplevel").decode().strip() == str(repo),
            "Run from a Git workspace root")
    source = manifest(repo)
    source_bytes = encoded(source)
    source_id = sha(source_bytes)
    config = tomllib.loads((repo / "rust-toolchain.toml").read_text())
    channel = config["toolchain"]["channel"]
    require(re.fullmatch(r"nightly-\d{4}-\d{2}-\d{2}|\d+\.\d+\.\d+", channel),
            "rust-toolchain.toml must pin a dated nightly or exact stable version")
    version = tomllib.loads((repo / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    require(re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?", version), "Invalid workspace version")
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    tag = f"v{version}-candidate.{timestamp}.{source['head'][:12]}.{source_id[:12]}"
    if synthetic:
        tag += ".synthetic"
    if output is None:
        output = DEFAULT_ROOT / tag
    require(output.is_absolute(), "--output-dir must be an absolute NEW directory")
    output = output.parent.resolve() / output.name
    require(not output.is_relative_to(repo), "Output must be outside the source repository")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.mkdir()  # Atomic reservation; even an existing empty directory fails.
    work = Path(tempfile.mkdtemp(prefix=".rch-candidate-work-", dir=output.parent))
    print(f"Candidate output (receipt.json marks completion): {output}", file=sys.stderr)
    print(f"Retained build/smoke workspace (no cleanup): {work}", file=sys.stderr)
    # No ambient commit overrides/wrappers/completion hooks; cargo still uses
    # the operator's installed toolchain/cache. No toolchain installation here.
    build_env = {k: os.environ[k] for k in ("PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME")
                 if k in os.environ}
    build_env.update({"LC_ALL": "C", "CARGO_TARGET_DIR": str(work / "target"),
                      "TMPDIR": str(work), "RCH_SHIM_BYPASS": "1",
                      "RCH_BUILD_SOURCE": f"{source['head']}-overlay-{source_id}"})
    rustc = runner(["rustup", "run", channel, "rustc", "-vV"], cwd=repo, env=build_env).decode()
    hosts = re.findall(r"^host: (\S+)$", rustc, re.MULTILINE)
    require(len(hosts) == 1 and hosts[0] in ("x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl", "aarch64-unknown-linux-gnu"),
            "Toolchain host must be a release-supported native Linux target")
    target = hosts[0]
    cargo = runner(["rustup", "run", channel, "cargo", "--version"], cwd=repo, env=build_env).decode().strip()
    archive_name = f"rch-{tag}-{target}.tar.gz"
    commands = []
    for profile, packages in (("wrapper-release", ("rch",)),
                              ("daemon-release", ("rchd", "rch-wkr"))):
        command = ["rustup", "run", channel, "cargo", "build", "--locked",
                   "--profile", profile, "--target", target]
        for name in packages:
            command += ["-p", name, "--bin", name]
        commands.append(command)
        print("Building: " + " ".join(command), file=sys.stderr)
        runner(command, cwd=repo, env=build_env, capture=False)
        stable(repo, source)
    home = work / "smoke-home"
    home.mkdir()
    for directory in ("config", "cache", "data", "state", "runtime", "tmp"):
        (home / directory).mkdir(mode=0o700)
    smoke_env = {"PATH": "/usr/bin:/bin", "HOME": str(home), "LC_ALL": "C",
                 "XDG_CONFIG_HOME": str(home / "config"), "XDG_CACHE_HOME": str(home / "cache"),
                 "XDG_DATA_HOME": str(home / "data"), "XDG_STATE_HOME": str(home / "state"),
                 "XDG_RUNTIME_DIR": str(home / "runtime"), "TMPDIR": str(home / "tmp"),
                 "RCH_CONFIG_DIR": str(home / "config"), "RCH_NO_SELF_HEALING": "1",
                 "RCH_NO_UPDATE_CHECK": "1", "RCH_NO_HOOK": "1", "RCH_NO_RC": "1",
                 "RCH_SHIM_BYPASS": "1", "RCH_SOCKET_PATH": str(home / "runtime" / "absent.sock"),
                 "NO_COLOR": "1"}
    binaries = {}
    smoke = {}
    for name in CORE:
        profile = "wrapper-release" if name == "rch" else "daemon-release"
        binary = work / "target" / target / profile / name
        require(binary.is_file() and not binary.is_symlink() and os.access(binary, os.X_OK),
                f"Missing executable: {binary}")
        binaries[name] = {"sha256": file_sha(binary), "profile": profile}
        smoke[name] = {}
        for flag in ("--version", "--help"):
            # No inherited stdin, hook mode, config, daemon, or shell integration.
            args = [str(binary)] + (["--no-self-healing"] if name == "rch" else []) + [flag]
            text = runner(args, cwd=home, env=smoke_env, timeout=30)
            require(text.strip(), f"Empty smoke output: {name} {flag}")
            require(len(text) <= 1024 * 1024, f"Unexpectedly large smoke output: {name}")
            smoke[name][flag] = {"exit_code": 0, "stdout_sha256": sha(text)}
            if flag == "--version":
                smoke[name][flag]["text"] = text.decode().strip()
                expected_version = f"{name} {version} (commit {source['head'][:12]}-overlay-{source_id})"
                require(text.decode().strip() == expected_version,
                        f"Binary version/source stamp does not match candidate: {name}")
        require(file_sha(binary) == binaries[name]["sha256"], f"Binary changed during smoke: {name}")
    stable(repo, source)
    archive = output / archive_name
    archive_stage = output / ("." + archive_name + ".staged")
    with archive_stage.open("xb") as handle:
        with tarfile.open(fileobj=handle, mode="w:gz") as tar:
            for name in CORE:
                binary = work / "target" / target / binaries[name]["profile"] / name
                info = tar.gettarinfo(str(binary), arcname=name)
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mode = 0o755
                with binary.open("rb") as data:
                    tar.addfile(info, data)
        handle.flush()
        os.fsync(handle.fileno())
    # Verify the actual archive, not just the source executables.
    with tarfile.open(archive_stage, "r:gz") as tar:
        require(tar.getnames() == list(CORE), "Archive must contain exactly the three flat core binaries")
        for entry in tar.getmembers():
            require(entry.isfile(), "Archive contains a non-file member")
            with tar.extractfile(entry) as data:
                require(hashlib.file_digest(data, "sha256").hexdigest() == binaries[entry.name]["sha256"],
                        "Archived binary changed")
    stable(repo, source)
    digest = file_sha(archive_stage)
    install_args = ["bash", str(repo / "install.sh"), "--offline", str(archive),
                    "--version", tag.removeprefix("v")]
    receipt = {"schema": "rch-linux-candidate-receipt-v1", "status": "complete",
               "classification": "unsigned dirty candidate (not a signed release)",
               "proof": "synthetic-fixture-only; NOT real binaries" if synthetic else "native-build-and-cli-smoke-only",
               "created_utc": timestamp, "tag": tag, "workspace_version": version,
               "target": target, "toolchain": channel, "rustc": rustc.strip(), "cargo": cargo,
               "source_manifest": "source-manifest.json", "source_manifest_sha256": source_id,
               "head": source["head"], "source_dirty": source["dirty"],
               "build_source": build_env["RCH_BUILD_SOURCE"], "build_commands": commands,
               "archive": archive_name, "archive_sha256": digest,
               "binaries": binaries, "smoke": smoke,
               "version_note": "Archive tag is a candidate identifier only. All binaries retain workspace_version and carry build_source in --version; Cargo.toml is not rewritten.",
               "offline_install_argv": install_args,
               "unsigned_install_note": "Optional signature policy is required for this unsigned archive; choose host-specific installer flags and preserve rollback before invoking offline_install_argv.",
               "deployment": "not performed; optional RABS binaries not built or required",
               "rollback": "Before deployment retain the previous installation and its receipt; this candidate contains no previous binaries or host configuration."}
    os.link(archive_stage, archive)
    publish(output / (archive_name + ".sha256"), f"{digest}  {archive_name}\n".encode())
    publish(output / "source-manifest.json", source_bytes)
    # Consumers must require this last-published completion record and verify
    # its archive/source digests. Partial directories are never candidates.
    stable(repo, source)
    publish(output / "receipt.json", encoded(receipt))
    descriptor = os.open(output, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    print(output)
    if not synthetic:
        print("Not executed. After source verification and rollback backup, unsigned offline install template:\n"
              + "RCH_SIGNATURE_POLICY=optional " + shlex.join(install_args), file=sys.stderr)
    return output


def verify_source(repo, candidate):
    receipt = json.loads((candidate / "receipt.json").read_bytes())
    expected = (candidate / "source-manifest.json").read_bytes()
    require(receipt["status"] == "complete" and sha(expected) == receipt["source_manifest_sha256"],
            "Incomplete candidate or source manifest digest mismatch")
    require(encoded(manifest(repo.resolve())) == expected, "Current source does not match candidate")
    require(Path(receipt["archive"]).name == receipt["archive"], "Invalid receipt archive name")
    require(file_sha(candidate / receipt["archive"]) == receipt["archive_sha256"], "Candidate archive digest mismatch")
    print("Source and archive match the unsigned candidate receipt (not signature verification).")


def self_test(root):
    """Real Git/tar/hash checks, fake Rust tools/binaries; never invoke cargo."""
    root.mkdir(parents=True, exist_ok=False)
    repo = root / "fixture"
    repo.mkdir()
    fixture_files = {"Cargo.toml": '[workspace]\nmembers = ["rch", "rchd", "rch-wkr"]\n[workspace.package]\nversion = "0.0.0"\n',
                     "Cargo.lock": "# synthetic lock\n", "rust-toolchain.toml": '[toolchain]\nchannel = "nightly-2026-08-31"\n',
                     "rch/src/main.rs": "// synthetic source\n", "scripts/fixture.sh": "# fixture\n"}
    for name, text in fixture_files.items():
        path = repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        exclusive_file(path, text.encode())
    run(["git", "-c", "init.defaultBranch=main", "init", "--quiet", str(repo)], cwd=root)
    git(repo, "add", "--", ".")
    git(repo, "-c", "user.name=Packaging Fixture", "-c", "user.email=fixture@invalid",
        "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "synthetic fixture")
    exclusive_file(repo / "rch/src/untracked.rs", b"// untracked source\n")
    exclusive_file(repo / ".env", b"fixture-only-secret-not-for-manifest\n")
    calls = []

    def fake(args, **kwargs):
        if args[0] != "rustup":
            return run(args, **kwargs)
        calls.append(args)
        require(args[1:3] == ["run", "nightly-2026-08-31"], "Toolchain was not pinned")
        if args[3:] == ["rustc", "-vV"]:
            return b"rustc synthetic\nhost: x86_64-unknown-linux-gnu\n"
        if args[3:] == ["cargo", "--version"]:
            return b"cargo synthetic\n"
        require(args[3:6] == ["cargo", "build", "--locked"], "Build was not locked")
        packages = [args[i + 1] for i, value in enumerate(args) if value == "-p"]
        profile = args[args.index("--profile") + 1]
        require(packages == (["rch"] if profile == "wrapper-release" else ["rchd", "rch-wkr"]),
                "Unexpected packages (optional sidecars must not be built)")
        for name in packages:
            binary = Path(kwargs["env"]["CARGO_TARGET_DIR"]) / "x86_64-unknown-linux-gnu" / profile / name
            binary.parent.mkdir(parents=True, exist_ok=True)
            stamp = kwargs["env"]["RCH_BUILD_SOURCE"]
            commit, fingerprint = stamp.split("-overlay-")
            version_text = f"{name} 0.0.0 (commit {commit[:12]}-overlay-{fingerprint})"
            fixture = ('#!/bin/sh\nset -eu\ntest "$RCH_NO_SELF_HEALING" = 1\n'
                       'test "$RCH_NO_HOOK" = 1\ntest "$PWD" = "$HOME"\n'
                       'test -z "${RCH_HOOK_MODE:-}"\n'
                       'printf "SYNTHETIC fixture, NOT a Rust binary\\n" >&2\n'
                       f"printf '%s\\n' {shlex.quote(version_text)}\n")
            exclusive_file(binary, fixture.encode(), 0o755)
        return None

    candidate = package(repo, root / "candidate", fake, synthetic=True)
    receipt = json.loads((candidate / "receipt.json").read_text())
    require("synthetic" in receipt["proof"], "Synthetic proof was mislabeled")
    require(len(calls) == 4, "Unexpected Rust tool invocations")
    source = json.loads((candidate / "source-manifest.json").read_text())
    require(any(f["path"] == "rch/src/untracked.rs" and not f["tracked"] for f in source["files"]),
            "Untracked source not fingerprinted")
    require(not any(f["path"] == ".env" for f in source["files"]), "Secret included")
    verify_source(repo, candidate)
    archive = candidate / receipt["archive"]
    require((candidate / (archive.name + ".sha256")).read_text() == f"{file_sha(archive)}  {archive.name}\n",
            "Checksum sidecar does not match release format")
    try:
        package(repo, candidate, fake, synthetic=True)
    except FileExistsError:
        pass
    else:
        raise RuntimeError("Existing output directory was overwritten")
    for path in (repo / "rch/src/main.rs", repo / "rch/src/untracked.rs"):
        with path.open("ab") as handle:
            handle.write(b"// changed fixture\n")
        try:
            verify_source(repo, candidate)
        except RuntimeError:
            pass
        else:
            raise RuntimeError("Changed source was accepted")
    mutated = False

    def changing_build(args, **kwargs):
        nonlocal mutated
        result = fake(args, **kwargs)
        if "build" in args and not mutated:
            mutated = True
            with (repo / "rch/src/main.rs").open("ab") as handle:
                handle.write(b"// changed during fake build\n")
        return result

    try:
        package(repo, root / "changed-during-build", changing_build, synthetic=True)
    except RuntimeError as error:
        require("Source changed during packaging" in str(error), str(error))
    else:
        raise RuntimeError("Source change during build was accepted")
    require(not (root / "changed-during-build/receipt.json").exists(), "Failed build published receipt")
    print(f"PASS: synthetic packaging/source-drift/no-clobber/smoke-isolation checks; NO real Rust builds. Fixtures retained: {root}")


def main():
    parser = argparse.ArgumentParser(description="Package ONLY rch/rchd/rch-wkr as an unsigned dirty Linux candidate. Never deploy, configure hooks, or clean up. Python 3.11+, Git and an installed pinned Rust toolchain required.")
    parser.add_argument("--output-dir", type=Path, help="absolute NEW candidate directory; default /mnt/hdd-vibe-coding-share/rch-candidates/<candidate-tag>")
    parser.add_argument("--verify-source", type=Path, metavar="CANDIDATE_DIR", help="read-only verification against current HEAD/diff/source and archived bytes; no build")
    parser.add_argument("--self-test", action="store_true", help="hermetic fake Rust tools/binaries, real Git/tar; retained new fixture directory, no actual cargo")
    repo = Path(sys.argv[1]).resolve()
    args = parser.parse_args(sys.argv[2:])
    require(not (args.self_test and args.verify_source), "Choose self-test OR source verification")
    require(not (args.verify_source and args.output_dir), "Source verification has no output directory")
    os.umask(0o077)
    if args.self_test:
        root = args.output_dir or DEFAULT_ROOT / ("self-test-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ"))
        require(root.is_absolute(), "Self-test output must be absolute")
        self_test(root)
    elif args.verify_source:
        verify_source(repo, args.verify_source.resolve())
    else:
        package(repo, args.output_dir)


try:
    main()
except (OSError, RuntimeError, ValueError, KeyError, subprocess.SubprocessError) as error:
    print(f"package_linux_candidate: {error}", file=sys.stderr)
    sys.exit(1)
PY

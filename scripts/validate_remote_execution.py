#!/usr/bin/env python3
"""Run one real workload and require a completed remote RCH result.

This opt-in acceptance entrypoint is deliberately outside e2e_*.sh discovery:
ordinary CI has no operator fleet. It records the actual RCH envelope and the
merged legacy machine-mode stream, or separate native streams with --verification.
It never accepts mocked SSH/local fallback and never installs hooks, provisions
workers, or changes the selected project.
Run success, expected workload failure, and warm-cache invocations separately.
Cancellation, cache identity and storage cleanup still need separate evidence.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


def validate_result(result, returncode, expected_exit, worker):
    if not isinstance(result, dict):
        raise ValueError("RCH result must be an object")
    if result.get("api_version") != "1.0":
        raise ValueError("unsupported RCH exec result schema")
    if result.get("location") != "remote" or result.get("outcome") != "completed":
        raise ValueError("workload did not complete remotely")
    if not worker or result.get("worker_id") != worker:
        raise ValueError("result did not come from the requested worker")
    remote_exit = result.get("remote_exit_code")
    if type(remote_exit) is not int or remote_exit != expected_exit or returncode != expected_exit:
        raise ValueError("process/remote exit differs from expected workload status")
    if result.get("error_code") is not None or result.get("fallback_reason") is not None:
        raise ValueError("infrastructure failure or fallback is not workload completion")


def execution_capture(rch, command, evidence, worker, verification, inherited):
    env = inherited.copy()
    env.update(RCH_REQUIRE_REMOTE="1", RCH_WORKER=worker,
               RCH_NO_SELF_HEALING="1", RCH_NO_UPDATE_CHECK="1",
               RCH_VISIBILITY="verbose", NO_COLOR="1")
    if verification:
        # The typed invocation preserves native streams; requesting --json would
        # correctly refuse that conflicting stdout contract. Only result.json is
        # authoritative, never output emitted by the workload itself.
        for key in ("RCH_JSON", "RCH_OUTPUT_FORMAT", "RCH_HOOK_MODE"):
            env.pop(key, None)
        invocation = [rch, "--no-self-healing", "exec", "--verification",
                      "--workspace", "shared", "--json-result", str(evidence / "result.json"),
                      "--", *command]
        return invocation, env, evidence / "stdout.bin", evidence / "stderr.bin"
    env["RCH_OUTPUT_FORMAT"] = "json"
    return ([rch, "--no-self-healing", "--json", "exec", "--", *command],
            env, evidence / "result.json", evidence / "workload.log")


class ResultContractTests(unittest.TestCase):
    def result(self, code=0):
        return dict(api_version="1.0", outcome="completed", location="remote",
                    worker_id="test-worker", remote_exit_code=code,
                    error_code=None, fallback_reason=None)

    def test_success_and_expected_workload_failure(self):
        for code in (0, 1, 7, 101, 137):
            validate_result(self.result(code), code, code, "test-worker")

    def test_refusal_local_unknown_and_collection_failure_never_pass(self):
        for field, value in (("location", "local"), ("outcome", "refused"),
                             ("outcome", "collection_error"), ("outcome", "unknown"),
                             ("worker_id", "other"), ("remote_exit_code", None),
                             ("remote_exit_code", False), ("api_version", "2.0"),
                             ("error_code", "RCH-E309"), ("fallback_reason", "busy")):
            with self.subTest(field=field, value=value):
                result = self.result()
                result[field] = value
                with self.assertRaises(ValueError):
                    validate_result(result, 0, 0, "test-worker")

    def test_native_capture_never_treats_workload_stdout_as_result(self):
        inherited = {"RCH_JSON": "1", "RCH_OUTPUT_FORMAT": "json", "RCH_HOOK_MODE": "1", "KEEP": "value"}
        command = ["pytest", "a test.py"]
        args, env, out, err = execution_capture("/opt/rch", command, Path("/evidence"), "worker", True, inherited)
        self.assertNotIn("--json", args)
        self.assertIn("--verification", args)
        self.assertIn("--json-result", args)
        self.assertEqual(args[-2:], command)
        self.assertEqual(out, Path("/evidence/stdout.bin"))
        self.assertEqual(err, Path("/evidence/stderr.bin"))
        self.assertEqual(env["RCH_REQUIRE_REMOTE"], "1")
        self.assertEqual(env["RCH_WORKER"], "worker")
        self.assertEqual(env["KEEP"], "value")
        self.assertFalse(any(key in env for key in ("RCH_JSON", "RCH_OUTPUT_FORMAT", "RCH_HOOK_MODE")))
        self.assertEqual(inherited["RCH_JSON"], "1", "only child environment changes")

    def test_legacy_capture_keeps_machine_contract(self):
        args, env, out, err = execution_capture("rch", ["cargo", "test"], Path("/evidence"), "worker", False, {})
        self.assertIn("--json", args)
        self.assertNotIn("--verification", args)
        self.assertEqual(env["RCH_OUTPUT_FORMAT"], "json")
        self.assertEqual(out, Path("/evidence/result.json"))
        self.assertEqual(err, Path("/evidence/workload.log"))

    def test_process_and_remote_status_must_agree(self):
        with self.assertRaises(ValueError):
            validate_result(self.result(7), 0, 7, "test-worker")
        with self.assertRaises(ValueError):
            validate_result(self.result(0), 7, 7, "test-worker")
        with self.assertRaises(ValueError):
            validate_result([], 0, 0, "test-worker")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--rch", default="rch")
    parser.add_argument("--project", type=Path)
    parser.add_argument("--worker")
    parser.add_argument("--verification", action="store_true",
                        help="exercise ordinary verification through an explicitly configured shared workspace")
    parser.add_argument("--expected-exit", type=int, default=0)
    parser.add_argument("--evidence-root", type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.self_test:
        return not unittest.TextTestRunner(verbosity=2).run(
            unittest.defaultTestLoader.loadTestsFromTestCase(ResultContractTests)
        ).wasSuccessful()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not args.project or not args.worker or not args.evidence_root or not command:
        parser.error("--project, --worker, --evidence-root and -- <command> are required")
    if not 0 <= args.expected_exit <= 255:
        parser.error("expected exit must be in 0..255")
    # Do not silently unset mocks: reject a test environment pretending to be live.
    if any(name.startswith("RCH_MOCK_") or name == "RCH_TEST_MODE" for name in os.environ):
        parser.error("mock/test environment cannot qualify real remote execution")
    project = args.project.resolve(strict=True)
    if not project.is_dir():
        parser.error("project must be a directory")
    root = args.evidence_root.resolve(strict=True)
    evidence = Path(tempfile.mkdtemp(prefix="rch-remote-", dir=root))
    print(f"Evidence: {evidence}", file=sys.stderr)
    invocation, env, stdout_path, stderr_path = execution_capture(
        args.rch, command, evidence, args.worker, args.verification, os.environ)
    try:
        with stdout_path.open("xb") as stdout, stderr_path.open("xb") as stderr:
            completed = subprocess.run(invocation, cwd=project, env=env,
                                       stdout=stdout, stderr=stderr, check=False)
        (evidence / "exit-code.txt").write_text(f"{completed.returncode}\n")
        result = json.loads((evidence / "result.json").read_bytes())
        validate_result(result, completed.returncode, args.expected_exit, args.worker)
    except (OSError, ValueError) as error:
        print(f"FAIL: {error}; inspect {evidence}", file=sys.stderr)
        return 1
    print("PASS: requested workload completed on the requested remote worker; "
          "this alone is not cache/cancellation/storage or full migration acceptance.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

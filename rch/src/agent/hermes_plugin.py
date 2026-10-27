"""RCH Hermes integration: rewrite terminal commands, never execute them here.

Protocol follows remote-test/hermes-plugin: register pre_tool_call, mutate the
existing args dictionary, and return None to retain normal terminal execution.
"""

import json
import os
from pathlib import Path
import shlex
import subprocess
import sys

HOOK_TIMEOUT_SECONDS = 3


def register(ctx):
    ctx.register_hook("pre_tool_call", _pre_tool_call)


def _resolved_cwd(args, task_id):
    workdir = args.get("workdir")
    if isinstance(workdir, str) and workdir.strip():
        return os.path.realpath(workdir)
    try:
        from tools.terminal_tool import _get_env_config, get_session_cwd

        recorded = get_session_cwd(task_id)
        if isinstance(recorded, str) and recorded.strip():
            return os.path.realpath(recorded)
        configured = _get_env_config().get("cwd")
        if isinstance(configured, str) and configured.strip():
            return os.path.realpath(configured)
    except Exception:
        pass
    configured = os.environ.get("TERMINAL_CWD", "")
    if configured.strip().lower() not in {"", ".", "./", "auto", "cwd"}:
        return os.path.realpath(configured)
    return os.path.realpath(os.getcwd())


def _pre_tool_call(tool_name=None, args=None, task_id="", session_id="", **kwargs):
    if tool_name != "terminal":
        return None
    if not isinstance(args, dict):
        return {"action": "block", "message": "RCH received no valid terminal arguments."}
    command = args.get("command")
    if not isinstance(command, str) or not command.strip():
        return {"action": "block", "message": "RCH received no valid terminal command."}
    try:
        words = shlex.split(command)
    except ValueError:
        words = []  # Let the shell, not this adapter, interpret unsupported input.
    if words and Path(words[0]).name in {"rch", "remote-test"}:
        return None

    # Preserve every terminal argument (env, timeout, background, workdir, etc.).
    # Only the command field is accepted back from RCH's rewrite-only hook.
    cwd = _resolved_cwd(args, task_id)
    event = {
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": dict(args),
        "cwd": cwd,
        "session_id": session_id or kwargs.get("sessionId") or "",
    }
    invocation = kwargs.get("tool_invocation_id") or kwargs.get("tool_call_id")
    if invocation is not None:
        event["tool_invocation_id"] = str(invocation)
    try:
        result = subprocess.run(
            ["rch"], input=json.dumps(event), text=True, capture_output=True,
            cwd=cwd, timeout=HOOK_TIMEOUT_SECONDS, check=False,
        )
        if result.stderr.strip():
            print(result.stderr.strip(), file=sys.stderr)
        if result.returncode != 0:
            raise ValueError(f"hook exit {result.returncode}")
        if not result.stdout.strip():
            return None  # RCH pass-through is an empty stdout response.
        payload = json.loads(result.stdout)
        decision = payload.get("hookSpecificOutput") if isinstance(payload, dict) else None
        if not isinstance(decision, dict) or decision.get("hookEventName") != "PreToolUse":
            raise ValueError("invalid hook response")
        # No permissionDecision is intentional for compound rewrites: the host
        # still applies its normal permissions. This callback grants no approval.
        if decision.get("permissionDecision") not in {None, "allow"}:
            raise ValueError("unsupported hook decision")
        updated = decision.get("updatedInput")
        rewritten = updated.get("command") if isinstance(updated, dict) else None
        if not isinstance(rewritten, str) or not rewritten.strip():
            raise ValueError("missing rewritten command")
    except (OSError, subprocess.TimeoutExpired, ValueError, TypeError) as exc:
        print(f"[RCH] Hermes adapter unavailable ({exc}); leaving command local", file=sys.stderr)
        return None
    args["command"] = rewritten
    return None


if __name__ == "__main__":
    # Inline protocol tests; run with python3 -I hermes_plugin.py. All hook
    # subprocess responses are fixtures, not claims of a live remote build.
    import copy
    import types
    import unittest
    from unittest.mock import Mock, patch

    def response(command="rch exec -- cargo test", permission="allow"):
        decision = {"hookEventName": "PreToolUse", "updatedInput": {"command": command}}
        if permission is not None:
            decision["permissionDecision"] = permission
        return types.SimpleNamespace(returncode=0, stderr="", stdout=json.dumps({"hookSpecificOutput": decision}))

    class ProtocolTests(unittest.TestCase):
        def test_registration(self):
            ctx = Mock()
            register(ctx)
            ctx.register_hook.assert_called_once_with("pre_tool_call", _pre_tool_call)

        def test_rewrite_preserves_arguments_and_context(self):
            args = {"command": "cargo test", "workdir": "/tmp", "env": {"CUSTOM": "1"}, "background": True, "timeout": 11}
            original = copy.deepcopy(args)
            with patch("subprocess.run", return_value=response()) as run:
                self.assertIsNone(_pre_tool_call("terminal", args, session_id="session", tool_call_id="call"))
            self.assertEqual(args, {**original, "command": "rch exec -- cargo test"})
            event = json.loads(run.call_args.kwargs["input"])
            self.assertEqual(event["tool_input"], original)
            self.assertEqual(event["cwd"], os.path.realpath("/tmp"))
            self.assertEqual(event["session_id"], "session")
            self.assertEqual(event["tool_invocation_id"], "call")
            self.assertEqual(run.call_args.kwargs["timeout"], 3)
            self.assertEqual(run.call_args.args[0], ["rch"])

        def test_compound_rewrite_does_not_require_auto_approval(self):
            args = {"command": "cd /tmp && cargo test"}
            with patch("subprocess.run", return_value=response("cd /tmp && rch exec -- cargo test", None)):
                self.assertIsNone(_pre_tool_call("terminal", args))
            self.assertEqual(args["command"], "cd /tmp && rch exec -- cargo test")

        def test_pass_through_and_failures_preserve_exact_args(self):
            outputs = [
                types.SimpleNamespace(returncode=0, stderr="", stdout=""),
                types.SimpleNamespace(returncode=1, stderr="failed", stdout=""),
                types.SimpleNamespace(returncode=0, stderr="", stdout="not json"),
                types.SimpleNamespace(returncode=0, stderr="", stdout="[]"),
                types.SimpleNamespace(returncode=0, stderr="", stdout='{"hookSpecificOutput": null}'),
                response(permission="deny"), response(command=""),
                FileNotFoundError("rch"), subprocess.TimeoutExpired("rch", 3),
            ]
            for output in outputs:
                with self.subTest(output=output):
                    args = {"command": "  cargo test\n", "env": {"KEEP": "yes"}, "timeout": 10}
                    original = copy.deepcopy(args)
                    mock = Mock(side_effect=output) if isinstance(output, Exception) else Mock(return_value=output)
                    with patch("subprocess.run", mock):
                        self.assertIsNone(_pre_tool_call("terminal", args))
                    self.assertEqual(args, original)

        def test_nonterminal_and_already_wrapped_never_invoke_hook(self):
            with patch("subprocess.run") as run:
                self.assertIsNone(_pre_tool_call("read_file", {}))
                for command in ["rch exec -- cargo test", "'/opt/my tools/rch' exec -- cargo test", "remote-test cargo test"]:
                    args = {"command": command}
                    self.assertIsNone(_pre_tool_call("terminal", args))
                    self.assertEqual(args["command"], command)
                run.assert_not_called()

        def test_malformed_input_blocks_only_missing_valid_command(self):
            for args in [None, [], {}, {"command": " "}, {"command": 1}]:
                self.assertEqual(_pre_tool_call("terminal", args)["action"], "block")

        def test_cwd_precedence(self):
            terminal = types.ModuleType("tools.terminal_tool")
            terminal.get_session_cwd = Mock(return_value="/session")
            terminal._get_env_config = Mock(return_value={"cwd": "/configured"})
            with patch.dict(sys.modules, {"tools.terminal_tool": terminal}), patch.dict(os.environ, {"TERMINAL_CWD": "/environment"}):
                self.assertEqual(_resolved_cwd({"workdir": "/explicit"}, "task"), "/explicit")
                self.assertEqual(_resolved_cwd({}, "task"), "/session")
                terminal.get_session_cwd.assert_called_with("task")
                terminal.get_session_cwd.return_value = ""
                self.assertEqual(_resolved_cwd({}, "task"), "/configured")
                terminal._get_env_config.return_value = {}
                self.assertEqual(_resolved_cwd({}, "task"), "/environment")

    unittest.main()

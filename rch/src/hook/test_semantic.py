"""Run with python3 -I rch/src/hook/test_semantic.py (no pip or sibling imports).

Execution fixtures intentionally remain in /tmp for inspection; repository
policy forbids agents deleting even their own generated files.
"""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
PATH = Path(__file__).with_name("semantic.py")
SPEC = importlib.util.spec_from_file_location("rch_semantic", PATH)
semantic = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = semantic
SPEC.loader.exec_module(semantic)


def rewrite(command):
    encoded = command.encode("utf-8")
    edits = semantic.plan(command)
    for edit in reversed(edits):
        assert encoded[edit["start"]:edit["end"]].decode() == edit["word"]
        prefix = "rch exec --verification"
        for name in edit["env"]:
            prefix += " --forward-env " + name
        encoded = encoded[:edit["start"]] + (prefix + " -- ").encode() + encoded[edit["start"]:]
    return encoded.decode()


class PlannerTests(unittest.TestCase):
    def test_node_leading_options_reach_shared_verification_without_rewriting_arguments(self):
        for command in (
            "node --import 'setup with spaces.mjs' --test 'literal ; $HOME' ''",
            "node --require setup.cjs --test tests/unit.cjs",
            "node -r setup.cjs --test", "node -rsetup.cjs --test",
            "node --loader custom-loader --test",
            "node --experimental-loader custom-loader --test",
            "node --require=setup.cjs --import=setup.mjs --loader=loader --test",
            "node --no-warnings --trace-warnings --enable-source-maps --test",
            "node --test-name-pattern='some test' --test-skip-pattern slow --test",
            "node --test-concurrency 2 --test-timeout=5000 --test-shard 1/2 --test",
            "node --test-only --test-force-exit --test-reporter tap --test=tests/unit.cjs",
            "/usr/bin/node --import setup.mjs --test",
        ):
            for prefix in ("", "rtk proxy ", "env CI=1 "):
                source = prefix + command
                self.assertEqual(rewrite(source), "rch exec --verification -- " + source, source)
            nested = "rtk run -c " + shlex.quote(command)
            self.assertEqual(rewrite(nested), "rch exec --verification -- " + nested)

    def test_node_prefix_values_and_script_boundaries_cannot_fake_test_mode(self):
        for command in (
            "node app.js --test", "node --require setup.cjs app.js --test",
            "node --require --test app.js", "node --import --test", "node -r --test",
            "node --test-name-pattern --test", "node --import= --test",
            "node --loader --test", "node --test-reporter custom.mjs --test",
            "node --test-reporter --test", "node --test=", "node -- --test",
            "node --watch --test", "node --unknown-option --test",
            "node --no-warnings=yes --test", "node --import $MODULE --test",
            "node --eval 'console.log(1)' --test", "node --print '1' --test",
        ):
            self.assertEqual(rewrite(command), command, command)

    def test_token_storage_remains_immutable_and_preserves_default_operator(self):
        word = semantic.Word("value", "'value'", 0, 7)
        self.assertEqual((word.value, word.raw, word.start, word.end, word.operator),
                         ("value", "'value'", 0, 7, False))
        with self.assertRaises(AttributeError):
            word.value = "mutated"
        self.assertTrue(semantic.Word("&&", "&&", 8, 10, True).operator)
        self.assertEqual(semantic.lex("'value' && pytest")[0], word)

    def test_embedded_helper_needs_no_site_initialization(self):
        source = "pytest -q && cargo test"
        # Exercise the production -I -S -c form, including stdin EOF, rather
        # than merely importing the module in the already-running test process.
        helper = PATH.read_text()
        for mode in (None, "hook"):
            request = {"command": source}
            if mode:
                request["mode"] = mode
            result = subprocess.run([sys.executable, "-I", "-S", "-c", helper],
                                    input=json.dumps(request).encode(),
                                    capture_output=True, timeout=5, check=True)
            response = json.loads(result.stdout)
            self.assertEqual(response["version"], 2 if mode else 1)
            self.assertEqual(response["insertions"], semantic.plan(source, hook=bool(mode)))
            self.assertEqual(result.stderr, b"")

    def test_hook_components_cover_both_orders_without_reconstructing_shell(self):
        for source in ("pytest -q && cargo test", "cargo test && pytest -q",
                       "printf '中文'; /usr/bin/clang -c 'two words.c' && pytest -q"):
            raw = source.encode()
            edits = semantic.plan(source, hook=True)
            expected = 3 if source.startswith("printf") else 2
            self.assertEqual(len(edits), expected)
            for edit in edits:
                self.assertEqual(raw[edit["start"]:edit["end"]].decode(), edit["word"])
                self.assertTrue(raw[edit["start"]:edit["command_end"]].startswith(edit["word"].encode()))
                self.assertTrue(edit["native_eligible"])
                self.assertEqual(edit["env"], [])
            # Generic/shim responses retain the v1 fields and behaviour.
            for edit in semantic.plan(source):
                self.assertEqual(set(edit), {"start", "end", "word", "env"})

    def test_hook_native_eligibility_preserves_stream_and_environment_contracts(self):
        for source in ("cargo test | tee log", "cargo test >log && pytest",
                       "(cargo test); pytest", "cargo test & pytest"):
            edits = semantic.plan(source, hook=True)
            cargo = next(edit for edit in edits if edit["word"] == "cargo")
            self.assertFalse(cargo["native_eligible"], source)
            self.assertTrue(cargo["verification"], source)
        for source in ("X='a b' cargo test && pytest", "export X='a b'; cargo test"):
            cargo = next(edit for edit in semantic.plan(source, hook=True) if edit["word"] == "cargo")
            self.assertFalse(cargo["native_eligible"])
            self.assertEqual(cargo["env"], ["X"])
        source = "rtk run 'pytest && cargo test' && cargo test"
        edits = semantic.plan(source, hook=True)
        self.assertEqual([edit["word"] for edit in edits], ["rtk", "cargo"])
        self.assertFalse(edits[0]["native_eligible"])
        self.assertTrue(edits[0]["verification"])
        self.assertTrue(edits[1]["native_eligible"])

    def test_hook_mode_has_no_partial_native_plan_for_unsupported_grammar(self):
        for source in ("eval 'pytest' && cargo test", "if true; then pytest; fi && cargo test",
                       "export X=1 && pytest; cargo test", "pytest <<EOF && cargo test",
                       "pytest\ncargo test", "alias pytest=false; cargo test"):
            self.assertEqual(semantic.plan(source, hook=True), [], source)

    def test_hook_argv_spans_exclude_only_redirections_not_authored_arguments(self):
        source = "cargo >'two words.log' test --features 'a b' && pytest"
        edits = semantic.plan(source, hook=True)
        raw = source.encode()
        self.assertEqual([raw[arg["start"]:arg["end"]].decode()
                          for arg in edits[0]["argv"]],
                         ["cargo", "test", "--features", "'a b'"])
        self.assertFalse(edits[0]["native_eligible"])
        self.assertTrue(edits[0]["verification"])
        self.assertEqual(edits[1]["word"], "pytest")
        for source in ("exec -a alternate cargo test && pytest",
                       "time -p cargo test && pytest", "! ! cargo test && pytest"):
            self.assertNotIn("!", [edit["word"] for edit in semantic.plan(source, hook=True)])
            self.assertNotIn("time", [edit["word"] for edit in semantic.plan(source, hook=True)])
            self.assertNotIn("-a", [edit["word"] for edit in semantic.plan(source, hook=True)])

    def test_hook_wire_mode_is_explicit_and_keeps_legacy_schema(self):
        for request, version in (({"command": "pytest && cargo test"}, 1),
                                 ({"command": "pytest && cargo test", "mode": "hook"}, 2)):
            response = subprocess.run([sys.executable, "-I", str(PATH)],
                                      input=json.dumps(request).encode(), capture_output=True,
                                      timeout=5, check=True)
            self.assertEqual(json.loads(response.stdout)["version"], version)
        bad = subprocess.run([sys.executable, "-I", str(PATH)],
                             input=b'{"command":"cargo test","mode":"unknown"}',
                             capture_output=True, timeout=5)
        self.assertNotEqual(bad.returncode, 0)
        self.assertEqual(bad.stdout, b"")

    def test_nested_exports_and_conditionals_have_one_shell_owner(self):
        inner = (
            "export GOTOOLCHAIN=local GOSUMDB=off GOFLAGS=-mod=mod "
            "GOPROXY=file:///fixture/module-proxy "
            "RCH_ACCEPT_LITERAL='literal value ; $HOME 中文'; "
            "go test -v -count=1 ./... && node --test probe.test.cjs"
        )
        for source in (inner, "false && export X=wrong; go test || node --test probe.test.cjs"):
            # Top-level exports still require branch-sensitive forwarding.
            self.assertEqual(rewrite(source), source)
            for prefix in ("rtk proxy sh -c ", "rtk proxy bash -c ", "rtk run -c ", "rtk run "):
                command = prefix + shlex.quote(source)
                self.assertEqual(rewrite(command), "rch exec --verification -- " + command)
                self.assertEqual(semantic.plan(command)[0]["env"], [])
        for source in (
            "(export X=1); go test", "export X=1 | go test",
            "export X=1 & go test", "export X=1; go test | node --test",
            "export -n X; go test && node --test", "export X=1; eval 'go test'",
            "export X=1; if true; then go test; fi",
        ):
            command = "rtk proxy sh -c " + shlex.quote(source)
            self.assertEqual(rewrite(command), command)

    def test_rtk_literal_export_quoted_segments(self):
        inner = (
            "export GOTOOLCHAIN=local GOSUMDB=off GOFLAGS=-mod=mod "
            "GOPROXY=file:///mnt/vibe-coding-share/scratch/"
            "rch-rtk-live-01a1387e-20261015T044808Z-00d4c2/module-proxy "
            "RCH_ACCEPT_LITERAL='literal value ; $HOME 中文'; "
            "/usr/local/go/bin/go test -v -count=1 ./..."
        )
        for prefix in ("rtk run -c ", "rtk run ", "rtk proxy sh -c ", "rtk proxy bash -c "):
            command = prefix + shlex.quote(inner)
            with self.subTest(command=command):
                self.assertEqual(rewrite(command), "rch exec --verification -- " + command)
                self.assertEqual(shlex.split(command)[-1], inner)
        # Shell itself is the oracle for lexical decoding. These are only
        # positional data, never executed as shell source by this check.
        for value in (inner, "'go' test", 'literal \\" quote', "'$HOME' ; `data` * [x] {a,b} ~ 中文"):
            raw = shlex.quote(value)
            words = semantic.lex(raw)
            self.assertEqual(len(words), 1)
            self.assertTrue(semantic.literal(words[0]), raw)
            actual = subprocess.check_output(["/bin/sh", "-c", "printf '%s' " + raw])
            self.assertEqual(words[0].value.encode(), actual)
            self.assertEqual(actual, value.encode())

    def test_rtk_dynamic_and_unsupported_shell_sources_still_miss(self):
        for command in (
            'rtk run -c "export X=\'$HOME\'; go test"',
            "rtk run -c 'go test '$SUFFIX",
            "rtk run -c 'go test '$(printf ./...)",
            "rtk run -c 'go test '`printf ./...`",
            "rtk run -c $'go test'",
            "rtk run -c *.sh",
            "rtk run -c 'go test",  # malformed quote
            "rtk run -c " + shlex.quote('export X=x; "$TOOL" test'),
            "rtk proxy sh -c " + shlex.quote("if true; then go test; fi"),
            "rtk proxy sh -c " + shlex.quote("eval 'go test'"),
            "rtk run -c " + shlex.quote("printf '%s' 'go test $HOME'"),
        ):
            with self.subTest(command=command):
                self.assertEqual(rewrite(command), command)

    def test_rtk_046_shell_proxy_and_filtered_workloads(self):
        for command in (
            "rtk run -c 'cargo build --release'",
            "rtk run --command='go test ./...'",
            "rtk run 'export A=literal; /usr/local/go/bin/go test ./...'",
            "rtk run go test ./...",
            "rtk proxy sh -c 'go test ./... && node --test probe.test.cjs'",
            "rtk --ultra-compact cargo test --offline -- --nocapture",
            "rtk go test ./...", "rtk go build ./...",
            "rtk test bun test", "rtk err bun build entry.ts", "rtk summary bun build entry.ts",
            "rtk proxy /usr/local/go/bin/go test ./...",
            "rtk test /opt/bun/bin/bun test", "rtk cargo build --release",
        ):
            with self.subTest(command=command):
                self.assertEqual(rewrite(command), "rch exec --verification -- " + command)
        for command in (
            "rtk run -c 'printf cargo; exit 7'", "rtk proxy git status",
            "rtk run 'echo go test'", "rtk go --help", "rtk cargo --version",
        ):
            self.assertEqual(rewrite(command), command)

    def test_sibling_families_and_rch_extensions(self):
        commands = (
            "go test ./...", "go vet ./...", "go build ./cmd/server",
            "npm test", "npm --prefix web run build", "npm --prefix=web run test:unit",
            "pnpm run typecheck", "pnpm --filter web test", "pnpm -r run lint",
            "bun test", "bun run build", "bun typecheck", "bun x tsc --noEmit",
            "node --test test/example.test.js", "node --test=foo",
            "python3 -m pytest -q", "python -m unittest discover", "python3.12 -I -m pytest",
            "uv run pytest -q", "uv run --project web python -m unittest",
            "cargo test --locked", "cargo check --locked", "cargo build --locked",
            "cargo clippy --locked", "cargo +nightly test",
            "make test", "make test-unit", "make -C web check",
            "/usr/local/go/bin/go test ./...", "PATH=$HOME/bin:$PATH go test ./...",
            "swift test --parallel", "swift build -c release", "rtk swift test",
            "tools/rtk proxy swift build -c debug", "env -C . go test ./...",
            "env SECRET=value go test ./...", 'env X="$VALUE" go test ./...', "command go test ./...",
            "exec pytest -q", "bash -c 'go test ./...'", "sh -lc 'cd web && npm test'",
            "timeout --kill-after=5s 30s go test ./...", "nice -n 3 pytest",
            "npx --package jest@30 jest fixture.test.cjs", "npm exec -- tsc --noEmit",
            "pnpm exec playwright test", "bunx vitest run", "prettier --check .",
            "vite build", "eslint .", "./scripts/run_tests.sh",
        )
        for command in commands:
            with self.subTest(command=command):
                self.assertEqual(len(semantic.plan(command)), 1)
                self.assertIn("rch exec --verification", rewrite(command))

    def test_unknown_data_opaque_and_already_rewritten_unchanged(self):
        commands = (
            "git status", "printf 'pytest'", "echo go test", "npm install", "bun dev",
            "python app.py", "uv sync", "swift run", "prettier --write .",
            "rch exec --verification -- pytest", "env X=1 rch exec -- pytest",
            "go $OP ./...", "${TOOL} test", "'go test'", "command -v pytest",
            "exec -a other pytest", "if true; then pytest; fi",
            "for x in 1; do pytest; done", "{ pytest; }", "f() { pytest; }; f",
            "alias pytest=other; pytest", "eval 'pytest'", "source x; pytest",
            "pytest <(printf x)", "pytest <<EOF\nx\nEOF", "pytest # comment",
            "pytest\necho done", "pytest &&", "pytest 'broken", "pytest >",
            "pytest $(unterminated", "pytest | | echo x", "(pytest", "pytest )",
            "printf $(pytest)", "bash -c \"pytest $ARGS\"",
        )
        for command in commands:
            with self.subTest(command=command):
                self.assertEqual(rewrite(command), command)

    def test_exact_authored_structure_and_spans(self):
        command = 'printf é; cd "web dir" && X="a b" pytest 2>&1 | tee out; go test ./...>last &'
        self.assertEqual(
            rewrite(command),
            'printf é; cd "web dir" && X="a b" rch exec --verification --forward-env X -- pytest 2>&1 | tee out; rch exec --verification -- go test ./...>last &',
        )
        self.assertEqual(rewrite("2>err pytest -q"), "2>err rch exec --verification -- pytest -q")
        self.assertEqual(rewrite("pytest >out -q"), "rch exec --verification -- pytest >out -q")
        self.assertEqual(rewrite("(pytest && go test)"), "(rch exec --verification -- pytest && rch exec --verification -- go test)")

    def test_planning_is_pure_and_isolated(self):
        fixture = Path(tempfile.mkdtemp(prefix="rch-semantic-isolation-"))
        marker = fixture / "must-not-exist"
        command = f"pytest $(touch {marker})"
        response = subprocess.run(
            [sys.executable, "-I", "-c", PATH.read_text()],
            input=json.dumps({"command": command}), text=True, capture_output=True,
            cwd=fixture, timeout=3, check=True,
            env={**os.environ, "PYTHONPATH": str(fixture)},
        )
        self.assertEqual(json.loads(response.stdout)["version"], 1)
        self.assertEqual(len(json.loads(response.stdout)["insertions"]), 1)
        self.assertFalse(marker.exists(), "planning must never evaluate substitutions")
        self.assertEqual(sorted(p.name for p in fixture.iterdir()), [])

    def test_exports_with_ambiguous_scope_stay_unchanged(self):
        for command in (
            "export X=1 && pytest", "(export X=1); pytest", "export X=1 | pytest",
            "false && export X=1; pytest", "export -n X; pytest",
            "export $NAMES; pytest", 'export "X=$VALUE"; pytest',
            "export X=1; unset X; pytest", "set -a; X=1; pytest",
            "declare -x X=1; pytest", "command export X=1; pytest",
            "command -- export TEST_X=expected; pytest -q",
            "command -p export TEST_X=expected; pytest -q",
            "command -- command -- export TEST_X=expected; pytest -q",
            "X=1 export X; pytest", "export X+; pytest",
        ):
            with self.subTest(command=command):
                self.assertEqual(rewrite(command), command)

    def test_limits(self):
        self.assertEqual(semantic.plan("pytest " + "x" * semantic.MAX_INPUT), [])
        self.assertEqual(semantic.plan("(" * 30 + "pytest" + ")" * 30), [])
        self.assertEqual(semantic.plan("pytest " + "x " * 5000), [])
        self.assertEqual(semantic.plan("pytest\0bad"), [])
        self.assertEqual(semantic.plan("X=\udcff pytest"), [])


TOOL = r'''#!/usr/bin/python3 -I
import json, os, sys
args = sys.argv[1:]
record = {"tool": os.path.basename(sys.argv[0]), "args": args, "cwd": os.getcwd(), "X": os.environ.get("X"), "TEST_X": os.environ.get("TEST_X")}
if "--stdin" in args:
    record["stdin"] = sys.stdin.read()
print(json.dumps(record, sort_keys=True), flush=True)
print("fixture-stderr", file=sys.stderr, flush=True)
if "--fail" in args:
    sys.exit(7)
'''

FAKE_RCH = r'''#!/usr/bin/python3 -I
import json, os, sys
args = sys.argv[1:]
assert args[:2] == ["exec", "--verification"], args
args = args[2:]
forwarded = {}
while args[:1] == ["--forward-env"]:
    if args[1] in os.environ:
        forwarded[args[1]] = os.environ[args[1]]
    args = args[2:]
assert args[:1] == ["--"], args
args = args[1:]
with open(os.environ["RCH_FIXTURE_LOG"], "a") as log:
    log.write(json.dumps({"argv": args, "forwarded": forwarded, "cwd": os.getcwd()}) + "\n")
# Simulate an isolated worker rather than inheriting local state and hiding
# missing --forward-env declarations. PATH is the fixture's toolchain allowlist.
os.execvpe(args[0], args, {"PATH": os.environ["PATH"], **forwarded})
'''


class ExecutionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = Path(tempfile.mkdtemp(prefix="rch-semantic-execution-"))
        cls.bin = cls.root / "bin"
        cls.bin.mkdir()
        cls.work = cls.root / "web dir"
        cls.work.mkdir()
        for name in ("one.go", "two.go"):
            (cls.work / name).write_text("")
        for name in ("go", "pytest", "npm", "swift"):
            path = cls.bin / name
            path.write_text(TOOL)
            path.chmod(0o755)
        (cls.bin / "rch").write_text(FAKE_RCH)
        (cls.bin / "rch").chmod(0o755)
        cls.log = cls.root / "invocations.jsonl"
        cls.env = {**os.environ, "PATH": str(cls.bin) + ":/usr/bin:/bin", "PACKAGE": "space name", "RCH_FIXTURE_LOG": str(cls.log)}
        print("Execution fixtures retained at", cls.root, file=sys.stderr)

    def run_shell(self, command):
        return subprocess.run(["/bin/bash", "--noprofile", "--norc", "-c", command], cwd=self.root,
                              env=self.env, capture_output=True, text=True, timeout=5)

    def assert_execution_equal(self, command):
        transformed = rewrite(command)
        self.assertNotEqual(transformed, command)
        before = self.run_shell(command)
        after = self.run_shell(transformed)
        self.assertEqual((after.returncode, after.stdout, after.stderr),
                         (before.returncode, before.stdout, before.stderr), transformed)
        return after

    def test_argv_expansions_cwd_and_inline_env(self):
        command = 'cd "web dir" && X="a b" pytest "$PACKAGE" *.go "$(printf subst)" "" ";" "2>literal"'
        result = self.assert_execution_equal(command)
        data = json.loads(result.stdout)
        self.assertEqual(data["args"], ["space name", "one.go", "two.go", "subst", "", ";", "2>literal"])
        self.assertEqual(data["cwd"], str(self.work))
        self.assertEqual(data["X"], "a b")
        records = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(records[-1]["forwarded"], {"X": "a b"})

    def test_exported_environment_survives_isolated_worker(self):
        for command, expected in (
            ("export TEST_X=expected; pytest -q", "expected"),
            ('export TEST_X="$PACKAGE"; pytest -q', "space name"),
            ("TEST_X=before; export TEST_X; pytest -q", "before"),
            ("export TEST_X=first; TEST_X=second; pytest -q", "second"),
            ("export TEST_X=first; TEST_X=inline pytest -q", "inline"),
            ("export TEST_X=before; cd 'web dir'; pytest -q", "before"),
        ):
            with self.subTest(command=command):
                result = self.assert_execution_equal(command)
                self.assertEqual(json.loads(result.stdout)["TEST_X"], expected)
                records = [json.loads(line) for line in self.log.read_text().splitlines()]
                self.assertEqual(records[-1]["forwarded"]["TEST_X"], expected)

    def test_stdin_stdout_stderr_pipelines(self):
        self.assert_execution_equal("printf 'input data' | pytest --stdin 2>&1 | /usr/bin/sort")
        self.assert_execution_equal("pytest |& /usr/bin/sort")
        self.assert_execution_equal("pytest 2 >number.out; /bin/cat number.out")
        self.assert_execution_equal("pytest 2>errors.out; /bin/cat errors.out")
        self.assert_execution_equal("pytest >out --stdin <<<'here data'; /bin/cat out")
        self.assert_execution_equal("2>before.err pytest >after.out; /bin/cat before.err after.out")

    def test_exit_operators_groups_and_multiple_components(self):
        self.assert_execution_equal("pytest --fail && printf WRONG || printf fallback; printf ':%s' $?")
        self.assert_execution_equal("(cd 'web dir' && pytest); go test ./...")
        self.assert_execution_equal("false && pytest; printf done")
        self.assert_execution_equal("! pytest --fail")
        self.assert_execution_equal("command pytest 'quoted value'")
        self.assert_execution_equal("exec pytest 'quoted value'")

    def test_nested_shell_has_explicit_argv_and_preserves_positional_args(self):
        self.assert_execution_equal("bash -c 'cd \"web dir\" && pytest \"$1\"' fixture 'two words'")
        records = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(records[-1]["argv"], ["bash", "-c", 'cd "web dir" && pytest "$1"', "fixture", "two words"])

    def test_nested_exports_short_circuit_and_side_effects(self):
        for inner in (
            "export X='literal value ; $HOME 中文'; pytest && go test",
            "export X='literal value ; $HOME 中文'; pytest --fail && go test",
            "false && export X=wrong; pytest --fail || go test",
            "export X='literal value ; $HOME 中文'; cd 'web dir'; pytest && printf shell-side-effect",
        ):
            command = "sh -c " + shlex.quote(inner)
            before = len(self.log.read_text().splitlines()) if self.log.exists() else 0
            self.assert_execution_equal(command)
            records = [json.loads(line) for line in self.log.read_text().splitlines()]
            self.assertEqual(len(records), before + 1)
            self.assertEqual(records[-1]["argv"], ["sh", "-c", inner])
            self.assertEqual(records[-1]["forwarded"], {})

    def test_env_and_timeout_wrappers(self):
        self.assert_execution_equal("env -C 'web dir' X='a b' pytest --stdin <<<'input'")
        self.assert_execution_equal("timeout --preserve-status 3s pytest --fail")


if __name__ == "__main__":
    unittest.main(verbosity=2)

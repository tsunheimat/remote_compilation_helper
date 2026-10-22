"""RCH-owned, stdlib-only shell structure and verification planner.

Adapted from remote-test's AST/semantic/policy separation. This is not a shell
interpreter or permission classifier. Only executable-word insertion spans are
returned; the Rust caller owns the replacement text. Authored shell bytes,
expansion, cwd changes, operators and redirects remain in the invoking shell.
Unsupported grammar is an optimization miss, never permission to execute.
Only command-authored prefix assignments and simple exports request explicit
forwarding. Ambient inherited variables follow the executor's configured remote
environment allowlist; this planner never copies the entire process environment
or broadens that allowlist (which could expose credentials).
"""
from __future__ import annotations

import json
import os
import re
import shlex
import sys
from dataclasses import dataclass

MAX_INPUT = 65536
MAX_TOKENS = 4096
MAX_DEPTH = 24
SEPARATORS = {";", "&&", "||", "|", "|&", "&", "(", ")"}
REDIRECTS = {">", ">>", "<", "<<<", "<>", ">&", "<&", ">|", "&>", "&>>"}
CONTROL = {"if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "select", "function", "{", "}", "[[", "]]", "coproc"}
ASSIGNMENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\+?=.*", re.S)
SCRIPT = re.compile(r"(?:test|build|lint|typecheck|check|verify|vet)(?:[:.-].*)?\Z")
PYTHON = re.compile(r"python(?:3(?:\.\d+)?)?\Z")


class ParseMiss(ValueError):
    """Shell syntax cannot be preserved by this bounded structure parser."""


@dataclass(frozen=True)
class Word:
    value: str
    raw: str
    start: int
    end: int
    operator: bool = False


def expansion_end(source: str, start: int, depth: int = 0) -> int:
    """Skip an opaque substitution without interpreting any of its contents."""
    if depth > MAX_DEPTH:
        raise ParseMiss("expansion nesting limit")
    if source[start] == "`":
        i = start + 1
        while i < len(source):
            if source[i] == "\\":
                i += 2
            elif source[i] == "`":
                return i + 1
            else:
                i += 1
        raise ParseMiss("unterminated backtick")
    closing = ")" if source[start + 1] == "(" else "}"
    opening = source[start + 1]
    nesting = 1
    quote = None
    i = start + 2
    while i < len(source):
        c = source[i]
        if quote == "'":
            if c == "'":
                quote = None
            i += 1
            continue
        if c == "\\":
            i += 2
            continue
        if c == "`" or source.startswith("$(", i) or source.startswith("${", i):
            i = expansion_end(source, i, depth + 1)
            continue
        if quote == '"':
            if c == '"':
                quote = None
        elif c in "'\"":
            quote = c
        elif c == opening:
            nesting += 1
        elif c == closing:
            nesting -= 1
            if not nesting:
                return i + 1
        i += 1
    raise ParseMiss("unterminated expansion")


def decode(raw: str) -> str:
    # shlex is used ONLY for lexical decoding; opaque substitutions may contain
    # whitespace and are intentionally never used as command-family evidence.
    try:
        words = shlex.split(raw, comments=False, posix=True)
        return words[0] if len(words) == 1 else raw
    except ValueError:
        return raw


def lex(source: str) -> list[Word]:
    result = []
    i = 0
    operators = ("&>>", "<<<", "&&", "||", "|&", ">>", "<<", ">&", "<&", "<>", ">|", "&>")
    while i < len(source):
        if len(result) >= MAX_TOKENS:
            raise ParseMiss("token limit")
        c = source[i]
        if c in " \t\r":
            i += 1
            continue
        if c in "\n#":
            raise ParseMiss("newline/comment grammar unsupported")
        if source.startswith(("<(", ">("), i):
            raise ParseMiss("process substitution needs local descriptors")
        start = i
        if c in ";&|<>()":
            op = next((op for op in operators if source.startswith(op, i)), c)
            if op == "<<":
                raise ParseMiss("heredoc grammar unsupported")
            i += len(op)
            result.append(Word(op, op, start, i, True))
            continue
        quote = None
        while i < len(source):
            c = source[i]
            if quote == "'":
                i += 1
                if c == "'":
                    quote = None
                continue
            if c == "\\":
                if i + 1 == len(source):
                    raise ParseMiss("trailing escape")
                i += 2
                continue
            if c == "`" or source.startswith("$(", i) or source.startswith("${", i):
                i = expansion_end(source, i)
                continue
            if quote == '"':
                if c == '"':
                    quote = None
                i += 1
                continue
            if source.startswith(("<(", ">("), i):
                raise ParseMiss("process substitution needs local descriptors")
            if c.isspace() or c in ";&|<>()":
                break
            if c in "'\"":
                quote = c
            i += 1
        if quote:
            raise ParseMiss("unterminated quote")
        raw = source[start:i]
        if not raw:
            raise ParseMiss("unsupported whitespace")
        result.append(Word(decode(raw), raw, start, i))
    return result


def invocations(tokens: list[Word]) -> list[list[Word]]:
    """Validate sequence/pipeline/group structure and retain word spans."""
    result, current = [], []
    depth = 0
    expect_command = True
    previous = None
    for token in tokens:
        if token.operator and token.value in SEPARATORS:
            if current:
                result.append(current)
                current = []
                expect_command = False
            op = token.value
            if op == "(":
                if not expect_command:
                    raise ParseMiss("function or malformed group")
                depth += 1
                if depth > MAX_DEPTH:
                    raise ParseMiss("group nesting limit")
            elif op == ")":
                if depth == 0 or (expect_command and previous not in {";", "&"}):
                    raise ParseMiss("malformed closing group")
                depth -= 1
                expect_command = False
            else:
                if expect_command:
                    raise ParseMiss("empty command")
                expect_command = True
            previous = op
        else:
            if not expect_command and not current:
                raise ParseMiss("unsupported group suffix")
            current.append(token)
            previous = None
    if current:
        result.append(current)
        expect_command = False
    if depth or (expect_command and previous not in {";", "&", None}):
        raise ParseMiss("incomplete shell structure")
    return result


def arguments(tokens: list[Word]) -> list[Word]:
    """Ignore redirections for classification, without moving a single byte."""
    kept = []
    i = 0
    while i < len(tokens):
        token = tokens[i]
        if token.operator:
            if token.value not in REDIRECTS or i + 1 == len(tokens) or tokens[i + 1].operator:
                raise ParseMiss("invalid redirection")
            # An IO number must be unquoted and lexically adjacent: `2 >x`
            # passes argument 2, whereas `2>x` selects stderr.
            if kept and kept[-1].raw.isdigit() and kept[-1].end == token.start:
                kept.pop()
            elif kept and re.fullmatch(r"\{[A-Za-z_][A-Za-z0-9_]*\}", kept[-1].raw) and kept[-1].end == token.start:
                raise ParseMiss("dynamic descriptor binding")
            i += 2
        else:
            kept.append(token)
            i += 1
    return kept


def literal(word: Word) -> bool:
    # A shell word may concatenate quoted segments, notably shlex.quote's
    # '... '"'"' ...' encoding of an embedded apostrophe. Decide whether magic
    # characters are active at THIS shell layer before inspecting nested source.
    # Never evaluate expansions to obtain command-family evidence.
    quote = None
    i = 0
    while i < len(word.raw):
        c = word.raw[i]
        if quote == "'":
            if c == "'":
                quote = None
        elif c == "\\":
            i += 1
            # Retain the existing conservative refusal for escaped expansion
            # syntax too; shlex does not decode all double-quote escapes like sh.
            if i == len(word.raw) or word.raw[i] in "$`*?[]{}~\n":
                return False
        elif quote == '"':
            if c == '"':
                quote = None
            elif c in "$`":
                return False
        elif c in "'\"":
            quote = c
        elif c in "$`*?[]{}~":
            return False
        i += 1
    return quote is None


def family(words: list[Word], depth: int = 0) -> bool:
    if not words or depth > MAX_DEPTH or not literal(words[0]):
        return False
    exe = os.path.basename(words[0].value)
    args = words[1:]
    values = [w.value if literal(w) else "" for w in args]
    if exe == "rch":
        return False
    if exe == "rtk":
        # RTK 0.46: run joins positional args and executes sh -c; proxy
        # preserves argv. Inspect only literal shell source and leave the
        # entire authored RTK invocation intact for the worker to execute.
        while args and args[0].value in {"-v", "-vv", "-vvv", "--verbose", "--ultra-compact", "--skip-env"}:
            args = args[1:]
        if not args or not literal(args[0]):
            return False
        mode, payload = args[0].value, args[1:]
        while payload and payload[0].value in {"--ultra-compact", "--skip-env"}:
            payload = payload[1:]
        if payload and payload[0].value == "--":
            payload = payload[1:]
        if mode == "run":
            if not payload or not all(literal(word) for word in payload):
                return False
            if payload[0].value in {"-c", "--command"}:
                return len(payload) == 2 and bool(plan(payload[1].value, depth + 1, whole_shell=True))
            if payload[0].value.startswith("--command="):
                return len(payload) == 1 and bool(plan(payload[0].value.partition("=")[2], depth + 1, whole_shell=True))
            return bool(plan(" ".join(word.value for word in payload), depth + 1, whole_shell=True))
        if mode in {"proxy", "test", "err", "summary"}:
            return family(payload, depth + 1)
        return family(args, depth + 1)
    if exe in {"bash", "sh"}:
        if len(args) < 2 or values[0] not in {"-c", "-lc"} or not literal(args[1]):
            return False
        return bool(plan(args[1].value, depth + 1, whole_shell=True))
    if exe in {"command", "exec"}:
        # These are shell builtins, not executable argv wrappers. The outer
        # planner keeps them local, but they cannot be forwarded through env.
        return False
    if exe in {"env", "nice", "timeout"}:
        i = 0
        while i < len(args):
            value = values[i]
            if exe == "env" and ASSIGNMENT.fullmatch(args[i].value):
                # Assignment values are data, not classification evidence;
                # preserve dynamic expansion in the caller's original argv.
                i += 1
                continue
            if not value:
                return False
            if value == "--":
                i += 1
                break
            if exe == "env" and (ASSIGNMENT.fullmatch(args[i].value) or value in {"-i", "--ignore-environment"}):
                i += 1
            elif value in ({"-C", "--chdir", "-u", "--unset"} if exe == "env" else {"-n", "--adjustment"} if exe == "nice" else {"-s", "--signal", "-k", "--kill-after"}):
                i += 2
            elif value.startswith(("--chdir=", "--unset=")) and exe == "env":
                i += 1
            elif value.startswith("-") and exe in {"nice", "timeout"}:
                i += 1
            else:
                break
        if exe == "timeout":
            if i >= len(args) or not re.fullmatch(r"[0-9]+(?:\.[0-9]+)?[smhd]?", values[i]):
                return False
            i += 1
        return family(args[i:], depth + 1)
    if exe in {"go", "cargo", "swift"}:
        if exe == "cargo" and values and values[0].startswith("+"):
            values = values[1:]
        return bool(values and values[0] in {"go": {"test", "vet", "build"}, "cargo": {"test", "t", "check", "c", "build", "b", "clippy"}, "swift": {"test", "build"}}[exe])
    if exe == "pytest":
        return True
    if PYTHON.fullmatch(exe):
        i = 0
        while i < len(values) and values[i] in {"-I", "-B", "-E", "-s", "-S", "-u"}:
            i += 1
        return values[i:i + 2] in (["-m", "pytest"], ["-m", "unittest"])
    if exe == "uv":
        if values[:1] != ["run"]:
            return False
        i = 1
        while i < len(values) and values[i].startswith("-"):
            i += 2 if values[i] in {"--project", "--directory", "--package", "--python", "--with", "--group"} else 1
        return family(args[i:], depth + 1)
    if exe == "node":
        return bool(values and (values[0] == "--test" or values[0].startswith("--test=")))
    if exe in {"npm", "pnpm", "bun"}:
        i = 0
        while i < len(values):
            value = values[i]
            if value in {"--prefix", "--dir", "-C", "--filter", "--cwd", "--workspace", "-w"}:
                i += 2
            elif value.startswith(("--prefix=", "--dir=", "--filter=", "--cwd=", "--workspace=")) or value in {"-r", "--recursive", "--workspaces"}:
                i += 1
            else:
                break
        rest = values[i:]
        if not rest:
            return False
        if rest[0] == "test" or (exe in {"pnpm", "bun"} and SCRIPT.fullmatch(rest[0])):
            return True
        if rest[0] in {"run", "run-script"} and len(rest) > 1:
            return bool(SCRIPT.fullmatch(rest[1]))
        if rest[0] in {"exec", "x"}:
            return javascript(args[i + 1:], depth + 1)
        return False
    if exe == "make":
        targets, i = [], 0
        while i < len(values):
            v = values[i]
            if v in {"-C", "--directory", "-f", "--file", "-j", "--jobs"}:
                i += 2
                continue
            if not v:
                return False
            if not v.startswith("-") and "=" not in v:
                targets.append(v)
            i += 1
        return bool(targets) and all(SCRIPT.fullmatch(t) for t in targets)
    if words[0].value in {"scripts/run_tests.sh", "./scripts/run_tests.sh"}:
        return True
    return javascript(words, depth + 1, direct=True)


def javascript(words: list[Word], depth: int, direct: bool = False) -> bool:
    if not words or depth > MAX_DEPTH:
        return False
    i = 0
    if not direct:
        while i < len(words):
            value = words[i].value
            if value in {"-p", "--package"}:
                i += 2
            elif value.startswith("--package=") or value in {"--", "--no-install", "--yes", "-y"}:
                i += 1
            else:
                break
    if i >= len(words) or not literal(words[i]):
        return False
    exe = os.path.basename(words[i].value)
    args = [w.value if literal(w) else "" for w in words[i + 1:]]
    if exe in {"npx", "bunx"}:
        return javascript(words[i + 1:], depth + 1)
    if exe in {"vitest", "jest", "tsc", "eslint"}:
        return True
    if exe == "prettier":
        return "--check" in args or "-c" in args
    return bool(args and ((exe == "playwright" and args[0] == "test") or (exe == "vite" and args[0] == "build")))


def plan(source: str, depth: int = 0, *, whole_shell: bool = False) -> list[dict]:
    if depth > MAX_DEPTH or not isinstance(source, str) or "\0" in source:
        return []
    try:
        if len(source.encode("utf-8")) > MAX_INPUT:
            return []
        tokens = lex(source)
        commands = [arguments(parts) for parts in invocations(tokens)]
        # Export state belongs to a shell scope, not the flattened invocation
        # stream. Track only unconditional top-level sequences here. For a
        # literal nested shell, family() uses only the existence of a workload:
        # it ships the WHOLE shell unchanged, never these inner spans/env lists.
        # Its worker shell owns export state and short-circuit evaluation, so
        # flat conditional lists need no branch-sensitive env reconstruction.
        # Groups, pipelines and background exports remain conservative misses.
        for words in commands:
            prefix = 0
            while prefix < len(words):
                word = words[prefix]
                if ASSIGNMENT.fullmatch(word.raw) or word.value == "!":
                    prefix += 1
                elif word.value in {"command", "exec"}:
                    prefix += 1
                    # Keep shell lookup/exec options from concealing a stateful
                    # builtin (`command -- export`, `command -p export`).
                    while prefix < len(words) and words[prefix].value.startswith("-"):
                        option = words[prefix].value
                        prefix += 1
                        if option == "--":
                            break
                        if option == "-a" and word.value == "exec":
                            prefix += 1
                else:
                    break
            if prefix and prefix < len(words) and words[prefix].value in {"export", "unset", "declare", "typeset", "local", "readonly", "set", "builtin"}:
                return []
        has_exports = any(words and words[0].value == "export" for words in commands)
        export_separators = {";", "&&", "||"} if whole_shell else {";"}
        if has_exports and any(token.operator and token.value in SEPARATORS - export_separators for token in tokens):
            return []
        # Unsupported control syntax/stateful lookup is a whole-command miss.
        # It must not make a later body invocation look like an ordinary stage.
        for words in commands:
            if words and words[0].value in CONTROL | {"alias", "unalias", "function", "source", ".", "eval", "hash", "enable", "disable", "unset", "declare", "typeset", "local", "readonly", "set", "builtin"}:
                return []
        edits = []
        exported = []
        for words in commands:
            if words and words[0].value == "export":
                if len(words) == 1:
                    continue  # `export` alone only prints current state.
                for word in words[1:]:
                    # Recognize a literal binding name while allowing the value
                    # to expand normally in the unchanged local export command.
                    name = word.raw.split("=", 1)[0]
                    if "=" in word.raw:
                        name = name.rstrip("+")
                    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name):
                        return []
                    if "=" not in word.raw and not literal(word):
                        return []
                    if name not in exported:
                        exported.append(name)
                continue
            i = 0
            env = list(exported)
            while i < len(words) and ASSIGNMENT.fullmatch(words[i].raw):
                env.append(words[i].raw.split("=", 1)[0].rstrip("+"))
                i += 1
            if i < len(words) and words[i].value == "!" and words[i].raw == "!":
                i += 1
            while i < len(words) and words[i].value in {"command", "exec"} and literal(words[i]):
                builtin = words[i].value
                i += 1
                if i < len(words) and words[i].value == "--":
                    i += 1
                elif i < len(words) and words[i].value.startswith("-"):
                    # Query modes, exec -a, and altered lookup/env need more than
                    # a transparent prefix insertion; leave them untouched.
                    break
                if builtin == "exec" and i == len(words):
                    break
            if family(words[i:], depth):
                word = words[i]
                edits.append({"start": len(source[:word.start].encode("utf-8")), "end": len(source[:word.end].encode("utf-8")), "word": word.raw, "env": env})
        return edits
    except (ParseMiss, RecursionError, ValueError):
        return []


def main() -> None:
    raw = sys.stdin.buffer.read(MAX_INPUT * 6 + 1024)
    request = json.loads(raw)
    if not isinstance(request, dict) or set(request) != {"command"} or not isinstance(request["command"], str):
        raise ValueError("invalid planner input")
    print(json.dumps({"version": 1, "insertions": plan(request["command"])}, ensure_ascii=True, separators=(",", ":")))


if __name__ == "__main__":
    main()

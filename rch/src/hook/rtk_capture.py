"""RCH's worker-side RTK stream capture. Runs only inside an owned job group.

RTK remains the command/filter implementation. Tool proxies preserve its argv
and relay bytes while saving the input to its filters. No command is replayed.
The collector is read-only and bounded, and never follows output symlinks.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import stat
import subprocess
import sys
import uuid

LIMIT = 64 * 1024 * 1024
FILES = 256
NAME = re.compile(r"(?:outer|[a-f0-9-]{36})\.(?:stdout|stderr|json)\Z")


def directory(path, create=False):
    path = Path(path)
    assert path.is_absolute() and '..' not in path.parts
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:]:
            if create:
                try:
                    os.mkdir(part, 0o700, dir_fd=fd)
                except FileExistsError:
                    pass
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        info = os.fstat(fd)
        assert info.st_uid == os.geteuid() and not info.st_mode & 0o077, 'unowned capture directory'
        return fd
    except BaseException:
        os.close(fd)
        raise


def read_file(fd, name, maximum):
    child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=fd)
    with os.fdopen(child, 'rb') as stream:
        info = os.fstat(stream.fileno())
        assert stat.S_ISREG(info.st_mode) and info.st_nlink == 1
        assert info.st_uid == os.geteuid() and info.st_size <= maximum
        data = stream.read(maximum + 1)
        assert len(data) <= maximum
        return data


def write_new(fd, name, data, mode=0o600):
    child = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode, dir_fd=fd)
    with os.fdopen(child, 'wb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def checked_root(root, identity):
    fd = directory(root)
    assert read_file(fd, '.owner', 128).decode() == identity, 'capture ownership changed'
    return fd


def capture(fd, name, argv, environment):
    # One shared budget includes raw tool streams and outer filtered streams.
    # Exhaustion is a failed job, never a silently truncated successful log.
    budget = os.open('.budget', os.O_RDWR | os.O_NOFOLLOW, dir_fd=fd)
    outputs = {}
    fcntl.flock(budget, fcntl.LOCK_EX)
    # Reserve all three files before spawning. Concurrent tool invocations must
    # not silently exceed the collector's count limit.
    if len([entry for entry in os.listdir(fd) if NAME.fullmatch(entry)]) + 3 > FILES:
        write_new(fd, 'capture.error', b'RTK output exceeded the 256-file job budget; logs are incomplete')
        raise RuntimeError('output file budget exceeded')
    for suffix in ['stdout', 'stderr']:
        outputs[suffix] = os.open(name + '.' + suffix, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd)
    metadata = os.open(name + '.json', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd)
    os.write(metadata, json.dumps({'argv': argv, 'cwd': os.getcwd(), 'exit': None}).encode())
    fcntl.flock(budget, fcntl.LOCK_UN)
    child = subprocess.Popen(argv, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    poll = selectors.DefaultSelector()
    poll.register(child.stdout, selectors.EVENT_READ, ('stdout', 1))
    poll.register(child.stderr, selectors.EVENT_READ, ('stderr', 2))
    while poll.get_map():
        for key, _ in poll.select():
            data = os.read(key.fileobj.fileno(), 65536)
            if not data:
                poll.unregister(key.fileobj)
                key.fileobj.close()
                continue
            fcntl.flock(budget, fcntl.LOCK_EX)
            os.lseek(budget, 0, 0)
            used = int(os.read(budget, 32) or b'0') + len(data)
            if used > LIMIT:
                write_new(fd, 'capture.error', b'RTK output exceeded the 64 MiB job budget; logs are incomplete')
                os.killpg(os.getpgrp(), signal.SIGTERM)
                raise RuntimeError('output budget exceeded')
            os.lseek(budget, 0, 0)
            os.write(budget, str(used).encode())
            fcntl.flock(budget, fcntl.LOCK_UN)
            suffix, destination = key.data
            # Save before forwarding, including bytes before timeout/SIGINT.
            with os.fdopen(os.dup(outputs[suffix]), 'ab', buffering=0) as stream:
                stream.write(data)
            while data:
                data = data[os.write(destination, data):]
    status = child.wait()
    for output in outputs.values():
        os.fsync(output)
        os.close(output)
    os.close(budget)
    os.lseek(metadata, 0, 0)
    os.ftruncate(metadata, 0)
    with os.fdopen(metadata, 'w') as stream:
        json.dump({'argv': argv, 'cwd': os.getcwd(), 'exit': status}, stream)
        stream.flush()
        os.fsync(stream.fileno())
    return 128 - status if status < 0 else status


def resolve_tool(name, path):
    for directory_name in path.split(os.pathsep):
        candidate = Path(directory_name or '.') / name
        if not candidate.is_file() or not os.access(candidate, os.X_OK):
            continue
        resolved = str(candidate.resolve())
        if any(part in resolved for part in ['/.rch/shims/', '/remote-test/shims/', '/remote_test/shims/', '/remote-test-tool-bin/', '/remote-test-go-bin/']):
            continue
        with candidate.open('rb') as stream:
            prefix = stream.read(1024)
        if b'rch generic ' in prefix or b'rch cargo shim' in prefix or b'remote-test-tool-shim' in prefix:
            continue
        return str(candidate.absolute())
    return None


def instrument_filter(fd, root, identity, argv):
    """Capture an exact executable passed to RTK's generic filters."""
    index = 1
    while index < len(argv) and argv[index] in ['-v', '-vv', '-vvv', '--verbose', '--ultra-compact', '--skip-env']:
        index += 1
    if Path(argv[0]).name == 'rtk' and argv[index:index + 1] in [['test'], ['err'], ['summary']]:
        target = index + 1
        while target < len(argv) and argv[target] in ['--', '--ultra-compact', '--skip-env']:
            target += 1
        if target < len(argv) and '/' in argv[target]:
            name = 'bin/absolute-' + str(uuid.uuid4())
            tool = str(Path(argv[target]).absolute())
            script = '#!' + sys.executable + ' -I\nimport os,sys\nos.execv(' + repr(sys.executable) + ',[' + repr(sys.executable) + ',"-I",' + repr(root + '/.capture.py') + ',"direct",' + repr(root) + ',' + repr(identity) + ',' + repr(tool) + ',*sys.argv[1:]])\n'
            write_new(fd, name, script.encode(), 0o700)
            argv[target] = root + '/' + name
    return argv


def main():
    mode, root, identity = sys.argv[1:4]
    if mode == 'prepare':
        # The private, unpredictable job directory must be fresh. Parent
        # directories may already exist, but no component may be a symlink.
        parent = directory(str(Path(root).parent), create=True)
        os.mkdir(Path(root).name, 0o700, dir_fd=parent)
        fd = directory(root)
        write_new(fd, '.owner', identity.encode())
        write_new(fd, '.budget', b'0')
        write_new(fd, '.capture.py', sys.argv[4].encode())
        info = os.fstat(fd)
        print(json.dumps([info.st_dev, info.st_ino]))
        return
    fd = checked_root(root, identity)
    if mode == 'manifest':
        info = os.fstat(fd)
        assert json.loads(sys.argv[4]) == [info.st_dev, info.st_ino], 'capture directory replaced'
        names = [name for name in os.listdir(fd) if NAME.fullmatch(name)]
        assert len(names) <= FILES
        if 'capture.error' in os.listdir(fd):
            raise RuntimeError(read_file(fd, 'capture.error', 1024).decode())
        files, total = [], 0
        for name in sorted(names):
            data = read_file(fd, name, LIMIT)
            total += len(data)
            assert total <= LIMIT + 1024 * 1024
            files.append({'name': name, 'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
        assert {'outer.stdout', 'outer.stderr'} <= set(names), 'capture did not start'
        print(json.dumps(files))
    elif mode == 'read':
        name, offset, expected_directory = sys.argv[4:7]
        info = os.fstat(fd)
        assert json.loads(expected_directory) == [info.st_dev, info.st_ino], 'capture directory replaced'
        assert NAME.fullmatch(name)
        data = read_file(fd, name, LIMIT)
        offset = int(offset)
        assert 0 <= offset <= len(data)
        print(data[offset:offset + 1024 * 1024].hex())
    elif mode == 'tool':
        mapping = json.loads(read_file(fd, '.tools', 65536))
        name = sys.argv[4]
        assert name in mapping
        argv = [mapping[name], *sys.argv[5:]]
        if name == 'rtk':
            argv = instrument_filter(fd, root, identity, argv)
        sys.exit(capture(fd, str(uuid.uuid4()), argv, os.environ.copy()))
    elif mode == 'direct':
        sys.exit(capture(fd, str(uuid.uuid4()), sys.argv[4:], os.environ.copy()))
    elif mode == 'run':
        request = json.loads(sys.argv[4])
        env = os.environ.copy()
        # Keep RTK tracking and tee writes job-local too. Its own tee is not
        # the full-output contract (it has minimum size/rotation/truncation).
        env.update(RTK_TEE_DIR=root + '/tee', RTK_TEE='0', XDG_DATA_HOME=root + '/data')
        mapping = {name: path for name in ['rtk', 'go', 'cargo', 'cargo-clippy', 'bun', 'node', 'pytest', 'swift', 'npm', 'pnpm', 'npx']
                   if (path := resolve_tool(name, env.get('PATH', '/usr/bin:/bin')))}
        argv = request['value'] if request['mode'] == 'argv' else ['bash', '--noprofile', '--norc', '-c', request['value']]
        write_new(fd, '.tools', json.dumps(mapping).encode())
        os.mkdir('bin', 0o700, dir_fd=fd)
        argv = instrument_filter(fd, root, identity, argv)
        if argv[0] == 'rtk' and 'rtk' in mapping:
            argv[0] = mapping['rtk']
        for name in mapping:
            script = '#!' + sys.executable + ' -I\nimport os,sys\nos.execv(' + repr(sys.executable) + ',[' + repr(sys.executable) + ',"-I",' + repr(root + '/.capture.py') + ',"tool",' + repr(root) + ',' + repr(identity) + ',' + repr(name) + ',*sys.argv[1:]])\n'
            write_new(fd, 'bin/' + name, script.encode(), 0o700)
        env['PATH'] = root + '/bin:' + env.get('PATH', '/usr/bin:/bin')
        sys.exit(capture(fd, 'outer', argv, env))
    else:
        raise ValueError('unknown capture operation')


if __name__ == '__main__':
    main()

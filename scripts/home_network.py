#!/usr/bin/env python3
"""Connect to the three home-network computers. Run --help for commands."""
from __future__ import annotations

import argparse
import base64
import contextlib
import errno
import hashlib
import os
from pathlib import Path
import posixpath
import re
import shlex
import shutil
import socket
import stat
import sys
import tempfile
import time

HOSTS = (
    {"name": "FRANK", "host": "192.168.1.247", "user": "mitch", "os": "windows",
     "aliases": ("win",)},
    {"name": "MACINDOZE", "host": "192.168.1.243", "user": "mitch", "os": "windows",
     "aliases": ("imac",)},
    {"name": "Mitch's MacBook Pro", "host": "mitch-mac.local", "user": "mitchwarrenburg", "os": "macos",
     "aliases": ("macbook", "mitch-mac", "mitch-mac.local", "Mac.attlocal.net")},
)
CONNECT_TIMEOUT = 15
AUTH_TIMEOUT = 30
SSH_PORT = 22
KEEPALIVE_SECONDS = 30
IO_CHUNK_SIZE = 64 * 1024
CHANNEL_POLL_SECONDS = 0.01
INPUT_POLL_SECONDS = 0.02
DEFAULT_TERMINAL_SIZE = (100, 30)
WINDOWS_VT_INPUT = 0x0200
WINDOWS_VT_OUTPUT = 0x0004
WINDOWS_INPUT_LINE_ECHO_SIGNALS = 0x0007


def resolve_host(name):
    normalized = name.replace("\u2019", "'").casefold()
    for host in HOSTS:
        if normalized in (x.casefold() for x in (host["name"], *host["aliases"])):
            return host
    raise ValueError("Unknown machine: %s. Run 'hosts' for names and aliases." % name)


def parser():
    result = argparse.ArgumentParser(
        description="SSH/SFTP for the home network; password comes only from HOME_NETWORK_PW.",
        usage="%(prog)s hosts | MACHINE ACTION [arguments]",
        epilog="Run the platform wrapper with 'setup' once. Quote remote commands as one argument.")
    result.add_argument("machine", help="FRANK/win, MACINDOZE/imac, or macbook (exact names also accepted)")
    commands = result.add_subparsers(dest="action", required=True)
    for action, help_text in (
        ("info", "show account and address (offline)"),
        ("smb", "show FRANK's C/D/E SMB share paths (offline)"),
        ("check", "verify SSH command execution and SFTP access"),
        ("shell", "start a remote shell; requires an interactive terminal"),
        ("sftp", "start an SFTP prompt (pwd, cd, ls, get, put, exit)"),
    ):
        commands.add_parser(action, help=help_text)
    execute = commands.add_parser("exec", help="execute one quoted remote command and return its exit status")
    execute.add_argument("command", help="one command string for the remote default shell; stdin is closed")
    listing = commands.add_parser("ls", help="list a remote directory via SFTP")
    listing.add_argument("path", nargs="?", default=".")
    for action in ("get", "put"):
        transfer = commands.add_parser(action, help="copy to an exact destination; existing files are refused")
        transfer.add_argument("source")
        transfer.add_argument("destination")
        transfer.add_argument("--recursive", "-r", action="store_true", help="required for directories")
    return result


def dependency():
    try:
        import paramiko
    except ImportError as error:
        raise RuntimeError("SSH support is not installed. Run the platform wrapper with 'setup'.") from error
    return paramiko


def fingerprint(key):
    return "SHA256:" + base64.b64encode(hashlib.sha256(key.asbytes()).digest()).decode().rstrip("=")


def known_hosts_policy(paramiko, path):
    class AcceptNew(paramiko.MissingHostKeyPolicy):
        def missing_host_key(self, client, hostname, key):
            # Reload immediately before adding so independent repos share current trust.
            keys = paramiko.HostKeys()
            if path.exists():
                keys.load(str(path))
            previous = keys.lookup(hostname)
            if previous:
                if key.get_name() not in previous or previous[key.get_name()] != key:
                    raise paramiko.SSHException("Host key changed for %s; verify it before updating %s." % (hostname, path))
                return
            path.parent.mkdir(parents=True, exist_ok=True)
            entry = "%s %s %s\n" % (hostname, key.get_name(), key.get_base64())
            if path.exists() and path.stat().st_size:
                with path.open("rb") as existing:
                    existing.seek(-1, os.SEEK_END)
                    if existing.read(1) != b"\n":
                        entry = "\n" + entry
            # Append retains comments, hashed entries, permissions, and unrelated keys.
            fd = os.open(str(path), os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
            with os.fdopen(fd, "a", encoding="utf-8") as handle:
                handle.write(entry)
            client.get_host_keys().add(hostname, key.get_name(), key)
            print("Trusting new host %s: %s (%s)" % (hostname, fingerprint(key), path), file=sys.stderr)
    return AcceptNew()


def connection(host):
    password = os.environ.get("HOME_NETWORK_PW")
    if not password:
        raise ValueError("Set HOME_NETWORK_PW in this process before connecting.")
    paramiko = dependency()
    client = paramiko.SSHClient()
    path = Path.home() / ".ssh" / "known_hosts"
    if path.exists():
        client.load_host_keys(str(path))
    client.set_missing_host_key_policy(known_hosts_policy(paramiko, path))
    sock = None
    try:
        try:
            sock = socket.create_connection((host["host"], SSH_PORT), timeout=CONNECT_TIMEOUT)
        except socket.gaierror:
            # Some Windows mDNS providers resolve .local only through this IPv4 API.
            address = socket.gethostbyname(host["host"])
            sock = socket.create_connection((address, SSH_PORT), timeout=CONNECT_TIMEOUT)
        client.connect(hostname=host["host"], username=host["user"], password=password,
                       sock=sock, timeout=CONNECT_TIMEOUT, auth_timeout=AUTH_TIMEOUT,
                       banner_timeout=AUTH_TIMEOUT, allow_agent=False, look_for_keys=False)
        client.get_transport().set_keepalive(KEEPALIVE_SECONDS)
        return client
    except BaseException:
        client.close()
        if sock is not None:
            sock.close()
        raise


def output(data, stream):
    if hasattr(stream, "buffer"):
        stream.buffer.write(data)
        stream.buffer.flush()
    else:
        stream.write(data.decode("utf-8", errors="replace"))
        stream.flush()


def drain(channel):
    """Drain both windows before reading status, even for output above SSH's window size."""
    while True:
        if channel.recv_ready():
            output(channel.recv(IO_CHUNK_SIZE), sys.stdout)
        if channel.recv_stderr_ready():
            output(channel.recv_stderr(IO_CHUNK_SIZE), sys.stderr)
        if channel.exit_status_ready() and not channel.recv_ready() and not channel.recv_stderr_ready():
            status = channel.recv_exit_status()
            return status if status >= 0 else 255
        time.sleep(CHANNEL_POLL_SECONDS)


def execute(client, command):
    with client.get_transport().open_session(timeout=CONNECT_TIMEOUT) as channel:
        channel.exec_command(command)
        channel.shutdown_write()
        return drain(channel)


@contextlib.contextmanager
def raw_terminal():
    if os.name == "nt":
        import ctypes
        kernel = ctypes.windll.kernel32
        kernel.GetStdHandle.restype = ctypes.c_void_p
        kernel.GetConsoleMode.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong)]
        kernel.SetConsoleMode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        saved = []
        try:
            for handle_id, mask, clear in ((-10, WINDOWS_VT_INPUT, WINDOWS_INPUT_LINE_ECHO_SIGNALS), (-11, WINDOWS_VT_OUTPUT, 0)):
                handle = kernel.GetStdHandle(handle_id)
                mode = ctypes.c_ulong()
                if kernel.GetConsoleMode(handle, ctypes.byref(mode)):
                    saved.append((handle, mode.value))
                    kernel.SetConsoleMode(handle, (mode.value & ~clear) | mask)
            yield
        finally:
            for handle, mode in saved:
                kernel.SetConsoleMode(handle, mode)
    else:
        import termios
        import tty
        descriptor = sys.stdin.fileno()
        original = termios.tcgetattr(descriptor)
        try:
            tty.setraw(descriptor)
            yield
        finally:
            termios.tcsetattr(descriptor, termios.TCSADRAIN, original)


def shell(client):
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise ValueError("shell requires an interactive terminal. Agents should use exec with one quoted command.")
    size = shutil.get_terminal_size(DEFAULT_TERMINAL_SIZE)
    with client.invoke_shell(term="xterm-256color", width=size.columns, height=size.lines) as channel:
        with raw_terminal():
            while True:
                if channel.recv_ready():
                    output(channel.recv(IO_CHUNK_SIZE), sys.stdout)
                if channel.exit_status_ready() and not channel.recv_ready():
                    status = channel.recv_exit_status()
                    return status if status >= 0 else 255
                resized = shutil.get_terminal_size(DEFAULT_TERMINAL_SIZE)
                if resized != size:
                    channel.resize_pty(width=resized.columns, height=resized.lines)
                    size = resized
                if os.name == "nt":
                    import msvcrt
                    if msvcrt.kbhit():
                        char = msvcrt.getwch()
                        if char in ("\x00", "\xe0"):
                            char = {"H": "\x1b[A", "P": "\x1b[B", "M": "\x1b[C", "K": "\x1b[D",
                                    "G": "\x1b[H", "O": "\x1b[F", "S": "\x1b[3~"}.get(msvcrt.getwch(), "")
                        if char:
                            if len(char) == 1 and 0xD800 <= ord(char) <= 0xDBFF:
                                char = (char + msvcrt.getwch()).encode("utf-16-le", "surrogatepass").decode("utf-16-le")
                            channel.sendall(char.encode("utf-8"))
                    else:
                        time.sleep(CHANNEL_POLL_SECONDS)
                else:
                    import select
                    if select.select([sys.stdin], [], [], INPUT_POLL_SECONDS)[0]:
                        data = os.read(sys.stdin.fileno(), IO_CHUNK_SIZE)
                        if not data:
                            channel.shutdown_write()
                        else:
                            channel.sendall(data)


def safe_name(name):
    if (not name or name in (".", "..") or any(char in name for char in "/\\:\0")
            or any(ord(char) < 32 for char in name)):
        raise ValueError("Unsafe transfer entry: %r" % name)
    if os.name == "nt" and (name[-1:] in (".", " ") or any(char in name for char in '<>"|?*')
                            or name.split(".")[0].upper() in {"CON", "PRN", "AUX", "NUL", *["COM%d" % x for x in range(1, 10)], *["LPT%d" % x for x in range(1, 10)]}):
        raise ValueError("Unsupported Windows filename: %r" % name)
    return name


def local_path(value):
    path = Path(value).expanduser()
    if ".." in path.parts:
        raise ValueError("Use a local path without '..': %s" % value)
    path = Path(os.path.abspath(path))
    for part in reversed((path, *path.parents)):
        try:
            info = part.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400):
            raise ValueError("Local symlink or reparse point refused: %s" % part)
    return path


def remote_path(sftp, value, login_home=None):
    value = value.replace("\\", "/")
    if ".." in value.split("/") or "\0" in value:
        raise ValueError("Use a remote path without '..' or NUL.")
    if re.match(r"^[A-Za-z]:/", value):
        value = "/" + value
    if value == "~" or value.startswith("~/"):
        home = login_home if login_home is not None else sftp.normalize(".")
        value = posixpath.join(home, value[2:] if value != "~" else "")
    if not value.startswith("/"):
        value = posixpath.join(sftp.normalize("."), value)
    if re.match(r"^[A-Za-z]:/", value):
        value = "/" + value
    return posixpath.normpath(value)


def remote_stat(sftp, path):
    """Use lstat on every existing ancestor so traversal never follows a link."""
    current = "/" if path.startswith("/") else ""
    result = None
    for part in path.split("/"):
        if not part:
            continue
        current = posixpath.join(current, part)
        try:
            result = sftp.lstat(current)
        except OSError as error:
            if error.errno == errno.ENOENT:
                return None
            raise
        if stat.S_ISLNK(result.st_mode):
            raise ValueError("Remote symlink refused: %s" % current)
    return result if result is not None else sftp.lstat(path)


def require_type(info, path):
    if info is None:
        raise FileNotFoundError("Source does not exist: %s" % path)
    if not stat.S_ISREG(info.st_mode) and not stat.S_ISDIR(info.st_mode):
        raise ValueError("Only ordinary files and directories are supported: %s" % path)


def remote_tree(sftp, path, relative=""):
    info = remote_stat(sftp, path)
    require_type(info, path)
    yield relative, stat.S_ISDIR(info.st_mode)
    if stat.S_ISDIR(info.st_mode):
        for entry in sftp.listdir_attr(path):
            name = safe_name(entry.filename)
            yield from remote_tree(sftp, posixpath.join(path, name), posixpath.join(relative, name))


def local_tree(path, relative=""):
    path = local_path(path)
    info = path.lstat()
    require_type(info, path)
    yield relative, stat.S_ISDIR(info.st_mode)
    if stat.S_ISDIR(info.st_mode):
        for child in sorted(path.iterdir()):
            safe_name(child.name)
            yield from local_tree(child, posixpath.join(relative, child.name))


def check_destination(info, directory, path):
    if info is not None and not (directory and stat.S_ISDIR(info.st_mode)):
        raise FileExistsError("Destination already exists; choose a new path: %s" % path)


def local_stat(path):
    path = local_path(path)
    try:
        return path.lstat()
    except FileNotFoundError:
        return None


def publish_local(staged, target):
    if os.name == "nt":
        # Windows rename is exclusive and also works on exFAT (which has no hard links).
        os.rename(staged, target)
        return
    try:
        os.link(staged, target)
    except OSError as error:
        if error.errno not in (errno.EXDEV, errno.EPERM, errno.EOPNOTSUPP):
            raise
        # Filesystems without hard links retain no-overwrite safety via exclusive creation.
        handle = target.open("xb")
        try:
            with handle, open(staged, "rb") as source:
                shutil.copyfileobj(source, handle, IO_CHUNK_SIZE)
        except BaseException:
            target.unlink()
            raise


def get_files(sftp, source, destination, recursive=False):
    source = remote_path(sftp, source)
    destination = local_path(destination)
    source_info = remote_stat(sftp, source)
    require_type(source_info, source)
    if stat.S_ISDIR(source_info.st_mode) and not recursive:
        raise ValueError("Directory transfer requires --recursive.")
    plan = list(remote_tree(sftp, source))
    for relative, directory in plan:
        target = local_path(destination / relative)
        check_destination(local_stat(target), directory, target)
    for relative, directory in plan:
        target = local_path(destination / relative)
        remote = posixpath.join(source, relative) if relative else source
        if directory:
            target.mkdir(parents=True, exist_ok=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            # Stage a complete download before publishing it without overwriting.
            fd, staged = tempfile.mkstemp(prefix=".home-network-", dir=str(target.parent))
            try:
                with os.fdopen(fd, "wb") as handle:
                    require_type(remote_stat(sftp, remote), remote)
                    sftp.getfo(remote, handle)
                local_path(target)
                publish_local(staged, target)
            finally:
                with contextlib.suppress(FileNotFoundError):
                    os.unlink(staged)
            print("Downloaded %s -> %s" % (remote, target))


def remote_mkdirs(sftp, path):
    info = remote_stat(sftp, path)
    if info is not None:
        if not stat.S_ISDIR(info.st_mode):
            raise ValueError("Remote parent is not a directory: %s" % path)
        return
    parent = posixpath.dirname(path)
    if parent and parent != path:
        remote_mkdirs(sftp, parent)
    sftp.mkdir(path)


def put_files(sftp, source, destination, recursive=False):
    source = local_path(source)
    destination = remote_path(sftp, destination)
    source_info = source.lstat()
    require_type(source_info, source)
    if stat.S_ISDIR(source_info.st_mode) and not recursive:
        raise ValueError("Directory transfer requires --recursive.")
    plan = list(local_tree(source))
    for relative, directory in plan:
        target = posixpath.join(destination, relative) if relative else destination
        check_destination(remote_stat(sftp, target), directory, target)
    for relative, directory in plan:
        target = posixpath.join(destination, relative) if relative else destination
        local = local_path(source / relative)
        if directory:
            remote_mkdirs(sftp, target)
        else:
            remote_mkdirs(sftp, posixpath.dirname(target))
            # Exclusive creation prevents replacing a pre-existing remote file.
            handle = sftp.open(target, "wx")
            try:
                with handle, local.open("rb") as input_file:
                    shutil.copyfileobj(input_file, handle, IO_CHUNK_SIZE)
            except BaseException:
                with contextlib.suppress(OSError):
                    sftp.remove(target)
                raise
            print("Uploaded %s -> %s" % (local, target))


def list_files(sftp, path):
    path = remote_path(sftp, path)
    info = remote_stat(sftp, path)
    require_type(info, path)
    if not stat.S_ISDIR(info.st_mode):
        print("%12d  %s" % (info.st_size, path))
        return
    for item in sorted(sftp.listdir_attr(path), key=lambda value: value.filename.casefold()):
        suffix = "/" if stat.S_ISDIR(item.st_mode) else "@" if stat.S_ISLNK(item.st_mode) else ""
        print("%12d  %s%s" % (item.st_size, item.filename, suffix))


def sftp_prompt(sftp):
    login_home = sftp.normalize(".")
    print("SFTP: pwd, cd PATH, ls [PATH], get SOURCE DEST [-r], put SOURCE DEST [-r], exit")
    print("Quote paths with spaces; use forward slashes. Existing files and symlinks are refused.")
    while True:
        try:
            line = input("sftp> ")
        except EOFError:
            return 0
        try:
            args = shlex.split(line)
            if not args:
                continue
            action, *values = args
            if action in ("exit", "quit", "bye"):
                return 0
            if action == "pwd" and not values:
                print(sftp.normalize("."))
            elif action == "cd" and len(values) == 1:
                target = remote_path(sftp, values[0], login_home)
                info = remote_stat(sftp, target)
                if info is None or not stat.S_ISDIR(info.st_mode):
                    raise ValueError("Remote path is not a directory.")
                sftp.chdir(target)
            elif action == "ls" and len(values) <= 1:
                target = remote_path(sftp, values[0] if values else ".", login_home)
                list_files(sftp, target)
            elif action in ("get", "put"):
                recursive = "-r" in values or "--recursive" in values
                paths = [arg for arg in values if arg not in ("-r", "--recursive")]
                if len(paths) != 2:
                    raise ValueError("Use %s SOURCE DESTINATION [-r]." % action)
                remote_index = 0 if action == "get" else 1
                paths[remote_index] = remote_path(sftp, paths[remote_index], login_home)
                (get_files if action == "get" else put_files)(sftp, *paths, recursive=recursive)
            elif action == "help":
                print("pwd | cd PATH | ls [PATH] | get REMOTE LOCAL [-r] | put LOCAL REMOTE [-r] | exit")
            else:
                raise ValueError("Unknown command or wrong arguments. Type help.")
        except (OSError, ValueError) as error:
            print("Error: %s" % error, file=sys.stderr)


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv == ["hosts"]:
        for host in HOSTS:
            print("%s: %s@%s (aliases: %s)" % (host["name"], host["user"], host["host"], ", ".join(host["aliases"])))
        return 0
    args = parser().parse_args(argv)
    try:
        host = resolve_host(args.machine)
        if args.action == "info":
            print("%s\nSSH/SFTP: %s@%s:22\nRemote shell: %s" %
                  (host["name"], host["user"], host["host"], "PowerShell" if host["os"] == "windows" else "macOS login shell"))
            return 0
        if args.action == "smb":
            if host["name"] != "FRANK":
                raise ValueError("Shared drives are on FRANK. Run 'FRANK smb'.")
            for drive in ("C", "D", "E"):
                print("%s (%s): \\\\FRANK\\%s | \\\\192.168.1.247\\%s | smb://192.168.1.247/%s" %
                      (drive, "read-only" if drive == "C" else "read/write", drive, drive, drive))
            return 0
        if args.action == "shell" and (not sys.stdin.isatty() or not sys.stdout.isatty()):
            raise ValueError("shell requires an interactive terminal. Agents should use exec with one quoted command.")
        with connection(host) as client:
            if args.action == "exec":
                return execute(client, args.command)
            if args.action == "shell":
                return shell(client)
            if args.action == "check":
                result = execute(client, "$env:COMPUTERNAME; whoami" if host["os"] == "windows" else "hostname; whoami")
                if result:
                    return result
            with client.open_sftp() as sftp:
                if args.action == "check":
                    print("SSH and SFTP OK; remote home: %s" % sftp.normalize("."))
                elif args.action == "sftp":
                    return sftp_prompt(sftp)
                elif args.action == "ls":
                    list_files(sftp, args.path)
                elif args.action == "get":
                    get_files(sftp, args.source, args.destination, args.recursive)
                elif args.action == "put":
                    put_files(sftp, args.source, args.destination, args.recursive)
        return 0
    except (OSError, ValueError, RuntimeError) as error:
        print("Error: %s" % error, file=sys.stderr)
        return 1
    except Exception as error:
        # Paramiko's exception classes are imported only for online actions.
        print("Connection failed (%s): %s" % (type(error).__name__, error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)

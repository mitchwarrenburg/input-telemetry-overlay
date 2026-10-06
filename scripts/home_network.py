#!/usr/bin/env python3
"""Connect to the three home-network computers. Run --help for commands."""
from __future__ import annotations

import argparse
import base64
import codecs
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
# LAN waits and polling intervals, in seconds: chosen for a home network, not measured.
CONNECT_TIMEOUT = 15
AUTH_TIMEOUT = 30
KEEPALIVE_SECONDS = 30
CHANNEL_POLL_SECONDS = 0.01
INPUT_POLL_SECONDS = 0.02
# The registered SSH port, a chosen transfer buffer and the size assumed when no terminal reports one.
SSH_PORT = 22
IO_CHUNK_SIZE = 64 * 1024
DEFAULT_TERMINAL_SIZE = (100, 30)
# OpenSSH exits 255 for its own failures; shells report an interrupt as 128 + SIGINT (2) = 130.
MISSING_EXIT_STATUS_CODE = 255
INTERRUPTED_EXIT_CODE = 130
# Filenames are refused below U+0020, the end of the C0 control characters.
C0_CONTROL_LIMIT = 32
# Win32 file-system values: the reparse-point attribute and the reserved COM1-9/LPT1-9 devices.
WINDOWS_REPARSE_POINT_ATTRIBUTE = getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400)
WINDOWS_DOS_PORT_NUMBERS = range(1, 10)
# Win32 console API values: standard handles, mode flags, the key record type and its union size.
WINDOWS_STDIN_HANDLE = -10
WINDOWS_STDOUT_HANDLE = -11
WINDOWS_VT_INPUT = 0x0200
WINDOWS_VT_OUTPUT = 0x0004
WINDOWS_INPUT_LINE_ECHO_SIGNALS = 0x0007
WINDOWS_KEY_EVENT = 0x0001
WINDOWS_EVENT_UNION_BYTES = 16
# Chosen bound on console records read per poll, not measured.
WINDOWS_INPUT_BATCH_SIZE = 64
# Win32 virtual-key codes for keys which carry no character, and the sequences xterm sends for them.
WINDOWS_VK_MENU = 0x12
WINDOWS_VK_PRIOR = 0x21
WINDOWS_VK_NEXT = 0x22
WINDOWS_VK_END = 0x23
WINDOWS_VK_HOME = 0x24
WINDOWS_VK_LEFT = 0x25
WINDOWS_VK_UP = 0x26
WINDOWS_VK_RIGHT = 0x27
WINDOWS_VK_DOWN = 0x28
WINDOWS_VK_INSERT = 0x2D
WINDOWS_VK_DELETE = 0x2E
WINDOWS_KEY_SEQUENCES = {
    WINDOWS_VK_PRIOR: "\x1b[5~", WINDOWS_VK_NEXT: "\x1b[6~", WINDOWS_VK_END: "\x1b[F", WINDOWS_VK_HOME: "\x1b[H",
    WINDOWS_VK_LEFT: "\x1b[D", WINDOWS_VK_UP: "\x1b[A", WINDOWS_VK_RIGHT: "\x1b[C", WINDOWS_VK_DOWN: "\x1b[B",
    WINDOWS_VK_INSERT: "\x1b[2~", WINDOWS_VK_DELETE: "\x1b[3~",
}


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
        if channel.closed and not channel.recv_ready() and not channel.recv_stderr_ready():
            status = channel.recv_exit_status()
            return status if status >= 0 else MISSING_EXIT_STATUS_CODE
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
            for handle_id, mask, clear in ((WINDOWS_STDIN_HANDLE, WINDOWS_VT_INPUT, WINDOWS_INPUT_LINE_ECHO_SIGNALS),
                                           (WINDOWS_STDOUT_HANDLE, WINDOWS_VT_OUTPUT, 0)):
                handle = kernel.GetStdHandle(handle_id)
                mode = ctypes.c_ulong()
                if not kernel.GetConsoleMode(handle, ctypes.byref(mode)):
                    raise ctypes.WinError()
                saved.append((handle, mode.value))
                if not kernel.SetConsoleMode(handle, (mode.value & ~clear) | mask):
                    raise ctypes.WinError()
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


def windows_key_text(char, virtual_key, repeat, down=True):
    # UnicodeChar distinguishes real U+00E0 from keys which CRT encodes with an 0xE0 prefix.
    # Alt composition (including ConPTY surrogate pairs) delivers text on Alt key release.
    if not down and not (virtual_key == WINDOWS_VK_MENU and char != "\0"):
        return ""
    return (char if char != "\0" else WINDOWS_KEY_SEQUENCES.get(virtual_key, "")) * repeat


def windows_input_reader():
    """Read Unicode console events without blocking on mouse/resize/key-release events."""
    import ctypes
    from ctypes import wintypes

    class KeyEvent(ctypes.Structure):
        _fields_ = [("down", wintypes.BOOL), ("repeat", wintypes.WORD),
                    ("virtual_key", wintypes.WORD), ("scan", wintypes.WORD),
                    ("char", wintypes.WCHAR), ("control", wintypes.DWORD)]

    class EventData(ctypes.Union):
        _fields_ = [("key", KeyEvent), ("padding", ctypes.c_byte * WINDOWS_EVENT_UNION_BYTES)]

    class InputRecord(ctypes.Structure):
        _fields_ = [("kind", wintypes.WORD), ("event", EventData)]

    kernel = ctypes.windll.kernel32
    kernel.GetStdHandle.restype = wintypes.HANDLE
    kernel.GetNumberOfConsoleInputEvents.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    kernel.ReadConsoleInputW.argtypes = [wintypes.HANDLE, ctypes.POINTER(InputRecord), wintypes.DWORD,
                                      ctypes.POINTER(wintypes.DWORD)]
    handle = kernel.GetStdHandle(WINDOWS_STDIN_HANDLE)
    decoder = codecs.getincrementaldecoder("utf-16-le")("replace")

    def read():
        count = wintypes.DWORD()
        if not kernel.GetNumberOfConsoleInputEvents(handle, ctypes.byref(count)):
            raise ctypes.WinError()
        if not count.value:
            return b""
        records = (InputRecord * min(count.value, WINDOWS_INPUT_BATCH_SIZE))()
        if not kernel.ReadConsoleInputW(handle, records, len(records), ctypes.byref(count)):
            raise ctypes.WinError()
        text = "".join(windows_key_text(record.event.key.char, record.event.key.virtual_key,
                                       record.event.key.repeat, record.event.key.down)
                       for record in records[:count.value]
                       if record.kind == WINDOWS_KEY_EVENT)
        return decoder.decode(text.encode("utf-16-le", "surrogatepass")).encode("utf-8")
    return read


def shell(client):
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise ValueError("shell requires an interactive terminal. Agents should use exec with one quoted command.")
    size = shutil.get_terminal_size(DEFAULT_TERMINAL_SIZE)
    input_open = True
    with client.invoke_shell(term="xterm-256color", width=size.columns, height=size.lines) as channel:
        with raw_terminal():
            read_windows = windows_input_reader() if os.name == "nt" else None
            while True:
                if channel.recv_ready():
                    output(channel.recv(IO_CHUNK_SIZE), sys.stdout)
                if channel.closed and not channel.recv_ready():
                    status = channel.recv_exit_status()
                    return status if status >= 0 else MISSING_EXIT_STATUS_CODE
                resized = shutil.get_terminal_size(DEFAULT_TERMINAL_SIZE)
                if resized != size:
                    channel.resize_pty(width=resized.columns, height=resized.lines)
                    size = resized
                if os.name == "nt":
                    data = read_windows()
                    if data:
                        channel.sendall(data)
                    else:
                        time.sleep(CHANNEL_POLL_SECONDS)
                else:
                    import select
                    if not input_open:
                        time.sleep(CHANNEL_POLL_SECONDS)
                        continue
                    if select.select([sys.stdin], [], [], INPUT_POLL_SECONDS)[0]:
                        data = os.read(sys.stdin.fileno(), IO_CHUNK_SIZE)
                        if not data:
                            channel.shutdown_write()
                            input_open = False
                        else:
                            channel.sendall(data)


def safe_name(name, windows=None):
    if (not name or name in (".", "..") or any(char in name for char in "/\\:\0")
            or any(ord(char) < C0_CONTROL_LIMIT for char in name)):
        raise ValueError("Unsafe transfer entry: %r" % name)
    windows = os.name == "nt" if windows is None else windows
    if windows and (name[-1:] in (".", " ") or any(char in name for char in '<>"|?*')
                            or name.split(".")[0].upper() in {"CON", "PRN", "AUX", "NUL", *["COM%d" % x for x in WINDOWS_DOS_PORT_NUMBERS], *["LPT%d" % x for x in WINDOWS_DOS_PORT_NUMBERS]}):
        raise ValueError("Unsupported Windows filename: %r" % name)
    return name


def local_path(value, resolve_root=False):
    path = Path(value).expanduser()
    if ".." in path.parts:
        raise ValueError("Use a local path without '..': %s" % value)
    path = Path(os.path.abspath(path))
    if resolve_root:
        path = Path(os.path.realpath(path))
    for part in reversed((path, *path.parents)):
        try:
            info = part.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & WINDOWS_REPARSE_POINT_ATTRIBUTE:
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


def remote_root(sftp, value):
    """Resolve an explicit user root once, including existing ancestors of new destinations."""
    path = remote_path(sftp, value)
    missing = []
    while True:
        try:
            canonical = remote_path(sftp, sftp.normalize(path))
            return posixpath.join(canonical, *reversed(missing)) if missing else canonical
        except OSError as error:
            if error.errno != errno.ENOENT or posixpath.dirname(path) == path:
                raise
            missing.append(posixpath.basename(path))
            path = posixpath.dirname(path)


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
        if error.errno not in (errno.EXDEV, errno.EPERM, errno.EOPNOTSUPP, errno.ENOTSUP):
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
    source = remote_root(sftp, source)
    destination = local_path(destination, resolve_root=True)
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


def validate_remote_destination(path, windows):
    for index, component in enumerate(path.strip("/").split("/")):
        if component and not (index == 0 and re.fullmatch(r"[A-Za-z]:", component)):
            safe_name(component, windows=windows)


def put_files(sftp, source, destination, recursive=False, windows_destination=False):
    source = local_path(source, resolve_root=True)
    validate_remote_destination(remote_path(sftp, destination), windows_destination)
    destination = remote_root(sftp, destination)
    source_info = source.lstat()
    require_type(source_info, source)
    if stat.S_ISDIR(source_info.st_mode) and not recursive:
        raise ValueError("Directory transfer requires --recursive.")
    plan = list(local_tree(source))
    for relative, directory in plan:
        target = posixpath.join(destination, relative) if relative else destination
        # Validate the full target for its destination platform, including the explicit root.
        validate_remote_destination(target, windows_destination)
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
    path = remote_root(sftp, path)
    info = remote_stat(sftp, path)
    require_type(info, path)
    if not stat.S_ISDIR(info.st_mode):
        print("%12d  %s" % (info.st_size, path))
        return
    for item in sorted(sftp.listdir_attr(path), key=lambda value: value.filename.casefold()):
        suffix = "/" if stat.S_ISDIR(item.st_mode) else "@" if stat.S_ISLNK(item.st_mode) else ""
        print("%12d  %s%s" % (item.st_size, item.filename, suffix))


def sftp_prompt(sftp, windows_destination=False):
    login_home = sftp.normalize(".")
    print("SFTP: pwd, cd PATH, ls [PATH], get SOURCE DEST [-r], put SOURCE DEST [-r], exit")
    print("Quote paths with spaces; use forward slashes. Existing files are refused.")
    print("Below transfer roots, local links and server-reported symlinks are refused.")
    while True:
        try:
            line = input("sftp> ")
        except EOFError:
            return 0
        try:
            if "\\" in line:
                raise ValueError("Use forward slashes in SFTP paths; backslashes are not accepted.")
            args = shlex.split(line)
            if not args:
                continue
            action, *values = args
            if action in ("exit", "quit", "bye"):
                return 0
            if action == "pwd" and not values:
                print(sftp.normalize("."))
            elif action == "cd" and len(values) == 1:
                target = remote_root(sftp, remote_path(sftp, values[0], login_home))
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
                if action == "get":
                    get_files(sftp, *paths, recursive=recursive)
                else:
                    put_files(sftp, *paths, recursive=recursive, windows_destination=windows_destination)
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
            print("%s\nSSH/SFTP: %s@%s:%d\nRemote shell: %s" %
                  (host["name"], host["user"], host["host"], SSH_PORT, "PowerShell" if host["os"] == "windows" else "macOS login shell"))
            return 0
        if args.action == "smb":
            if host["name"] != "FRANK":
                raise ValueError("Shared drives are on FRANK. Run 'FRANK smb'.")
            for drive in ("C", "D", "E"):
                print("%s (%s): \\\\FRANK\\%s | \\\\%s\\%s | smb://%s/%s" %
                      (drive, "read-only" if drive == "C" else "read/write", drive, host["host"], drive, host["host"], drive))
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
                    return sftp_prompt(sftp, windows_destination=host["os"] == "windows")
                elif args.action == "ls":
                    list_files(sftp, args.path)
                elif args.action == "get":
                    get_files(sftp, args.source, args.destination, args.recursive)
                elif args.action == "put":
                    put_files(sftp, args.source, args.destination, args.recursive, windows_destination=host["os"] == "windows")
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
        sys.exit(INTERRUPTED_EXIT_CODE)

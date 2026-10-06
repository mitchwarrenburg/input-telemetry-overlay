"""Offline behavioral checks: python -m unittest discover -s scripts -p test_home_network.py."""
import contextlib
import errno
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("home_network", Path(__file__).with_name("home_network.py"))
network = importlib.util.module_from_spec(spec)
spec.loader.exec_module(network)
TEST_HOST_KEY_BITS = 1024
TEST_SETUP_FAILURE_CODE = 73
TEST_DARWIN_ENOTSUP = 45


class FakeSFTP:
    def __init__(self):
        self.files = {"/": None, "/home": None, "/home/user": None}
        self.extra_entries = []
        self.fail_download = False
        self.cwd = "/home/user"

    def normalize(self, path):
        return self.cwd if path == "." else path

    def chdir(self, path):
        self.cwd = path

    def lstat(self, path):
        if path not in self.files:
            raise FileNotFoundError(errno.ENOENT, "missing", path)
        value = self.files[path]
        mode = stat.S_IFDIR if value is None else stat.S_IFLNK if value == "LINK" else stat.S_IFREG
        return SimpleNamespace(st_mode=mode, st_size=len(value) if value else 0)

    def listdir_attr(self, path):
        result = []
        for name in self.files:
            if name.rpartition("/")[0] == path:
                item = self.lstat(name)
                item.filename = name.rpartition("/")[2]
                result.append(item)
        return result + self.extra_entries

    def getfo(self, path, target):
        target.write(self.files[path])
        if self.fail_download:
            raise OSError("simulated interrupted download")

    def mkdir(self, path):
        if path in self.files:
            raise FileExistsError(path)
        self.files[path] = None

    def open(self, path, mode):
        if mode != "wx":
            raise AssertionError("Uploads must use exclusive creation")
        if path in self.files:
            raise FileExistsError(path)
        self.files[path] = b""
        owner = self

        class Writer(io.BytesIO):
            def close(self):
                if not self.closed:
                    owner.files[path] = self.getvalue()
                super().close()
        return Writer()

    def remove(self, path):
        del self.files[path]


class NetworkTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name).resolve()
        self.sftp = FakeSFTP()
        self.stdout = patch("sys.stdout", new_callable=io.StringIO)
        self.stdout.start()
        self.addCleanup(self.stdout.stop)

    def test_aliases_and_exact_curly_name(self):
        for name, expected in (("WIN", "FRANK"), ("iMac", "MACINDOZE"), ("macbook", "Mitch's MacBook Pro"),
                               ("Mitch’s MacBook Pro", "Mitch's MacBook Pro"), ("mac.attlocal.net", "Mitch's MacBook Pro")):
            self.assertEqual(network.resolve_host(name)["name"], expected)
        with self.assertRaises(ValueError):
            network.resolve_host("another-machine")

    def test_offline_commands_need_no_password_or_dependency(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(network, "dependency", side_effect=AssertionError):
            for args in (["hosts"], ["win", "info"], ["FRANK", "smb"]):
                self.assertEqual(network.main(args), 0)
            with self.assertRaisesRegex(ValueError, "HOME_NETWORK_PW"):
                network.connection(network.HOSTS[0])

    def test_remote_paths_preserve_spaces_unicode_and_drive_root(self):
        self.assertEqual(network.remote_path(self.sftp, "D:\\data files\\café.txt"), "/D:/data files/café.txt")
        self.assertEqual(network.remote_path(self.sftp, "~/data file"), "/home/user/data file")
        with self.assertRaises(ValueError):
            network.remote_path(self.sftp, "../outside")

    def test_sftp_prompt_tilde_uses_login_home_after_cd_for_all_commands(self):
        self.sftp.files["/elsewhere"] = None
        commands = ["cd /elsewhere", "ls ~", "get ~/remote 'local copy'",
                    "put 'local source' ~/uploaded", "cd ~", "pwd", "exit"]
        with patch("builtins.input", side_effect=commands), \
                patch.object(network, "list_files") as listing, \
                patch.object(network, "get_files") as download, \
                patch.object(network, "put_files") as upload:
            self.assertEqual(network.sftp_prompt(self.sftp), 0)
        listing.assert_called_once_with(self.sftp, "/home/user")
        download.assert_called_once_with(self.sftp, "/home/user/remote", "local copy", recursive=False)
        upload.assert_called_once_with(self.sftp, "local source", "/home/user/uploaded", recursive=False, windows_destination=False)
        self.assertEqual(self.sftp.cwd, "/home/user")

    def test_prompt_rejects_backslashes_before_parsing_or_transfer(self):
        commands = [r"get remote.ibt D:\captures\file.ibt", "exit"]
        with patch("builtins.input", side_effect=commands), patch.object(network, "get_files") as download, \
                patch("sys.stderr", new_callable=io.StringIO) as errors:
            self.assertEqual(network.sftp_prompt(self.sftp), 0)
        download.assert_not_called()
        self.assertIn("forward slashes", errors.getvalue())

    def test_explicit_remote_link_root_resolves_but_nested_links_are_refused(self):
        self.sftp.files.update({"/private": None, "/private/tmp": None, "/private/tmp/file": b"ok"})
        normalize = self.sftp.normalize
        with patch.object(self.sftp, "normalize", side_effect=lambda path: "/private/tmp" if path == "/tmp" else normalize(path)):
            network.get_files(self.sftp, "/tmp", str(self.directory / "download"), recursive=True)
        self.assertEqual((self.directory / "download" / "file").read_bytes(), b"ok")
        self.sftp.files["/private/tmp/link"] = "LINK"
        with self.assertRaisesRegex(ValueError, "symlink"):
            network.get_files(self.sftp, "/private/tmp", str(self.directory / "refused"), recursive=True)
        self.assertFalse((self.directory / "refused").exists())

    def test_explicit_local_link_root_resolves_but_nested_links_are_refused(self):
        source = self.directory / "source"
        source.mkdir()
        (source / "file").write_bytes(b"ok")
        root_link = self.directory / "root-link"
        try:
            root_link.symlink_to(source, target_is_directory=True)
        except OSError as error:
            self.skipTest("Creating a local symlink requires platform permission: %s" % error)
        network.put_files(self.sftp, str(root_link), "/uploaded", recursive=True)
        self.assertEqual(self.sftp.files["/uploaded/file"], b"ok")
        self.sftp.files["/remote-file"] = b"download"
        network.get_files(self.sftp, "/remote-file", str(root_link / "download"))
        self.assertEqual((source / "download").read_bytes(), b"download")
        (source / "nested-link").symlink_to(source / "file")
        with self.assertRaisesRegex(ValueError, "symlink|reparse"):
            network.put_files(self.sftp, str(source), "/refused", recursive=True)
        self.assertNotIn("/refused", self.sftp.files)

    def test_remote_new_destination_resolves_existing_parent_once(self):
        calls = []
        def normalize(path):
            calls.append(path)
            if path != "/tmp":
                raise FileNotFoundError(errno.ENOENT, "missing")
            return "/private/tmp"
        with patch.object(self.sftp, "normalize", side_effect=normalize):
            self.assertEqual(network.remote_root(self.sftp, "/tmp/new/file"), "/private/tmp/new/file")
        self.assertEqual(calls, ["/tmp/new/file", "/tmp/new", "/tmp"])

    def test_windows_destination_rejects_reserved_roots_and_children_on_any_source_os(self):
        source = self.directory / "source"
        source.mkdir()
        for target in ("/C:/captures/CON.txt", "/C:/captures/trailing."):
            with self.assertRaisesRegex(ValueError, "Windows filename"):
                network.put_files(self.sftp, str(source), target, True, windows_destination=True)
        with patch.object(network, "local_tree", return_value=iter([("", True), ("CON", False)])), \
                self.assertRaisesRegex(ValueError, "Windows filename"):
            network.put_files(self.sftp, str(source), "/C:/captures/tree", True, windows_destination=True)
        self.assertNotIn("/C:/captures/tree", self.sftp.files)
        self.assertEqual(network.safe_name("CON", windows=False), "CON")

    def test_posix_publication_falls_back_for_distinct_enotsup_without_overwrite(self):
        source = self.directory / "staged"
        target = self.directory / "target"
        source.write_bytes(b"new")
        with patch.object(network.os, "name", "posix"), patch.object(network.errno, "ENOTSUP", TEST_DARWIN_ENOTSUP), \
                patch.object(network.os, "link", side_effect=OSError(TEST_DARWIN_ENOTSUP, "Darwin ENOTSUP")):
            network.publish_local(source, target)
            self.assertEqual(target.read_bytes(), b"new")
            with self.assertRaises(FileExistsError):
                network.publish_local(source, target)
        self.assertEqual(target.read_bytes(), b"new")

    def test_windows_key_records_preserve_a_grave_and_virtual_arrows(self):
        self.assertEqual(network.windows_key_text("à", 0, 1) + network.windows_key_text("H", 0, 1), "àH")
        self.assertEqual(network.windows_key_text("\0", network.WINDOWS_VK_UP, 1), "\x1b[A")
        self.assertEqual(network.windows_key_text("\0", network.WINDOWS_VK_LEFT, 2), "\x1b[D\x1b[D")
        self.assertEqual(network.windows_key_text("\0", 0, 1), "")
        self.assertEqual(network.windows_key_text("H", 0, 1, down=False), "")
        decoder = network.codecs.getincrementaldecoder("utf-16-le")("replace")
        parts = []
        for surrogate in ("\ud83d", "\ude00"):
            text = network.windows_key_text(surrogate, network.WINDOWS_VK_MENU, 1, down=False)
            parts.append(decoder.decode(text.encode("utf-16-le", "surrogatepass")))
        self.assertEqual("".join(parts), "😀")

    def test_download_and_upload_tree_with_unicode(self):
        self.sftp.files.update({"/source": None, "/source/sub dir": None, "/source/sub dir/café.txt": b"\x00hello\xff"})
        target = self.directory / "download"
        network.get_files(self.sftp, "/source", str(target), recursive=True)
        self.assertEqual((target / "sub dir" / "café.txt").read_bytes(), b"\x00hello\xff")
        network.put_files(self.sftp, str(target), "/uploaded", recursive=True)
        self.assertEqual(self.sftp.files["/uploaded/sub dir/café.txt"], b"\x00hello\xff")

    def test_directory_requires_explicit_recursive(self):
        with self.assertRaisesRegex(ValueError, "--recursive"):
            network.get_files(self.sftp, "/home", str(self.directory / "target"))
        with self.assertRaisesRegex(ValueError, "--recursive"):
            network.put_files(self.sftp, str(self.directory), "/target")

    def test_existing_file_prevents_any_download(self):
        self.sftp.files.update({"/source": None, "/source/new": b"new", "/source/keep": b"changed"})
        (self.directory / "keep").write_bytes(b"original")
        with self.assertRaises(FileExistsError):
            network.get_files(self.sftp, "/source", str(self.directory), True)
        self.assertEqual((self.directory / "keep").read_bytes(), b"original")
        self.assertFalse((self.directory / "new").exists())

    def test_existing_remote_file_remains_unchanged(self):
        self.sftp.files["/keep"] = b"original"
        source = self.directory / "new"
        source.write_bytes(b"changed")
        with self.assertRaises(FileExistsError):
            network.put_files(self.sftp, str(source), "/keep")
        self.assertEqual(self.sftp.files["/keep"], b"original")

    def test_failed_download_leaves_no_partial_or_staging_file(self):
        self.sftp.files["/source"] = b"partial"
        self.sftp.fail_download = True
        with self.assertRaises(OSError):
            network.get_files(self.sftp, "/source", str(self.directory / "target"))
        self.assertEqual(list(self.directory.iterdir()), [])

    def test_malicious_remote_names_rejected_before_local_changes(self):
        self.sftp.extra_entries = [SimpleNamespace(filename="../../outside", st_mode=stat.S_IFREG)]
        with self.assertRaisesRegex(ValueError, "Unsafe"):
            network.get_files(self.sftp, "/home/user", str(self.directory / "target"), True)
        self.assertFalse((self.directory / "target").exists())

    def test_remote_symlink_ancestor_rejected(self):
        self.sftp.files["/link"] = "LINK"
        with self.assertRaisesRegex(ValueError, "symlink"):
            network.get_files(self.sftp, "/link/file", str(self.directory / "target"))

    def test_local_symlink_or_junction_ancestor_rejected(self):
        parent = self.directory / "link"
        original = Path.lstat

        def attributes(path):
            if path == parent:
                return SimpleNamespace(st_mode=stat.S_IFDIR, st_file_attributes=network.WINDOWS_REPARSE_POINT_ATTRIBUTE)
            return original(path)
        with patch.object(Path, "lstat", attributes), self.assertRaisesRegex(ValueError, "reparse"):
            network.local_path(parent / "outside")
        with self.assertRaises(ValueError):
            network.local_path(self.directory / ".." / "outside")

    @unittest.skipUnless(os.name == "nt", "Windows rename contract")
    def test_exfat_download_uses_exclusive_rename_without_hardlinks(self):
        source = self.directory / "staged"
        target = self.directory / "target"
        source.write_bytes(b"new")
        with patch.object(network.os, "link", side_effect=OSError(errno.EOPNOTSUPP, "exFAT")):
            network.publish_local(source, target)
        self.assertEqual(target.read_bytes(), b"new")
        source.write_bytes(b"replacement")
        with self.assertRaises(FileExistsError):
            network.publish_local(source, target)
        self.assertEqual(target.read_bytes(), b"new")

    def test_drain_handles_both_streams_before_nonzero_exit(self):
        channel = Mock()
        channel.closed = True
        stdout = [b"stdout-a", b"stdout-b"]
        stderr = [b"stderr-a", b"stderr-b"]
        channel.recv_ready.side_effect = lambda: bool(stdout)
        channel.recv_stderr_ready.side_effect = lambda: bool(stderr)
        channel.recv.side_effect = lambda _: stdout.pop(0)
        channel.recv_stderr.side_effect = lambda _: stderr.pop(0)
        channel.exit_status_ready.return_value = True

        def status():
            self.assertEqual(stdout + stderr, [])
            return 37
        channel.recv_exit_status.side_effect = status
        with patch("sys.stderr", new_callable=io.StringIO) as errors:
            self.assertEqual(network.drain(channel), 37)
            self.assertEqual(errors.getvalue(), "stderr-astderr-b")

    def test_missing_remote_exit_status_is_failure(self):
        channel = Mock()
        channel.closed = True
        channel.recv_ready.return_value = False
        channel.recv_stderr_ready.return_value = False
        channel.exit_status_ready.return_value = True
        channel.recv_exit_status.return_value = -1
        self.assertEqual(network.drain(channel), network.MISSING_EXIT_STATUS_CODE)

    def test_posix_shell_stops_reading_stdin_after_eof_and_drains_output(self):
        channel = Mock()
        channel.closed = False
        channel.recv_ready.side_effect = [False, True, False, False]
        channel.recv.return_value = b"output after stdin closes"
        channel.exit_status_ready.side_effect = [False, False, True]
        channel.recv_exit_status.return_value = 17
        client = Mock()
        client.invoke_shell.return_value = contextlib.nullcontext(channel)
        input_stream = Mock()
        input_stream.isatty.return_value = True
        with patch.object(network.os, "name", "posix"), \
                patch.object(network, "raw_terminal", return_value=contextlib.nullcontext()), \
                patch.object(network.shutil, "get_terminal_size", return_value=os.terminal_size(network.DEFAULT_TERMINAL_SIZE)), \
                patch("sys.stdin", input_stream), patch("sys.stdout.isatty", return_value=True), \
                patch("select.select", return_value=([input_stream], [], [])) as select_input, \
                patch.object(network.os, "read", return_value=b"") as read_input, \
                patch.object(network.time, "sleep", side_effect=lambda _: setattr(channel, "closed", True)) as sleep:
            self.assertEqual(network.shell(client), 17)
        select_input.assert_called_once()
        read_input.assert_called_once()
        channel.shutdown_write.assert_called_once()
        channel.recv.assert_called_once_with(network.IO_CHUNK_SIZE)
        sleep.assert_called_once_with(network.CHANNEL_POLL_SECONDS)

    def test_drain_waits_for_delayed_output_after_exit_status(self):
        channel = Mock()
        channel.closed = False
        channel.exit_status_ready.return_value = True
        channel.recv_exit_status.return_value = 17
        stdout, stderr = [], []
        channel.recv_ready.side_effect = lambda: bool(stdout)
        channel.recv_stderr_ready.side_effect = lambda: bool(stderr)
        channel.recv.side_effect = lambda _: stdout.pop(0)
        channel.recv_stderr.side_effect = lambda _: stderr.pop(0)

        def late_output():
            stdout.append(b"late stdout")
            stderr.append(b"late stderr")
        ticks = iter((late_output, lambda: setattr(channel, "closed", True)))
        with patch.object(network.time, "sleep", side_effect=lambda _: next(ticks)()), \
                patch("sys.stdout", new_callable=io.StringIO) as output, \
                patch("sys.stderr", new_callable=io.StringIO) as errors:
            self.assertEqual(network.drain(channel), 17)
        self.assertEqual(output.getvalue(), "late stdout")
        self.assertEqual(errors.getvalue(), "late stderr")
        self.assertTrue(channel.closed)

    def test_shell_waits_for_delayed_terminal_output_after_exit_status(self):
        channel = Mock()
        channel.closed = False
        channel.exit_status_ready.return_value = True
        channel.recv_exit_status.return_value = 17
        pending = []
        channel.recv_ready.side_effect = lambda: bool(pending)
        channel.recv.side_effect = lambda _: pending.pop(0)
        client = Mock()
        client.invoke_shell.return_value = contextlib.nullcontext(channel)
        input_stream = Mock()
        input_stream.isatty.return_value = True
        ticks = iter((lambda: pending.append(b"late terminal output"), lambda: setattr(channel, "closed", True)))

        def poll_input(*args):
            next(ticks)()
            return [], [], []
        with patch.object(network.os, "name", "posix"), \
                patch.object(network, "raw_terminal", return_value=contextlib.nullcontext()), \
                patch.object(network.shutil, "get_terminal_size", return_value=os.terminal_size(network.DEFAULT_TERMINAL_SIZE)), \
                patch("sys.stdin", input_stream), \
                patch("sys.stdout", new_callable=io.StringIO) as output, \
                patch("sys.stdout.isatty", return_value=True), \
                patch("select.select", side_effect=poll_input):
            self.assertEqual(network.shell(client), 17)
        self.assertEqual(output.getvalue(), "late terminal output")
        self.assertTrue(channel.closed)

    def test_first_host_key_persists_and_changed_key_is_rejected(self):
        try:
            paramiko = network.dependency()
        except RuntimeError:
            self.skipTest("Run setup for host-key tests")
        path = self.directory / "known_hosts"
        path.write_text("# preserve this comment", encoding="utf-8")
        key = paramiko.RSAKey.generate(TEST_HOST_KEY_BITS)
        other_key = paramiko.RSAKey.generate(TEST_HOST_KEY_BITS)
        policy = network.known_hosts_policy(paramiko, path)
        client = paramiko.SSHClient()
        with patch("sys.stderr", new_callable=io.StringIO) as errors:
            policy.missing_host_key(client, "host", key)
            self.assertIn("SHA256:", errors.getvalue())
        saved = path.read_text(encoding="utf-8")
        self.assertTrue(saved.startswith("# preserve this comment\nhost "))
        policy.missing_host_key(client, "host", key)
        self.assertEqual(path.read_text(encoding="utf-8"), saved)
        with self.assertRaises(paramiko.SSHException):
            policy.missing_host_key(client, "host", other_key)
        self.assertEqual(path.read_text(encoding="utf-8"), saved)

    @unittest.skipUnless(os.name == "nt", "PowerShell 5.1 argv forwarding")
    def test_powershell_wrapper_preserves_quotes_dollars_unicode_empty_and_exit(self):
        wrapper = self.directory / "home-network.ps1"
        shutil.copyfile(Path(__file__).with_name("home-network.ps1"), wrapper)
        (self.directory / "home_network.py").write_text(
            "import sys,json; print(json.dumps(sys.argv[1:])); sys.exit(37)", encoding="utf-8")
        args = ["FRANK", "exec", '$a = "hello café"; Write-Output "$a"', "C:\\path with spaces\\", "", 'a\\"b']
        script = "& " + " ".join("'" + item.replace("'", "''") + "'" for item in [str(wrapper), *args])
        script += " 2024; exit $LASTEXITCODE"
        expected = args + ["2024"]
        result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 37, result.stderr)
        self.assertEqual(json.loads(result.stdout), expected)
        # Outer subprocess capture is insufficient: PowerShell itself must receive native stdout.
        command = "& " + " ".join("'" + item.replace("'", "''") + "'" for item in [str(wrapper), *args]) + " 2024"
        pipeline = "$captured = " + command + "; $status = $LASTEXITCODE; $captured | Write-Output; exit $status"
        result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", pipeline],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 37, result.stderr)
        self.assertEqual(json.loads(result.stdout), expected)

    @unittest.skipUnless(os.name == "nt", "PowerShell Unicode stdout capture")
    def test_powershell_wrapper_unicode_output_and_encoding_restoration(self):
        wrapper = self.directory / "home-network.ps1"
        shutil.copyfile(Path(__file__).with_name("home-network.ps1"), wrapper)
        (self.directory / "home_network.py").write_text("print('café_日本語')", encoding="utf-8")
        # Simulate a normal legacy console, capture inside PS, then serialize as ASCII code points.
        script = "[Console]::OutputEncoding = [Text.Encoding]::GetEncoding(437); "
        script += "$captured = & '" + str(wrapper).replace("'", "''") + "'; "
        script += "$codepoints = @($captured.ToCharArray() | ForEach-Object { [int]$_ }); "
        script += "Write-Output ($codepoints -join ','); Write-Output ([Console]::OutputEncoding.CodePage)"
        result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = result.stdout.splitlines()
        self.assertEqual([int(value) for value in lines[0].split(",")], [ord(char) for char in "café_日本語"])
        self.assertEqual(lines[1], "437")

    @unittest.skipUnless(os.name == "nt", "PowerShell 5.1 redirected native stderr")
    def test_powershell_redirected_stderr_preserves_stdout_and_status(self):
        wrapper = self.directory / "home-network.ps1"
        shutil.copyfile(Path(__file__).with_name("home-network.ps1"), wrapper)
        (self.directory / "home_network.py").write_text(
            "import sys; print('remote-error',file=sys.stderr); print('remote-output'); sys.exit(37)", encoding="utf-8")
        for redirect in ("2>&1", "2>$null"):
            script = "$ErrorActionPreference = 'Stop'; $captured = & '" + str(wrapper).replace("'", "''") + "' " + redirect
            script += "; $status = $LASTEXITCODE; $captured | ForEach-Object { Write-Output $_.ToString() }; exit $status"
            result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 37, result.stderr)
            self.assertIn("remote-output", result.stdout)
            if redirect == "2>&1":
                self.assertIn("remote-error", result.stdout)

    @unittest.skipUnless(os.name == "nt", "PowerShell 5.1 native setup stages")
    def test_powershell_setup_preserves_native_stderr_and_stage_failures(self):
        wrapper = self.directory / "home-network.ps1"
        shutil.copyfile(Path(__file__).with_name("home-network.ps1"), wrapper)
        shim_directory = self.directory / "bin"
        shim_directory.mkdir()
        shim = shim_directory / "py.exe"
        source = self.directory / "NativeSetupStub.cs"
        source.write_text(r'''
using System;
using System.IO;
using System.Reflection;
class NativeSetupStub {
    public static int Main(string[] args) {
        string stage = args[0] == "-3" ? "bootstrap" : args[0] == "-c" ? "version" : args[1];
        File.AppendAllText(Environment.GetEnvironmentVariable("HOME_NETWORK_TEST_CALLS"), stage + "\n");
        Console.Error.WriteLine("benign-warning-" + stage);
        if (Environment.GetEnvironmentVariable("HOME_NETWORK_TEST_FAIL_STAGE") == stage)
            return Int32.Parse(Environment.GetEnvironmentVariable("HOME_NETWORK_TEST_FAIL_CODE"));
        string executable = Assembly.GetExecutingAssembly().Location;
        if (stage == "bootstrap") Console.WriteLine(executable);
        else if (stage == "venv") {
            string destination = Path.Combine(args[2], "Scripts");
            Directory.CreateDirectory(destination);
            File.Copy(executable, Path.Combine(destination, "python.exe"), true);
        }
        return 0;
    }
}
''', encoding="utf-8")
        compile_script = "Add-Type -Path '" + str(source).replace("'", "''") + "' -OutputAssembly '" + str(shim).replace("'", "''") + "' -OutputType ConsoleApplication"
        compiled = subprocess.run(["powershell.exe", "-NoProfile", "-Command", compile_script], capture_output=True, text=True)
        self.assertEqual(compiled.returncode, 0, compiled.stderr)
        stages = ["bootstrap", "version", "venv", "pip"]
        for redirect in ("2>&1", "2>$null"):
            for failed_stage in ("", *stages):
                with self.subTest(redirect=redirect, failed_stage=failed_stage):
                    calls = self.directory / "calls.txt"
                    calls.write_text("", encoding="utf-8")
                    environment = dict(os.environ, PATH=str(shim_directory) + os.pathsep + os.environ["PATH"],
                                       LOCALAPPDATA=str(self.directory / "local-app-data"),
                                       HOME_NETWORK_TEST_CALLS=str(calls), HOME_NETWORK_TEST_FAIL_STAGE=failed_stage,
                                       HOME_NETWORK_TEST_FAIL_CODE=str(TEST_SETUP_FAILURE_CODE))
                    command = "$captured = & '" + str(wrapper).replace("'", "''") + "' setup " + redirect
                    command += "; $status = $LASTEXITCODE; $captured | ForEach-Object { Write-Output $_.ToString() }; exit $status"
                    result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", command],
                                            env=environment, capture_output=True, text=True)
                    expected_code = (1 if failed_stage in ("bootstrap", "version") else TEST_SETUP_FAILURE_CODE) if failed_stage else 0
                    self.assertEqual(result.returncode, expected_code, result.stdout + result.stderr)
                    expected_stages = stages[:stages.index(failed_stage) + 1] if failed_stage else stages
                    self.assertEqual(calls.read_text(encoding="utf-8").splitlines(), expected_stages)
                    if not failed_stage:
                        self.assertIn("Home-network tools installed", result.stdout)
                        if redirect == "2>&1":
                            for stage in stages:
                                self.assertIn("benign-warning-" + stage, result.stdout)

    @unittest.skipUnless(os.name == "nt", "PowerShell launcher failures and native status scope")
    def test_powershell_launcher_fails_closed_and_preserves_native_status(self):
        for redirect in ("", "2>&1", "2>$null"):
            for scenario in ("baseline-zero", "baseline-nonzero", "missing-localappdata", "invalid-venv"):
                with self.subTest(redirect=redirect, scenario=scenario), tempfile.TemporaryDirectory(dir=self.directory) as temporary:
                    root = Path(temporary).resolve()
                    wrapper = root / "home-network.ps1"
                    shutil.copyfile(Path(__file__).with_name("home-network.ps1"), wrapper)
                    (root / "home_network.py").write_text(
                        "import os,pathlib,sys; pathlib.Path(__file__).with_name('ran').write_text('yes'); "
                        "print('helper-output'); print('helper-error',file=sys.stderr); "
                        "sys.exit(int(os.environ['HOME_NETWORK_TEST_RESULT']))", encoding="utf-8")
                    expected_status = 0 if scenario == "baseline-zero" else 37
                    environment = dict(os.environ, LOCALAPPDATA=str(root / "app-data"),
                                       HOME_NETWORK_TEST_RESULT=str(expected_status))
                    if scenario == "missing-localappdata":
                        del environment["LOCALAPPDATA"]
                    elif scenario == "invalid-venv":
                        executable = root / "app-data" / "HomeNetwork" / "venv-v1" / "Scripts" / "python.exe"
                        executable.parent.mkdir(parents=True)
                        executable.write_bytes(b"not an executable")
                    command = "$captured = & '" + str(wrapper).replace("'", "''") + "' " + redirect
                    command += "; $status = $LASTEXITCODE; $captured | ForEach-Object { Write-Output $_.ToString() }; exit $status"
                    result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", command],
                                            env=environment, capture_output=True, text=True)
                    if scenario.startswith("baseline"):
                        self.assertEqual(result.returncode, expected_status, result.stdout + result.stderr)
                        self.assertTrue((root / "ran").exists())
                        self.assertIn("helper-output", result.stdout)
                        if redirect == "2>&1":
                            self.assertIn("helper-error", result.stdout)
                        elif redirect == "2>$null":
                            self.assertNotIn("helper-error", result.stdout + result.stderr)
                        else:
                            self.assertIn("helper-error", result.stderr)
                    else:
                        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                        self.assertFalse((root / "ran").exists())


if __name__ == "__main__":
    unittest.main()

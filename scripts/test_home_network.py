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
        self.directory = Path(self.temporary.name)
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
        upload.assert_called_once_with(self.sftp, "local source", "/home/user/uploaded", recursive=False)
        self.assertEqual(self.sftp.cwd, "/home/user")

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
                return SimpleNamespace(st_mode=stat.S_IFDIR, st_file_attributes=0x400)
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
        channel.recv_ready.return_value = False
        channel.recv_stderr_ready.return_value = False
        channel.exit_status_ready.return_value = True
        channel.recv_exit_status.return_value = -1
        self.assertEqual(network.drain(channel), 255)

    def test_first_host_key_persists_and_changed_key_is_rejected(self):
        try:
            paramiko = network.dependency()
        except RuntimeError:
            self.skipTest("Run setup for host-key tests")
        path = self.directory / "known_hosts"
        path.write_text("# preserve this comment", encoding="utf-8")
        key = paramiko.RSAKey.generate(1024)
        other_key = paramiko.RSAKey.generate(1024)
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
        script += "; exit $LASTEXITCODE"
        result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 37, result.stderr)
        self.assertEqual(json.loads(result.stdout), args)
        # Outer subprocess capture is insufficient: PowerShell itself must receive native stdout.
        command = "& " + " ".join("'" + item.replace("'", "''") + "'" for item in [str(wrapper), *args])
        pipeline = "$captured = " + command + "; $status = $LASTEXITCODE; $captured | Write-Output; exit $status"
        result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", pipeline],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 37, result.stderr)
        self.assertEqual(json.loads(result.stdout), args)

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


if __name__ == "__main__":
    unittest.main()

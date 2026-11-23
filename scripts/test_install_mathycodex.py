"""Exercise installer file safety without compiling Codex or changing the user's shell."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().with_name("install-mathycodex.sh")


@unittest.skipUnless(sys.platform == "darwin", "macOS release installer")
class MathyCodexInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="mathycodex install ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.release = self.root / "target" / "release"
        self.release.mkdir(parents=True)
        for name in ("codex", "codex-code-mode-host", "codex-exec-server"):
            path = self.release / name
            path.write_text("#!/bin/sh\necho installer-fixture\n")
            path.chmod(0o755)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.official = self.bin / "codex"
        self.official.write_text("existing official codex")
        self.env = {**os.environ,
                    "MATHYCODEX_TARGET_DIR": str(self.release.parent),
                    "MATHYCODEX_INSTALL_DIR": str(self.root / "lib"),
                    "MATHYCODEX_BIN_DIR": str(self.bin)}

    def run_installer(self):
        return subprocess.run([str(INSTALLER), "--no-build"], env=self.env,
                              capture_output=True, text=True)

    def test_installs_release_and_helpers_without_replacing_codex(self):
        for _ in range(2):
            result = self.run_installer()
            self.assertEqual(result.returncode, 0, result.stderr)
        command = self.bin / "mathycodex"
        self.assertTrue(command.is_symlink())
        output = subprocess.check_output([str(command)], text=True)
        self.assertEqual(output.strip(), "installer-fixture")
        for name in ("codex-code-mode-host", "codex-exec-server"):
            self.assertTrue(os.access(command.resolve().with_name(name), os.X_OK))
        self.assertEqual(self.official.read_text(), "existing official codex")

    def test_refuses_unrelated_existing_command(self):
        path = self.bin / "mathycodex"
        path.write_text("unrelated command")
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Refusing", result.stderr)
        self.assertEqual(path.read_text(), "unrelated command")
        self.assertFalse((self.root / "lib").exists())

    def test_missing_helper_leaves_existing_installation_untouched(self):
        self.assertEqual(self.run_installer().returncode, 0)
        installed = (self.bin / "mathycodex").resolve()
        previous = installed.read_bytes()
        (self.release / "codex").write_text("new binary")
        (self.release / "codex-exec-server").unlink()
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(installed.read_bytes(), previous)


if __name__ == "__main__":
    unittest.main()

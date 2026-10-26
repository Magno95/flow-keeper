#!/usr/bin/env python3
"""Exercise the macOS installer without touching installed apps or user data."""

import hashlib
import os
from pathlib import Path
import plistlib
import subprocess
import tarfile
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().with_name("install.sh")


@unittest.skipUnless(os.uname().sysname == "Darwin", "macOS installer")
class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="flok-installer-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fixtures = self.root / "fixtures"
        self.bundle = self.fixtures / "Flow Keeper.app"
        (self.bundle / "Contents").mkdir(parents=True)
        (self.bundle / "Contents/Info.plist").write_bytes(plistlib.dumps({
            "CFBundleShortVersionString": "0.7.0",
            "CFBundleIdentifier": "io.github.magno95.flowkeeper",
        }))
        self.archive = self.fixtures / "Flow-Keeper_0.7.0_universal.app.tar.gz"
        with tarfile.open(self.archive, "w:gz") as archive:
            archive.add(self.bundle, arcname=self.bundle.name)
        checksum = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        Path(str(self.archive) + ".sha256").write_text(checksum + "  " + self.archive.name + "\n")
        tools = self.root / "bin"
        tools.mkdir()
        stubs = {
            "pgrep": "#!/bin/sh\nexit 1\n",
            "codesign": "#!/bin/sh\nexit 0\n",
            "xattr": "#!/bin/sh\nexit 0\n",
            "open": "#!/bin/sh\nexit 97\n",
            "gh": """#!/bin/bash
if [ "$1" = auth ]; then exit 0; fi
if [ "$1" != release ] || [ "$2" != download ]; then exit 98; fi
while [ "$#" -gt 0 ]; do
  if [ "$1" = --dir ]; then shift; dest="$1"; fi
  shift
done
cp "$FLOW_KEEPER_INSTALL_FIXTURES"/*.app.tar.gz "$dest/"
if [ "$FLOW_KEEPER_INSTALL_TEST_MODE" != missing ]; then
  cp "$FLOW_KEEPER_INSTALL_FIXTURES"/*.sha256 "$dest/"
fi
if [ "$FLOW_KEEPER_INSTALL_TEST_MODE" = corrupt ]; then
  printf corrupt >> "$dest/Flow-Keeper_0.7.0_universal.app.tar.gz"
fi
""",
        }
        for name, content in stubs.items():
            executable = tools / name
            executable.write_text(content)
            executable.chmod(0o755)
        self.env = os.environ.copy()
        self.env.update(
            PATH=str(tools) + ":" + self.env["PATH"],
            FLOW_KEEPER_APP_DIR=str(self.root / "Applications"),
            FLOW_KEEPER_HOME=str(self.root / "data"),
            FLOW_KEEPER_INSTALL_FIXTURES=str(self.fixtures),
            FLOW_KEEPER_INSTALL_TEST_MODE="valid",
        )
        self.target = self.root / "Applications/Flow Keeper.app"

    def install(self, *args):
        return subprocess.run(
            ["bash", str(INSTALLER), *args, "--no-open"],
            env=self.env, capture_output=True, text=True,
        )

    def test_verified_release_installs_and_records_version(self):
        result = self.install("--version", "v0.7.0")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue((self.target / "Contents/Info.plist").is_file())
        import json
        manifest = json.loads((self.root / "data/install-manifest.json").read_text())
        self.assertEqual(manifest["version"], "0.7.0")
        self.assertEqual(manifest["appPath"], str(self.target))

    def test_corrupt_archive_does_not_replace_existing_app(self):
        self.env["FLOW_KEEPER_INSTALL_TEST_MODE"] = "corrupt"
        self.target.mkdir(parents=True)
        marker = self.target / "existing-app"
        marker.write_text("keep")
        result = self.install("--version", "v0.7.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum does not match", result.stderr)
        self.assertEqual(marker.read_text(), "keep")

    def test_missing_checksum_does_not_install(self):
        self.env["FLOW_KEEPER_INSTALL_TEST_MODE"] = "missing"
        result = self.install("--version", "v0.7.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum is missing", result.stderr)
        self.assertFalse(self.target.exists())

    def test_invalid_bundle_does_not_install(self):
        (self.bundle / "Contents/Info.plist").unlink()
        result = self.install("--bundle", str(self.bundle))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Invalid app bundle", result.stderr)
        self.assertFalse(self.target.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)

import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "deploy" / "update-vps.sh"


class UpdateVpsTests(unittest.TestCase):
    def test_script_parses(self):
        subprocess.run(["bash", "-n", SCRIPT], check=True)

    def test_exact_main_locked_release_and_rollback_are_required(self):
        source = SCRIPT.read_text(encoding="utf-8")
        self.assertIn("git merge --ff-only origin/main", source)
        self.assertIn("git rev-parse origin/main", source)
        self.assertIn('cargo "+$toolchain" build --locked --release', source)
        self.assertIn("build/release/tailgate", source)
        self.assertIn("/usr/local/lib/tailgate", source)
        self.assertIn("rollback()", source)
        self.assertIn('actual_exe=$(sudo readlink -f "/proc/$pid/exe"', source)

    def test_service_runs_the_versioned_current_release(self):
        unit = (ROOT / "deploy" / "tailgate.service").read_text(encoding="utf-8")
        self.assertIn(
            "ExecStart=/usr/local/lib/tailgate/current/tailgate",
            unit,
        )

    def test_toolchain_and_build_directory_are_pinned(self):
        self.assertIn('channel = "1.92.0"', (ROOT / "rust-toolchain.toml").read_text())
        self.assertIn('target-dir = "build"', (ROOT / ".cargo" / "config.toml").read_text())


if __name__ == "__main__":
    unittest.main()

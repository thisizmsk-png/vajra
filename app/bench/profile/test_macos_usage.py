"""Native ledger proof. No app window, agent, network, or user data directory."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


@unittest.skipUnless(sys.platform == "darwin" and shutil.which("clang"), "macOS and clang required")
class NativeUsageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.helper = str(Path(cls.tmp.name) / "macos_usage")
        try:
            subprocess.run(["clang", "-Wall", "-Wextra", "-Werror", "-O2",
                            str(Path(__file__).with_name("macos_usage.c")), "-o", cls.helper], check=True)
        except BaseException:
            cls.tmp.cleanup()
            raise

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_invalid_batch_has_no_partial_json(self):
        for argument in ["0", "-1", "", "1junk", "2147483648", "9" * 100]:
            with self.subTest(argument=argument):
                result = subprocess.run([self.helper, str(os.getpid()), argument],
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")

    def test_missing_pid_is_an_error_in_a_complete_batch(self):
        result = subprocess.run([self.helper, str(os.getpid()), "2147483647"],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        rows = json.loads(result.stdout)
        self.assertGreater(rows[0]["rssBytes"], 0)
        self.assertEqual(rows[1]["pid"], 2147483647)
        self.assertGreater(rows[1]["error"], 0)
        self.assertNotIn("rssBytes", rows[1])
        self.assertNotIn("footprintBytes", rows[1])

    def test_touched_allocation_moves_both_ledgers_and_keeps_identity(self):
        child = subprocess.Popen([sys.executable, "-u", "-c",
            "import sys; print('ready'); sys.stdin.readline(); "
            "b=bytearray(32*1024*1024); b[::4096]=b'x'*(len(b)//4096); "
            "print('grown'); sys.stdin.readline()"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(), "ready")
            before = json.loads(subprocess.check_output([self.helper, str(child.pid)], text=True))[0]
            child.stdin.write("grow\n")
            child.stdin.flush()
            self.assertEqual(child.stdout.readline().strip(), "grown")
            after = json.loads(subprocess.check_output([self.helper, str(child.pid)], text=True))[0]
            self.assertEqual(before["startAbs"], after["startAbs"])
            for key in ["rssBytes", "footprintBytes"]:
                self.assertGreater(before[key], 0)
                self.assertGreater(after[key] - before[key], 24 * 1024 * 1024)
        finally:
            child.stdin.close()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
            child.stdout.close()


if __name__ == "__main__":
    unittest.main()

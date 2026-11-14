"""Spike S12 helper: run a command in a pty for up to N seconds, answering the terminal queries an interactive Codex
sends at start (cursor position, device attributes, colours), as the app's own pty would. Stops one second after
<stopfile> holds <needle>.

    python3 -I s12_pty.py <seconds> <stopfile> <needle> -- <argv...>
"""
import os
import pty
import select
import signal
import sys
import time

secs, stopfile, needle = float(sys.argv[1]), sys.argv[2], sys.argv[3]
argv = sys.argv[sys.argv.index("--") + 1:]
pid, fd = pty.fork()
if pid == 0:
    os.execvp(argv[0], argv)
ANSWERS = {
    b"\x1b[6n": b"\x1b[1;1R",
    b"\x1b[c": b"\x1b[?62;22c",
    b"\x1b]10;?\x1b\\": b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\",
    b"\x1b]11;?\x1b\\": b"\x1b]11;rgb:0000/0000/0000\x1b\\",
    b"\x1b[?u": b"\x1b[?0u",
}
end = time.time() + secs
while time.time() < end:
    r, _, _ = select.select([fd], [], [], 0.2)
    if r:
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        for q, a in ANSWERS.items():
            if q in chunk:
                os.write(fd, a)
    try:
        with open(stopfile) as f:
            if needle in f.read():
                time.sleep(1.0)
                break
    except OSError:
        pass
for s in (signal.SIGTERM, signal.SIGKILL):
    try:
        os.kill(pid, s)
        time.sleep(0.5)
    except OSError:
        break

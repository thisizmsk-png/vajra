#!/usr/bin/env python3
"""Stand-in for a Claude Code run session: prints to its PTY and drives N workers' transcripts.
usage: fake_agent.py HOME CWD SESSION_ID WORKERS LINES_PER_S_PER_WORKER SECONDS"""
import json, os, re, sys, time
home, cwd, sid, workers, rate, secs = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4]), float(sys.argv[5]), float(sys.argv[6])
slug = re.sub(r"[^A-Za-z0-9]", "-", cwd)
sub = os.path.join(home, ".claude/projects", slug, sid, "subagents")
os.makedirs(sub, exist_ok=True)
fx = os.environ["FG12_FIXTURE"]
lines = open(os.path.join(fx, "agent-aa870dd40540481f0.jsonl")).read().splitlines()
meta = open(os.path.join(fx, "agent-aa870dd40540481f0.meta.json")).read()
files = []
for i in range(workers):
    aid = f"aa870dd40540481{i:02x}"[:17]
    open(os.path.join(sub, f"agent-{aid}.meta.json"), "w").write(meta)
    files.append(open(os.path.join(sub, f"agent-{aid}.jsonl"), "a"))
    files[-1].write(lines[0].replace("aa870dd40540481f0", aid) + "\n"); files[-1].flush()
print("fake run session up, workers:", workers, flush=True)
t0, n = time.time(), 0
while time.time() - t0 < secs:
    for i, f in enumerate(files):
        if rate > 0:
            f.write(lines[1 + (n % (len(lines) - 1))].replace("aa870dd40540481f0", f"w{i}") + "\n"); f.flush()
    if n % 10 == 0:
        print(f"[{int(time.time()-t0)}s] step {n} working on task {n % 7}", flush=True)
    n += 1
    time.sleep(1.0 / rate if rate > 0 else 1.0)

#!/usr/bin/env python3
"""Stand-in for `capped`: the same interface, a wall-clock cap only."""
import subprocess
import sys

args = sys.argv[1:]
secs = float(args[args.index("--time") + 1])
command = args[args.index("--") + 1:]
child = subprocess.Popen(command)
try:
    sys.exit(child.wait(timeout=secs))
except subprocess.TimeoutExpired:
    child.kill()
    sys.exit(124)

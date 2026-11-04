#!/usr/bin/env python3
"""Regenerate every TUI declutter mockup, check them, and rebuild ../compare.html."""
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
STEPS = [
    "gen_screens.py", "gen_dialogs.py", "gen_notices.py",
    "light_convert.py", "mono_convert.py",
    "check_screens.py", "check_dialogs.py", "check_notices.py",
    "light_check.py", "mono_check.py",
    "render.py",
]
for step in STEPS:
    print(f"== {step}", flush=True)
    r = subprocess.run([sys.executable, str(HERE / step)], cwd=HERE)
    if r.returncode != 0:
        sys.exit(f"{step} failed ({r.returncode})")
print("build ok")

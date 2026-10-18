#!/usr/bin/python -I
"""Build the pinned worktree before publication without changing it or installing."""

import argparse
import os
import pathlib
import re
import shutil
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("worktree", type=pathlib.Path)
parser.add_argument("output", type=pathlib.Path)
parser.add_argument("--nocheck", action="store_true")
args = parser.parse_args()
repo = args.worktree.resolve(strict=True)
recipe = repo / "packaging/arch"
text = (recipe / "PKGBUILD").read_text()
commit = re.search(r"^_commit=([0-9a-f]{40})$", text, re.M).group(1)
actual = subprocess.check_output(
    ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True
).strip()
if actual != commit:
    raise SystemExit("Worktree HEAD does not match the recipe pin")
subprocess.run(
    [
        "git",
        "-C",
        str(repo),
        "diff",
        "--exit-code",
        "HEAD",
        "--",
        ".",
        ":!packaging/arch",
    ],
    check=True,
)
output = args.output.resolve()
output.mkdir(mode=0o700, parents=True, exist_ok=False)
(output / "src").mkdir()
(output / "src/oo7").symlink_to(repo, target_is_directory=True)
for name in [
    "PKGBUILD",
    "render-data.py",
    "org.freedesktop.secrets.service",
    "README.md",
]:
    shutil.copyfile(recipe / name, output / name)
    if name != "PKGBUILD":
        (output / "src" / name).symlink_to(output / name)
env = os.environ.copy()
env.pop("DBUS_SESSION_BUS_ADDRESS", None)
env.pop("DBUS_SYSTEM_BUS_ADDRESS", None)
command = ["makepkg", "--noextract", "--noconfirm", "--holdver"]
if args.nocheck:
    command.append("--nocheck")
raise SystemExit(subprocess.run(command, cwd=output, env=env).returncode)

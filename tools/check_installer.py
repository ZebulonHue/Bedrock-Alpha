"""Check that every file the installer script references actually exists.

Inno Setup only discovers a missing or misnamed `Source:` when it runs, which
on CI means finding out eight minutes into a release build. This resolves the
`#define`s the same way Inno does and checks the paths up front, so a rename
that was applied in one place and not the other fails here instead.

    python tools/check_installer.py

Run it from the repository root with `cargo build --release` already done.
Exits non-zero if anything is missing.
"""

import os
import re
import sys

SCRIPT = os.path.join("installer", "project-bedrock.iss")


def main():
    with open(SCRIPT, encoding="utf-8") as handle:
        text = handle.read()

    defines = dict(re.findall(r'#define\s+(\w+)\s+"([^"]*)"', text))
    sep = "\\"
    missing = []

    for source in re.findall(r'^Source:\s*"([^"]+)"', text, re.M):
        resolved = re.sub(
            r"\{#(\w+)\}", lambda m: defines.get(m.group(1), m.group(0)), source
        )
        # Paths in the script are relative to the script's own directory.
        path = os.path.normpath(
            os.path.join("installer", *resolved.split(sep))
        )
        present = os.path.isfile(path)
        if not present:
            missing.append(path)
        print(f"  {'ok  ' if present else 'MISS'}  {resolved}")

    if missing:
        print(f"\n{len(missing)} missing file(s); the installer build would fail:")
        for path in missing:
            print(f"    {path}")
        return 1
    print("\nall installer sources present")
    return 0


if __name__ == "__main__":
    sys.exit(main())

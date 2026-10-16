#!/usr/bin/env python3
"""Translation management tool for Immermemo Slint UI.

Usage:
    python3 scripts/i18n.py extract        Extract POT template from Slint files
    python3 scripts/i18n.py check          Check for missing or empty translations
    python3 scripts/i18n.py sync           Extract and sync missing entries into PO files
"""

import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
APP_DIR = REPO_ROOT / "app"
UI_DIR = APP_DIR / "ui"
TRANS_DIR = APP_DIR / "translations"
POT_FILE = TRANS_DIR / "immermemo.pot"


def po_field(block, key):
    """The (possibly multi-line) quoted value of `key` in one PO entry, or None.

    A long string is written as `key ""` followed by continuation lines that
    each hold one quoted fragment; the value is the fragments joined.
    """
    m = re.search(r'^' + re.escape(key) + r'\s+"((?:[^"\\]|\\.)*)"[ \t]*\n?((?:^"(?:[^"\\]|\\.)*"[ \t]*\n?)*)', block, re.M)
    if not m:
        return None
    parts = [m.group(1)] + re.findall(r'"((?:[^"\\]|\\.)*)"', m.group(2))
    return "".join(parts)


def parse_po_blocks(file_path):
    """Parse a PO/POT file into a list of entry dicts."""
    if not file_path.exists():
        return []

    with open(file_path, "r", encoding="utf-8") as f:
        content = f.read()

    entries = []
    blocks = content.split("\n\n")
    for block in blocks:
        trimmed = block.strip()
        if not trimmed:
            continue

        ctxt = po_field(trimmed, "msgctxt")
        msg_id = po_field(trimmed, "msgid")
        # The header is the entry whose msgid is empty (and has no context).
        if msg_id == "" and ctxt is None:
            entries.append({"header": True, "block": block})
            continue

        msg_str = po_field(trimmed, "msgstr")
        if msg_str is None:
            msg_str = po_field(trimmed, "msgstr[0]") or ""
        if msg_id is not None:
            entries.append({
                "header": False,
                "ctxt": ctxt,
                "id": msg_id,
                "str": msg_str,
                "block": block
            })
    return entries


def extract_pot():
    """Run slint-tr-extractor to generate POT template."""
    print("Extracting translations from Slint UI...")
    slint_files = list(UI_DIR.glob("**/*.slint"))
    if not slint_files:
        print("No .slint files found under app/ui", file=sys.stderr)
        return False

    cmd = ["slint-tr-extractor", "-o", str(POT_FILE)] + [str(f) for f in slint_files]
    res = subprocess.run(cmd)
    if res.returncode != 0:
        print("Failed to run slint-tr-extractor", file=sys.stderr)
        return False
    print(f"Generated {POT_FILE}")
    return True


def get_po_files():
    """Find all immermemo.po files under app/translations."""
    return list(TRANS_DIR.glob("**/LC_MESSAGES/immermemo.po"))


def check_translations():
    """Check PO files against POT template."""
    if not POT_FILE.exists():
        if not extract_pot():
            return 1

    pot_entries = [e for e in parse_po_blocks(POT_FILE) if not e.get("header") and e.get("id")]
    po_files = get_po_files()

    if not po_files:
        print("No PO files found under app/translations", file=sys.stderr)
        return 1

    has_issues = False
    for po_path in po_files:
        lang = po_path.relative_to(TRANS_DIR).parts[0]
        po_entries = parse_po_blocks(po_path)

        po_map = {}
        for e in po_entries:
            if not e.get("header") and e.get("id"):
                po_map[(e["ctxt"], e["id"])] = e["str"]

        missing = []
        empty = []

        for e in pot_entries:
            key = (e["ctxt"], e["id"])
            if key not in po_map:
                missing.append(e)
            elif not po_map[key]:
                empty.append(e)

        print(f"=== Language: {lang} ({po_path}) ===")
        print(f"  Total keys in POT: {len(pot_entries)}")
        print(f"  Missing keys: {len(missing)}")
        print(f"  Empty translations: {len(empty)}")

        if missing:
            has_issues = True
            print("  [Missing entries]")
            for m in missing[:10]:
                print(f"    - ctxt: {m['ctxt'] or '<none>'} | id: \"{m['id']}\"")
            if len(missing) > 10:
                print(f"    ... and {len(missing) - 10} more")

        if empty:
            has_issues = True
            print("  [Empty translations]")
            for em in empty[:10]:
                print(f"    - ctxt: {em['ctxt'] or '<none>'} | id: \"{em['id']}\"")
            if len(empty) > 10:
                print(f"    ... and {len(empty) - 10} more")
        print()

    return 1 if has_issues else 0


def sync_translations():
    """Extract POT and add missing keys to PO files."""
    if not extract_pot():
        return 1

    pot_entries = [e for e in parse_po_blocks(POT_FILE) if not e.get("header") and e.get("id")]
    po_files = get_po_files()

    for po_path in po_files:
        po_entries = parse_po_blocks(po_path)

        existing_keys = set()
        id_to_existing_translation = {}

        for e in po_entries:
            if not e.get("header") and e.get("id"):
                existing_keys.add((e["ctxt"], e["id"]))
                if e["str"]:
                    id_to_existing_translation[e["id"]] = e["str"]

        to_append = []
        for e in pot_entries:
            key = (e["ctxt"], e["id"])
            if key not in existing_keys:
                to_append.append(e)

        if not to_append:
            print(f"No missing entries for {po_path}")
            continue

        print(f"Appending {len(to_append)} missing entries to {po_path}...")
        with open(po_path, "a", encoding="utf-8") as f:
            for item in to_append:
                f.write("\n")
                if item["ctxt"]:
                    f.write(f'msgctxt "{item["ctxt"]}"\n')
                # If there is already a translation for this exact msgid under another context, reuse it!
                reused_str = id_to_existing_translation.get(item["id"], "")
                f.write(f'msgid "{item["id"]}"\n')
                f.write(f'msgstr "{reused_str}"\n')

        print(f"Updated {po_path}")

    return 0


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in ("extract", "check", "sync"):
        print(__doc__)
        return 1

    action = sys.argv[1]
    if action == "extract":
        return 0 if extract_pot() else 1
    elif action == "check":
        return check_translations()
    elif action == "sync":
        return sync_translations()


if __name__ == "__main__":
    sys.exit(main())

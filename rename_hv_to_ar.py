"""
Rename language "Havakyrie" (hv) to "Arrow" (ar).
Run text replacements on all text files, then rename .hv/.hvc/.hvs files.
"""
import os
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))

SKIP_DIRS = {'.git', 'target', 'node_modules', '__pycache__'}
SKIP_EXTS = {'.pyc', '.pyo', '.exe', '.dll', '.so', '.lib', '.pdb',
             '.vsix', '.bak', '.lock', '.png', '.jpg', '.ico', '.gif',
             '.wasm', '.bin', '.obj', '.o', '.a', '.rlib', '.rmeta',
             '.d', '.map', '.woff', '.woff2', '.ttf'}
# Also skip this script itself to avoid self-modification issues
SKIP_FILES = {'rename_hv_to_ar.py'}

# Ordered replacements — most specific first to avoid partial matches
REPLACEMENTS = [
    # Full name variants
    ('Havakyrie', 'Arrow'),
    ('havakyrie', 'arrow'),
    ('HAVAKYRIE', 'ARROW'),
    # Struct/type names
    ('HvCallbacks', 'ArCallbacks'),
    ('HvConfig', 'ArConfig'),
    # Compound identifiers (longer before shorter)
    ('hv_config', 'ar_config'),
    ('hv-auto', 'ar-auto'),
    # scope name
    ('source.hv', 'source.ar'),
    # File extensions with dot (in string content) — .hvs before .hvc before .hv
    ('.hvs', '.ars'),
    ('.hvc', '.arc'),
    ('.hv',  '.ar'),
    # Identifier prefix/suffix
    ('hv_', 'ar_'),
    ('_hv',  '_ar'),
    # Quoted language-tag strings (standalone, without dot)
    ('"hvs"', '"ars"'),
    ('"hvc"', '"arc"'),
    ('"hv"',  '"ar"'),
    ("'hvs'", "'ars'"),
    ("'hvc'", "'arc'"),
    ("'hv'",  "'ar'"),
    # Import-tag syntax [hvc] / [hv] / [hvs]
    ('[hvc]', '[arc]'),
    ('[hvs]', '[ars]'),
    ('[hv]',  '[ar]'),
]


def is_binary(path):
    try:
        with open(path, 'rb') as f:
            chunk = f.read(8192)
        # If it contains a null byte it's likely binary
        return b'\x00' in chunk
    except Exception:
        return True


def process_file(path):
    if is_binary(path):
        return False
    try:
        with open(path, 'r', encoding='utf-8', errors='replace') as f:
            original = f.read()
    except Exception as e:
        print(f"  SKIP (read error): {path}: {e}")
        return False

    content = original
    for old, new in REPLACEMENTS:
        content = content.replace(old, new)

    if content != original:
        try:
            with open(path, 'w', encoding='utf-8', newline='') as f:
                f.write(content)
            return True
        except Exception as e:
            print(f"  ERROR writing {path}: {e}")
            return False
    return False


def walk_and_replace():
    changed = []
    for dirpath, dirnames, filenames in os.walk(ROOT):
        # Prune skip dirs in-place
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for fname in filenames:
            if fname in SKIP_FILES:
                continue
            _, ext = os.path.splitext(fname)
            if ext.lower() in SKIP_EXTS:
                continue
            full = os.path.join(dirpath, fname)
            if process_file(full):
                rel = os.path.relpath(full, ROOT)
                changed.append(rel)
    return changed


def rename_language_files():
    """Rename .hv/.hvc/.hvs files and hv_config.json / havakyrie.tmLanguage.json"""
    renamed = []
    for dirpath, dirnames, filenames in os.walk(ROOT):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for fname in filenames:
            old_path = os.path.join(dirpath, fname)
            new_fname = None

            # Extension renames
            if fname.endswith('.hvs'):
                new_fname = fname[:-4] + '.ars'
            elif fname.endswith('.hvc'):
                new_fname = fname[:-4] + '.arc'
            elif fname.endswith('.hv'):
                new_fname = fname[:-3] + '.ar'
            # Config files
            elif fname == 'hv_config.json':
                new_fname = 'ar_config.json'
            # VS Code syntax file
            elif fname == 'havakyrie.tmLanguage.json':
                new_fname = 'arrow.tmLanguage.json'
            # VS Code extension VSIX (rename the old one if present)
            elif fname == 'havakyrie-0.0.1.vsix':
                new_fname = 'arrow-0.0.1.vsix'
            elif fname == 'test-lang-0.0.1.vsix.bak':
                pass  # leave bak file as-is

            if new_fname:
                new_path = os.path.join(dirpath, new_fname)
                try:
                    os.rename(old_path, new_path)
                    rel_old = os.path.relpath(old_path, ROOT)
                    rel_new = os.path.relpath(new_path, ROOT)
                    renamed.append((rel_old, rel_new))
                except Exception as e:
                    print(f"  ERROR renaming {old_path}: {e}")
    return renamed


if __name__ == '__main__':
    print("=== Phase 1: Text replacements ===")
    changed = walk_and_replace()
    print(f"Modified {len(changed)} files:")
    for p in sorted(changed):
        print(f"  {p}")

    print()
    print("=== Phase 2: File renames ===")
    renamed = rename_language_files()
    print(f"Renamed {len(renamed)} files:")
    for old, new in sorted(renamed):
        print(f"  {old} -> {new}")

    print()
    print("Done.")

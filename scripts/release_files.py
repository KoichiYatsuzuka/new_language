# release_files.py — release.ps1 が使う、ファイルの中身を書き換える処理（版の番号・配布用の ar_config.json・コミットメッセージ）。
#
# PowerShell 5.1 で読み書きすると日本語を含むファイルが cp932 として壊れるので、書き換えはこちらで行う
# （BOM と改行コードはそのまま残す）。
#
#   python -X utf8 scripts/release_files.py bump    <repo> <old> <new>
#   python -X utf8 scripts/release_files.py config  <repo_config> <deployed_config|-> <out>
#   python -X utf8 scripts/release_files.py message <out> <prev> <new> <deploy_dir> [<trailer>]

import json
import re
import sys
from pathlib import Path

BOM = b"\xef\xbb\xbf"

# 版の番号が書かれている場所。（ファイル, 正規表現（版の番号は {v}）, 一致すべき数）。
# 一致の数が違えば書き換えずに止める（どこかの形が変わったのに黙って一部だけ上げない）。
VERSION_SITES = [
    ("Cargo.toml", r'(name = "arrow"\r?\nversion = "){v}(")', 1),
    ("crates/arrow-frontend/Cargo.toml", r'(name = "arrow-frontend"\r?\nversion = "){v}(")', 1),
    ("vscode-extension/package.json", r'(?m)(^  "version": "){v}(",)', 1),
    ("vscode-extension/package-lock.json", r'(?m)(^  "version": "){v}(",)', 1),
    ("vscode-extension/make-vsix.ps1", r'(arrow-){v}(\.vsix)', 1),
    ("vscode-extension/make-vsix.ps1", r'(Id="arrow" Version="){v}(")', 1),
]

# 配布先の ar_config.json で、配布先の機械に合わせて書かれた値（リポジトリの値で上書きしない）。
# 配布先に無ければ配布用の設定からも消す（例: `bridge_root` はリポジトリでは `vscode-extension`
# （node_modules の場所）だが、配布先にその場所は無く、既定（設定ファイルの場所）が正しい）。
DEPLOY_LOCAL = [("javascript", "node_path"), ("javascript", "bridge_root")]
# テストだけが使う節（ar_config::require_test_resource）。配布先には要らない。
DROP_SECTIONS = ["test"]


def read_text(path: Path):
    data = path.read_bytes()
    bom = data.startswith(BOM)
    return (data[3:] if bom else data).decode("utf-8"), bom


def write_text(path: Path, text: str, bom: bool):
    data = text.encode("utf-8")
    path.write_bytes((BOM + data) if bom else data)


def bump(repo: str, old: str, new: str):
    root = Path(repo)
    pending = {}
    for rel, pattern, expected in VERSION_SITES:
        path = root / rel
        text, bom = pending.get(rel) or read_text(path)
        regex = re.compile(pattern.format(v=re.escape(old)))
        text, n = regex.subn(lambda m: m.group(1) + new + m.group(2), text)
        if n != expected:
            sys.exit(f"bump: {rel}: expected {expected} match(es) of {pattern.format(v=old)!r}, found {n} (nothing written)")
        pending[rel] = (text, bom)
    for rel, (text, bom) in pending.items():
        write_text(root / rel, text, bom)
        print(f"  {rel}: {old} -> {new}")


def config(repo_config: str, deployed: str, out: str):
    new = json.loads(read_text(Path(repo_config))[0])
    old = json.loads(read_text(Path(deployed))[0]) if deployed != "-" else {}
    for sec in DROP_SECTIONS:
        if new.pop(sec, None) is not None:
            print(f"  dropped section '{sec}' (tests only)")
    for sec, key in DEPLOY_LOCAL:
        if sec not in new:
            continue
        if key in old.get(sec, {}):
            if new[sec].get(key) != old[sec][key]:
                print(f"  kept deployed {sec}.{key} = {json.dumps(old[sec][key], ensure_ascii=False)}"
                      f" (repo: {json.dumps(new[sec].get(key), ensure_ascii=False)})")
            new[sec][key] = old[sec][key]
        elif key in new[sec]:
            print(f"  removed {sec}.{key} (not set in the deployed config; repo: {json.dumps(new[sec][key], ensure_ascii=False)})")
            del new[sec][key]
    write_text(Path(out), json.dumps(new, indent=2, ensure_ascii=False) + "\n", False)


def message(out: str, prev: str, new: str, deploy_dir: str, trailer: str = ""):
    text = (
        f"バージョンを {new} にする（arrow・arrow-frontend・VS Code 拡張）\n"
        f"\n"
        f"配布先（{deploy_dir}）の {prev} の exe・ar_config.json・VSIX を old\\{prev} へ退避し、この版を {new} として\n"
        f"配置するため。VSIX の名前も arrow-{new}.vsix に変わる（make-vsix.ps1）。\n"
    )
    if trailer:
        text += f"\n{trailer}\n"
    write_text(Path(out), text, False)


if __name__ == "__main__":
    cmd, args = sys.argv[1], sys.argv[2:]
    {"bump": bump, "config": config, "message": message}[cmd](*args)

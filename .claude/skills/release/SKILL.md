---
name: release
description: Use when the user asks to raise Arrow's version (e.g. "v0.4.1 にあげて"), tag a release, rebuild the release arrow.exe / VSIX and copy them (with ar_config.json) to the deploy folder (../Arrow), or move the previous version's files to ../Arrow/old/<version>/. Gives the one-command procedure (scripts/release.ps1), what each step touches, the naming rules, and the traps (deploy-specific ar_config.json values, the VSIX that never builds byte-identically, re-running after a failure).
---

# Release — 版を上げ、タグを付け、配布先へ置く

`./scripts/release.ps1` が 1 回で全部やる。**PowerShell ツールから**リポジトリのルートで走らせる。

```powershell
./scripts/release.ps1 -Version 0.4.1 -CommitTrailer 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
# -Tag v_0_4_1（既定は v_<x>_<y>_<z>）  -DeployDir ..\Arrow（既定）  -PrevVersion 0.4.0（既定は配布先の arrow-*.vsix から）
```

⚠ `-CommitTrailer` には、その会話で指示された署名行（system-reminder の attribution）を渡す。

## 手順（スクリプトがこの順にやる）

| # | 段階 | 内容 |
|---|---|---|
| 0 | 前提の確認 | 作業ツリーがきれい（未コミットの変更があれば止まる）・配布先に `arrow.exe` / `arrow-<前の版>.vsix` / `ar_config.json` がある・`old/<前の版>/` が空・タグが無いか HEAD にある |
| 1 | 版を上げる | `scripts/release_files.py bump`: `Cargo.toml`・`crates/arrow-frontend/Cargo.toml`・`vscode-extension/package.json`・`package-lock.json`・`make-vsix.ps1`（VSIX のファイル名と manifest の `Version`）。**どこか 1 か所でも一致の数が違えば、何も書かずに止まる** |
| 2 | リリースビルド | `cargo build --release`（`target/release/arrow.exe`）と `make-vsix.ps1`（tsc・wasm の再ビルド込み・`arrow-<版>.vsix`）。どちらも時間制限つき（`-TimeoutSec`・既定 1 時間・超えたらプロセスの木ごと kill） |
| 3 | コミット | 版上げ・新しい VSIX・`out/` の追跡済み JS を今のブランチにコミット（古い VSIX は `git rm`）。**push しない** |
| 4 | タグ | そのコミットに注釈つきタグ（`git tag -a <Tag> -m "Arrow <版>"`） |
| 5 | 退避と配置 | 配布先の `arrow.exe`・`arrow-<前の版>.vsix`・`ar_config.json` を `old/<前の版>/` へ**移動**し、新しい exe・VSIX・配布用の `ar_config.json` を置く。`bridge/js_bridge.cjs` が変わっていればそれも差し替える（古いものは `old/<前の版>/bridge/`） |
| 6 | 動作確認 | 配布先の exe で 1 行のスクリプトを実行（`3` が出るか） |

## 名前の決まり

- **タグ**: `v_<x>_<y>_<z>`（0.4.1 から。2026-10-31 にユーザーの指定）。それより前のタグは `v0_1_0`〜`v0_4_0`
  （区切りの `_` が無い）。違う名前を指示されたら `-Tag` で渡す。
- **退避先**: `<DeployDir>/old/<x>.<y>.<z>/`（0.4.0 から。それより前は `old/v0.2.0`・`old/v0.3.0`）。
- **VSIX**: `arrow-<x>.<y>.<z>.vsix`（`make-vsix.ps1` が作る。リポジトリには最新の 1 つだけを追跡する）。

## 落とし穴

- ⚠ **配布先の `ar_config.json` はリポジトリのものと違う**。そのまま写すと配布先の JS ブリッジが動かなくなる。
  `release_files.py config` はリポジトリの設定を元に、次の 3 点を変えて配布用の設定を作る。
  - `javascript.node_path` は配布先の値を残す。配布先は同梱の Node 26 の絶対パスで、リポジトリの値は PATH 上の `node`。
  - `javascript.bridge_root` は配布先に無いので消す。リポジトリの値 `vscode-extension` は配布先に無い場所で、
    既定（設定ファイルのある場所）が正しい。
  - `test` 節は消す。テストの資源の場所で、配布先では要らない。

  配布先に固有の値が増えたら、`release_files.py` の `DEPLOY_LOCAL` に足す。
  何を残し何を消したかは実行時に表示される。
- ⚠ **VSIX は作るたびにバイト列が変わる**（zip の時刻）。版を上げずに `make-vsix.ps1` を走らせると、
  追跡中の VSIX に差分が出る。スクリプトが VSIX を作るのは版を上げたときだけ。
- ⚠ **失敗した後の再実行**: 版がもう上がっていて、その VSIX がコミット済みなら、1・3 段を飛ばす。
  タグがもう HEAD にあれば、4 段を飛ばす。どちらでもない中途半端な状態（版は上がったが未コミットなど）なら、
  作業ツリーがきれいでないので 0 段で止まる。直してからコミットし、再実行する。
- ⚠ **退避先は上書きしない**。`old/<前の版>/` にファイルがあれば止まる（以前の退避を消さないため）。
  配置の途中（移動の後）で失敗したら、`old/<前の版>/` から手で戻すか、足りないものを手で写す。
- ⚠ PowerShell 5.1 で日本語のファイルを読み書きすると壊れる。版の書き換え・設定・コミットメッセージは、
  すべて `release_files.py`（UTF-8・BOM と改行を保つ）でやる。`release.ps1` は ASCII だけで書く。
- ⚠ 配布先の exe が実行中だと、移動に失敗して止まる（VS Code の REPL 端末などを閉じる）。

## 終わったら報告すること

- 新しいコミットとタグ（`git log --oneline -1`・`git tag --points-at HEAD`）。
- 配布先と退避先の中身（`Get-ChildItem <DeployDir>, <DeployDir>\old\<前の版>`）。
- `ar_config.json` で残した値・消した値（スクリプトの表示）。
- push していないこと。

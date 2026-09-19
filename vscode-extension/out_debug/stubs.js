"use strict";
/**
 * stubs.ts — 外部モジュール（C++ / C# / Rust / 他の `.ar`）の型スタブをフロントエンドへ供給する。
 *
 * # 役割分担
 *
 * 解析器（wasm）は fs に触れない。触れないからこそ 1 打鍵ごとに走らせられる。
 * その代わり import 先のメンバ型が分からないので、**fs を持っているこちら側が読んで渡す**。
 *
 * ⚠ **ここは「判断をしない層」**。DLL / ヘッダの探索順（`source_dir` → `root_dir` →
 * `ar_config.json`）は言語側の規則で、`arrow.exe --emit-stubs` が解決してマニフェストに
 * 書く。このファイルがやるのは**マニフェストを読んで運ぶこと**だけで、鍵も組み立てない。
 * ここに探索規則を書いた瞬間に「拡張だけ解釈がずれる」という、`analysis.ts` を捨てた
 * ときの問題がそのまま戻る。
 *
 * # ⚠ 自動では再生成しない
 *
 * `--emit-stubs` は import 先を**実際に解決する**（.NET メタデータを読み、Python を
 * 起動する）ので秒オーダーになりうる。打鍵ごとに走らせる類のものではないし、
 * 外部プロセスを黙って起動するのも避けたい。⇒ 再生成は明示コマンド
 * （`Arrow: Refresh External Stubs`）のときだけ。マニフェストが無ければ、
 * 従来どおり「型が付かないだけ」で動く。
 */
Object.defineProperty(exports, "__esModule", { value: true });
exports.refreshStubs = exports.loadedManifestPath = exports.loadStubsFor = void 0;
const fs = require("fs");
const path = require("path");
const child_process_1 = require("child_process");
const vscode = require("vscode");
const frontend_1 = require("./frontend");
/** `--emit-stubs` が書き出すディレクトリ名（`src/stub_manifest.rs` と一致させること）。 */
const STUB_DIR = '.arrow-stubs';
const MANIFEST_NAME = 'manifest.json';
/** 現在 wasm に積んであるマニフェスト（パスと mtime）。同じものを積み直さないための記録。 */
let loaded = null;
/**
 * `document` に対応するマニフェストを探す。
 *
 * ドキュメントのあるディレクトリから上へ辿る。`--emit-stubs` は**エントリスクリプトの
 * 隣**に書くので、同じプロジェクトの下位ディレクトリにあるファイルを開いたときも
 * 拾えるようにするため。
 */
function findManifest(documentPath) {
    let dir = path.dirname(documentPath);
    for (;;) {
        const p = path.join(dir, STUB_DIR, MANIFEST_NAME);
        if (fs.existsSync(p))
            return p;
        // ⚠ **プロジェクトの外へ出ない。** 無条件に上まで辿ると、親ディレクトリに
        //    たまたま置かれた無関係な `.arrow-stubs/` を拾い、**別プロジェクトの型**が
        //    出てしまう。`ar_config.json`（Arrow のプロジェクト設定）か `.git` を境界にする。
        if (fs.existsSync(path.join(dir, 'ar_config.json')) ||
            fs.existsSync(path.join(dir, '.git'))) {
            return null;
        }
        const parent = path.dirname(dir);
        if (parent === dir)
            return null;
        dir = parent;
    }
}
/**
 * マニフェストを読んで wasm のスタブ表を**丸ごと入れ替える**。
 *
 * ⚠ **入れ替えたら解析キャッシュを捨てること。** キャッシュは `document.version` だけを
 *    鍵にしているので、テキストが変わっていない限り古い解析結果を返し続ける。
 *    呼び出し側（`extension.ts`）が `onStubsChanged` で対にしてある。
 *
 * @returns 積んだ件数。マニフェストが無ければ `null`（＝従来どおり型が付かないだけ）。
 */
function loadStubsFor(documentPath) {
    var _a;
    const manifestPath = findManifest(documentPath);
    if (!manifestPath) {
        // 前に別のプロジェクトのスタブを積んでいたなら捨てる。残すと**無関係な型**が出る。
        if (loaded) {
            (0, frontend_1.clearStubs)();
            loaded = null;
        }
        return null;
    }
    let mtimeMs = 0;
    try {
        mtimeMs = fs.statSync(manifestPath).mtimeMs;
    }
    catch {
        return null;
    }
    if (loaded && loaded.manifestPath === manifestPath && loaded.mtimeMs === mtimeMs) {
        return (0, frontend_1.stubCount)(); // 同じものが既に載っている
    }
    let entries;
    try {
        const raw = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
        entries = (_a = raw.stubs) !== null && _a !== void 0 ? _a : [];
    }
    catch {
        // 壊れたマニフェストで言語機能ごと止めない。型が付かないだけに倒す。
        return null;
    }
    const dir = path.dirname(manifestPath);
    (0, frontend_1.clearStubs)();
    for (const e of entries) {
        try {
            (0, frontend_1.setStub)(e.key, fs.readFileSync(path.join(dir, e.file), 'utf8'));
        }
        catch {
            // 1 件読めなくても残りは積む。
        }
    }
    loaded = { manifestPath, mtimeMs };
    return (0, frontend_1.stubCount)();
}
exports.loadStubsFor = loadStubsFor;
/** いま積んでいるマニフェストのパス（無ければ null）。ステータス表示用。 */
function loadedManifestPath() {
    var _a;
    return (_a = loaded === null || loaded === void 0 ? void 0 : loaded.manifestPath) !== null && _a !== void 0 ? _a : null;
}
exports.loadedManifestPath = loadedManifestPath;
/**
 * `arrow.exe --emit-stubs <file>` を起動してスタブを作り直す。
 *
 * ⚠ **明示コマンドからのみ呼ぶこと**（冒頭 doc）。
 * ⚠ 実行ファイルの場所は設定 `arrow.executablePath`。未設定なら何もしない
 *    （黙って PATH を探しに行かない — どの `arrow.exe` が動くか利用者に分からなくなる）。
 */
function refreshStubs(documentPath) {
    var _a;
    const exe = (_a = vscode.workspace.getConfiguration('arrow').get('executablePath')) === null || _a === void 0 ? void 0 : _a.trim();
    if (!exe) {
        return Promise.resolve({
            ok: false,
            message: 'set `arrow.executablePath` to your arrow.exe first',
        });
    }
    return new Promise(resolve => {
        const child = (0, child_process_1.spawn)(exe, ['--emit-stubs', documentPath], {
            cwd: path.dirname(documentPath),
        });
        let out = '';
        child.stdout.on('data', d => { out += String(d); });
        child.stderr.on('data', d => { out += String(d); });
        child.on('error', e => resolve({ ok: false, message: String(e) }));
        child.on('close', code => {
            if (code !== 0) {
                resolve({ ok: false, message: out.trim() || `exit ${code}` });
                return;
            }
            // 作り直したので、次の読み込みで必ず積み直させる。
            loaded = null;
            resolve({ ok: true, message: out.trim() });
        });
    });
}
exports.refreshStubs = refreshStubs;
//# sourceMappingURL=stubs.js.map
"use strict";
/**
 * frontend.ts — Arrow の実装そのものを解析器として使うためのブリッジ。
 *
 * `crates/arrow-frontend` を wasm32 にビルドしたものを読み込む。中身は
 * `cargo run` が使うのと**同一のソース**（src/lexer, src/parser, src/type_check）で、
 * TypeScript 側には言語仕様の判断が一切無い。従来 analysis.ts / type_infer.ts が
 * 正規表現で近似していた部分を、本物のパーサと型検査器で置き換えるのが目的。
 *
 * 依存は wasm ファイル 1 個だけ。VSIX に同梱され、外部プロセスも Rust ツールチェーンも
 * 実行時には要らない。
 */
Object.defineProperty(exports, "__esModule", { value: true });
exports.stubCount = exports.clearStubs = exports.setStub = exports.analyze = exports.frontendLoadError = exports.isFrontendReady = exports.loadFrontend = void 0;
const fs = require("fs");
const path = require("path");
let exports_ = null;
let loadError = null;
/**
 * wasm を読み込む。1 度だけ実行され、以降はキャッシュされたインスタンスを使う。
 *
 * `WebAssembly.instantiate` ではなく同期版の `Module` + `Instance` を使う。
 * 425 KB のコンパイルは実測 1〜2 ms で、activate を非同期にする価値が無いため。
 *
 * @param extensionPath 拡張のルートディレクトリ（`context.extensionPath`）
 * @returns 読み込みに成功したか
 */
function loadFrontend(extensionPath) {
    if (exports_)
        return true;
    if (loadError)
        return false;
    const candidates = [
        // 配布形態: VSIX に同梱されたもの。
        path.join(extensionPath, 'out', 'arrow_frontend.wasm'),
        // 開発時: リポジトリの cargo 出力を直接読む。これがあるおかげで、
        // VSIX を作り直さなくても run_debug.js で最新のフロントエンドを試せる。
        path.join(extensionPath, '..', 'crates', 'arrow-frontend', 'target', 'wasm32-unknown-unknown', 'release', 'arrow_frontend.wasm'),
    ];
    try {
        const wasmPath = candidates.find(p => fs.existsSync(p));
        if (!wasmPath) {
            loadError = `arrow_frontend.wasm not found (looked in: ${candidates.join(', ')})`;
            return false;
        }
        const bytes = fs.readFileSync(wasmPath);
        const module = new WebAssembly.Module(bytes);
        const instance = new WebAssembly.Instance(module, {});
        exports_ = instance.exports;
        return true;
    }
    catch (e) {
        loadError = String(e);
        return false;
    }
}
exports.loadFrontend = loadFrontend;
/** wasm が使える状態か。false のときは `frontendLoadError()` に理由が入る。 */
function isFrontendReady() {
    return exports_ !== null;
}
exports.isFrontendReady = isFrontendReady;
/** 読み込みに失敗した理由（成功していれば null）。 */
function frontendLoadError() {
    return loadError;
}
exports.frontendLoadError = frontendLoadError;
/**
 * ソース 1 本を解析する。
 *
 * ⚠ `memory.buffer` は `ar_alloc` / `ar_analyze` がメモリを拡張すると
 *    **差し替わる**（古い ArrayBuffer は detached になる）。そのため
 *    TypedArray は wasm を呼ぶ**たびに作り直す**こと。使い回すと、大きめの
 *    ファイルで確保が伸びた瞬間に空の結果や例外になる。
 *
 * @returns 解析結果。wasm が使えない場合は null。
 */
function analyze(source) {
    const ex = exports_;
    if (!ex)
        return null;
    const bytes = new TextEncoder().encode(source);
    const ptr = ex.ar_alloc(bytes.length);
    if (ptr === 0 && bytes.length > 0)
        return null;
    try {
        // alloc 後に buffer を取り直す（拡張されている可能性がある）。
        new Uint8Array(ex.memory.buffer, ptr, bytes.length).set(bytes);
        ex.ar_analyze(ptr, bytes.length);
        // analyze 後にも取り直す（解析中に確保が伸びている）。
        const out = new Uint8Array(ex.memory.buffer, ex.ar_result_ptr(), ex.ar_result_len());
        return JSON.parse(new TextDecoder().decode(out));
    }
    catch {
        return null;
    }
    finally {
        ex.ar_free(ptr, bytes.length);
    }
}
exports.analyze = analyze;
/**
 * wasm へ UTF-8 文字列を書き込み、`(ptr, len)` で `f` に渡す。
 *
 * ⚠ `memory.buffer` は `ar_alloc` がメモリを拡張すると差し替わるので、
 *    TypedArray は alloc の**後**に作ること（`analyze` と同じ注意）。
 */
function withUtf8(ex, s, f) {
    const bytes = new TextEncoder().encode(s);
    const ptr = ex.ar_alloc(bytes.length);
    if (ptr === 0 && bytes.length > 0)
        return null;
    try {
        new Uint8Array(ex.memory.buffer, ptr, bytes.length).set(bytes);
        return f(ptr, bytes.length);
    }
    finally {
        ex.ar_free(ptr, bytes.length);
    }
}
/**
 * 型スタブを 1 件登録する。
 *
 * ⚠ **これを呼ぶと `analyze()` の結果がスタブの有無に依存する。** 呼び出し側は
 *    ドキュメントを切り替えるたびに [`clearStubs`] してから積み直し、**同時に
 *    解析キャッシュも捨てる**こと（捨てないと「スタブを更新したのに古い型が出続ける」）。
 *
 * @param key `arrow.exe --emit-stubs` が出したマニフェストの `key` をそのまま渡す。
 *            拡張側で組み立てない（探索規則を TS に持ち込まないため）。
 */
function setStub(key, source) {
    var _a;
    const ex = exports_;
    if (!ex)
        return false;
    // 2 本の文字列を同時に渡すので、外側を確保したまま内側を確保する。
    // ⚠ 内側の alloc でメモリが伸びると外側の ptr が指す ArrayBuffer は detach するが、
    //    **ptr（数値）自体は有効**なので、書き込み側でだけ取り直せばよい。
    return (_a = withUtf8(ex, key, (kp, kl) => {
        var _a;
        return (_a = withUtf8(ex, source, (sp, sl) => {
            ex.ar_set_stub(kp, kl, sp, sl);
            return true;
        })) !== null && _a !== void 0 ? _a : false;
    })) !== null && _a !== void 0 ? _a : false;
}
exports.setStub = setStub;
/** 登録済みのスタブをすべて捨てる。 */
function clearStubs() {
    exports_ === null || exports_ === void 0 ? void 0 : exports_.ar_clear_stubs();
}
exports.clearStubs = clearStubs;
/** 登録済みスタブの件数（配線確認・ログ用）。 */
function stubCount() {
    var _a;
    return (_a = exports_ === null || exports_ === void 0 ? void 0 : exports_.ar_stub_count()) !== null && _a !== void 0 ? _a : 0;
}
exports.stubCount = stubCount;
//# sourceMappingURL=frontend.js.map
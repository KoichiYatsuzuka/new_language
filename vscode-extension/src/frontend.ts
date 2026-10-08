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

import * as fs from 'fs';
import * as path from 'path';
import { hostImports, toWasmPath } from './wasm_host';

// `WebAssembly` の型は `lib: ["ES2020"]` には含まれず、`"DOM"` を足すと Node に
// 存在しないブラウザ API まで型が通ってしまう。実際に使う 3 つだけをここで宣言する。
interface WasmMemory { buffer: ArrayBuffer }
interface WasmInstance { exports: Record<string, unknown> }
declare const WebAssembly: {
    Module: new (bytes: Uint8Array) => object;
    Instance: new (module: object, imports?: object) => WasmInstance;
};

/** wasm が公開する C ABI。詳細は crates/arrow-frontend/src/wasm.rs を参照。 */
interface FrontendExports {
    memory: WasmMemory;
    ar_alloc(len: number): number;
    ar_free(ptr: number, len: number): void;
    ar_analyze(ptr: number, len: number): number;
    ar_analyze_at(pathPtr: number, pathLen: number, srcPtr: number, srcLen: number): number;
    ar_result_ptr(): number;
    ar_result_len(): number;
    ar_set_stub(keyPtr: number, keyLen: number, srcPtr: number, srcLen: number): void;
    ar_clear_stubs(): void;
    ar_stub_count(): number;
    ar_builtins(): number;
}

/** 診断 1 件。`at` は 0 始まりの行・列で、位置不明のときは null。 */
export interface WasmDiagnostic {
    /** 0 = error, 1 = warning（VS Code の DiagnosticSeverity に対応）。 */
    severity: number;
    message: string;
    /** エラー種別名（`StaticTypeError` など）。 */
    source: string;
    at: { line: number; col: number } | null;
}

export interface AnalysisResult {
    /** 構文解析が成功したか。false のとき diagnostics は空。 */
    ok: boolean;
    parseError: string | null;
    diagnostics: WasmDiagnostic[];
    stmtCount?: number;
}

let exports_: FrontendExports | null = null;
let loadError: string | null = null;

/**
 * wasm を読み込む。1 度だけ実行され、以降はキャッシュされたインスタンスを使う。
 *
 * `WebAssembly.instantiate` ではなく同期版の `Module` + `Instance` を使う。
 * 425 KB のコンパイルは実測 1〜2 ms で、activate を非同期にする価値が無いため。
 *
 * wasm には import 先を読むためのホスト関数（`wasm_host.ts`）を渡す。
 *
 * @param extensionPath 拡張のルートディレクトリ（`context.extensionPath`）
 * @param wasmPath 読む wasm を直接指定する（`compare_wasm_frontend.ps1` が使う）。省略時は探す
 * @returns 読み込みに成功したか
 */
export function loadFrontend(extensionPath: string, wasmPath?: string): boolean {
    if (exports_) return true;
    if (loadError) return false;
    const candidates = wasmPath ? [wasmPath] : [
        // 配布形態: VSIX に同梱されたもの。
        path.join(extensionPath, 'out', 'arrow_frontend.wasm'),
        // 開発時: リポジトリの cargo 出力を直接読む。これがあるおかげで、
        // VSIX を作り直さなくても run_debug.js で最新のフロントエンドを試せる。
        path.join(extensionPath, '..', 'crates', 'arrow-frontend',
                  'target', 'wasm32-unknown-unknown', 'release', 'arrow_frontend.wasm'),
    ];
    try {
        const found = candidates.find(p => fs.existsSync(p));
        if (!found) {
            loadError = `arrow_frontend.wasm not found (looked in: ${candidates.join(', ')})`;
            return false;
        }
        const bytes = fs.readFileSync(found);
        const module = new WebAssembly.Module(bytes);
        // ⚠ ホスト関数はメモリを**呼ばれるたびに**取り直す（伸びると差し替わる）。
        const instance = new WebAssembly.Instance(module, {
            arrow_host: hostImports(() => (exports_ as FrontendExports).memory),
        });
        exports_ = instance.exports as unknown as FrontendExports;
        return true;
    } catch (e) {
        loadError = String(e);
        return false;
    }
}

/** wasm が使える状態か。false のときは `frontendLoadError()` に理由が入る。 */
export function isFrontendReady(): boolean {
    return exports_ !== null;
}

/** 読み込みに失敗した理由（成功していれば null）。 */
export function frontendLoadError(): string | null {
    return loadError;
}

/**
 * ソース 1 本を解析する。
 *
 * ⚠ `memory.buffer` は `ar_alloc` / `ar_analyze` がメモリを拡張すると
 *    **差し替わる**（古い ArrayBuffer は detached になる）。そのため
 *    TypedArray は wasm を呼ぶ**たびに作り直す**こと。使い回すと、大きめの
 *    ファイルで確保が伸びた瞬間に空の結果や例外になる。
 *
 * @param fsPath ドキュメントのファイルのパス（ホストの形）。import はここから CLI と同じ規則で
 *               探す。保存されていないドキュメントなどファイルが無いときは省略する
 * @returns 解析結果。wasm が使えない場合は null。
 */
export function analyze(source: string, fsPath?: string): AnalysisResult | null {
    const ex = exports_;
    if (!ex) return null;

    try {
        const run = (sp: number, sl: number): void => {
            if (fsPath === undefined) {
                ex.ar_analyze(sp, sl);
            } else {
                withUtf8(ex, toWasmPath(fsPath), (pp, pl) => ex.ar_analyze_at(pp, pl, sp, sl));
            }
        };
        if (withUtf8(ex, source, (sp, sl) => { run(sp, sl); return true; }) === null) return null;
        // analyze 後に buffer を取り直す（解析中に確保が伸びている）。
        const out = new Uint8Array(ex.memory.buffer, ex.ar_result_ptr(), ex.ar_result_len());
        return JSON.parse(new TextDecoder().decode(out)) as AnalysisResult;
    } catch {
        return null;
    }
}

/**
 * 組み込み関数の宣言（`builtins.ars`）の本文。wasm が埋め込んでいるものを受け取る。
 *
 * 型検査器が使っているのと**同じ本文**なので、hover・補完に出る組み込みと型検査が食い違わない
 * （以前は拡張が自前のファイルを読んでいて、実行時に無い関数まで並んでいた）。
 *
 * @returns 本文。wasm が使えない場合は null。
 */
export function builtinsSource(): string | null {
    const ex = exports_;
    if (!ex) return null;
    const len = ex.ar_builtins();
    // 呼んだ後に buffer を取り直す（`analyze` と同じ注意）。
    const out = new Uint8Array(ex.memory.buffer, ex.ar_result_ptr(), len);
    return new TextDecoder().decode(out);
}

/**
 * wasm へ UTF-8 文字列を書き込み、`(ptr, len)` で `f` に渡す。
 *
 * ⚠ `memory.buffer` は `ar_alloc` がメモリを拡張すると差し替わるので、
 *    TypedArray は alloc の**後**に作ること（`analyze` と同じ注意）。
 */
function withUtf8<T>(ex: FrontendExports, s: string, f: (ptr: number, len: number) => T): T | null {
    const bytes = new TextEncoder().encode(s);
    const ptr = ex.ar_alloc(bytes.length);
    if (ptr === 0 && bytes.length > 0) return null;
    try {
        new Uint8Array(ex.memory.buffer, ptr, bytes.length).set(bytes);
        return f(ptr, bytes.length);
    } finally {
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
export function setStub(key: string, source: string): boolean {
    const ex = exports_;
    if (!ex) return false;
    // 2 本の文字列を同時に渡すので、外側を確保したまま内側を確保する。
    // ⚠ 内側の alloc でメモリが伸びると外側の ptr が指す ArrayBuffer は detach するが、
    //    **ptr（数値）自体は有効**なので、書き込み側でだけ取り直せばよい。
    return withUtf8(ex, key, (kp, kl) =>
        withUtf8(ex, source, (sp, sl) => {
            ex.ar_set_stub(kp, kl, sp, sl);
            return true;
        }) ?? false,
    ) ?? false;
}

/** 登録済みのスタブをすべて捨てる。 */
export function clearStubs(): void {
    exports_?.ar_clear_stubs();
}

/** 登録済みスタブの件数（配線確認・ログ用）。 */
export function stubCount(): number {
    return exports_?.ar_stub_count() ?? 0;
}

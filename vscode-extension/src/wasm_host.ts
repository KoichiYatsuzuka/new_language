/**
 * wasm_host.ts — wasm（`crates/arrow-frontend`）が import 先を読むためのホスト関数。
 *
 * import の処理（探索の規則・読み込み・構文解析・型）はすべて Rust 側（`src/parser/imports/`）にあり、
 * CLI と**同じコード**が wasm の中で動く。wasm はファイルも環境変数も持たないので、外界に触る所だけを
 * ここに頼む（Rust 側の窓口は `src/import_fs.rs` の wasm 版）。ここは「読む」「在るか見る」だけを受け持ち、
 * **判断をしない**（どこを探すか・何を読むかを TypeScript で決めると、拡張だけ解釈がずれる）。
 *
 * # 受け渡しの手順
 *
 * 値を返す関数（`host_read` など）は結果を**保留**に置いて長さを返す（無ければ負）。wasm はその長さの
 * 領域を確保してから `host_take(ptr)` で写させる。wasm の関数をホストから呼び返さずに済む
 * （import の中から export を呼ぶと、確保でメモリが伸びたときの扱いが込み入る）。
 *
 * # パスの形
 *
 * wasm の `std::path` は Unix の規則（区切りは `/`・`/` で始まれば絶対パス）。Windows のパス
 * `D:\a\b.ar` はそのままでは親も絶対かどうかも取れないので、境界では `/D:/a/b.ar` の形にする
 * （VS Code の URI と同じ形）。ホストでファイルに触るときに元へ戻す。
 * ⚠ wasm に渡すパスは**すべて** `toWasmPath` を通すこと（解析するドキュメントのパス・`realpath`・
 *   カレントディレクトリ・環境変数のパス）。1 つでも漏れると、そこから先の探索が黙って外れる。
 *
 * ⚠ `vscode` に依存しないこと。`crates/arrow-frontend/dump_diags.js`（`compare_wasm_frontend.ps1`）が
 *   拡張の外から読み込み、**拡張と同じホスト**で wasm を動かして CLI と突き合わせる。
 */

import * as child_process from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

interface HostMemory { buffer: ArrayBuffer }

const isWindows = process.platform === 'win32';

/** ホストのパス → wasm に渡すパス（Windows では `D:\a\b` → `/D:/a/b`）。 */
export function toWasmPath(p: string): string {
    if (!isWindows) return p;
    const s = p.replace(/\\/g, '/');
    return /^[A-Za-z]:(\/|$)/.test(s) ? '/' + s : s;
}

/**
 * wasm から来たパス → ホストのパス（`/D:/a/b` → `D:/a/b`・`/D:` → `D:/`。Node は `/` 区切りも受ける）。
 *
 * ⚠ Windows では、ドライブの無い `/...` は wasm にしか無い場所（ドライブ `/D:` の上の `/`）なので
 *   `null`（無い）を返す。親をたどって設定ファイルを探すループがそこを覗いても、何も見つからない
 *   （CLI はドライブのルートで止まる）。`//server/share`（UNC）はそのまま。
 */
export function toHostPath(p: string): string | null {
    if (!isWindows) return p;
    if (/^\/[A-Za-z]:$/.test(p)) return p.slice(1) + '/';
    if (/^\/[A-Za-z]:\//.test(p)) return p.slice(1);
    if (p.startsWith('/') && !p.startsWith('//')) return null;
    return p;
}

/**
 * wasm が出した文面の中のパス（`/D:/a/b`・`/D:`）をホストの形（`D:/a/b`・`D:/`）に直す（表示用）。
 *
 * import の誤り（探した場所の一覧など）はパスを含む。wasm の形のまま見せると利用者には見慣れない。
 */
export function displayPaths(text: string): string {
    if (!isWindows) return text;
    return text.replace(/(^|[\s'"(])\/([A-Za-z]):(?:\/|(?=['"\s),]|$))/g, '$1$2:/');
}

/** ホストのパスが無い（wasm にしか無い場所）ときに投げる。 */
function hostPath(p: string): string {
    const h = toHostPath(p);
    if (h === null) throw Object.assign(new Error(`no such path: ${p}`), { code: 'ENOENT' });
    return h;
}

/** `host_read` などが失敗したときの戻り値（wasm 側の `io::ErrorKind` に写す）。 */
const NOT_FOUND = -1;
const OTHER_ERROR = -2;

function errorCode(e: unknown): number {
    const code = (e as { code?: string })?.code;
    return code === 'ENOENT' || code === 'ENOTDIR' ? NOT_FOUND : OTHER_ERROR;
}

/** 次の `host_take` で wasm へ写す値。 */
let pending: Uint8Array | null = null;

/** カレントディレクトリとして wasm に見せるディレクトリ（ホストの形）。null なら `process.cwd()`。 */
let cwd: string | null = null;

/**
 * wasm に見せるカレントディレクトリを決める。
 *
 * CLI はカレントディレクトリでも設定ファイル（`ar_config.json`）を探す。拡張では
 * **利用者が `arrow` を走らせる場所**に当たるワークスペースのフォルダを渡す
 * （拡張のプロセスのカレントディレクトリは利用者のプロジェクトと無関係）。
 */
export function setHostCwd(dir: string | null): void {
    cwd = dir;
}

/** Python の標準ライブラリと site-packages の場所（1 度だけ調べる）。 */
let pythonDirs: string[] | null = null;

/**
 * Python に `sysconfig` を尋ねる。
 *
 * ⚠ CLI の `import_fs::python_lib_dirs`（`src/import_fs.rs`）と**同じ問い合わせ**（候補の実行ファイルの順・
 *   スクリプト）。片方だけ変えると、拡張と CLI で `import[py]` の探索先がずれる。
 */
function queryPythonDirs(): string[] {
    const script =
        "import sysconfig; " +
        "paths = [sysconfig.get_path('stdlib'), sysconfig.get_path('purelib')]; " +
        "print('\\n'.join(p for p in paths if p))";
    const candidates = isWindows ? ['py', 'python', 'python3'] : ['python3', 'python'];
    for (const exe of candidates) {
        try {
            const out = child_process.execFileSync(exe, ['-c', script], {
                encoding: 'utf8',
                stdio: ['ignore', 'pipe', 'ignore'],
                timeout: 10_000,
                windowsHide: true,
            });
            const dirs = out.split(/\r?\n/).map(l => l.trim()).filter(l => l.length > 0);
            if (dirs.length > 0) return dirs;
        } catch {
            // 次の候補へ（見つからない・失敗した）。
        }
    }
    return [];
}

/**
 * wasm の `arrow_host` モジュールとして渡す関数一式。
 *
 * @param memory wasm の線形メモリを返す関数。⚠ メモリは伸びると `buffer` が差し替わるので、
 *               使うたびに取り直す（保持しない）。
 */
export function hostImports(memory: () => HostMemory): Record<string, (...args: number[]) => number | void> {
    const decoder = new TextDecoder();
    const encoder = new TextEncoder();
    const str = (ptr: number, len: number): string =>
        decoder.decode(new Uint8Array(memory().buffer, ptr, len));
    const put = (bytes: Uint8Array): number => {
        pending = bytes;
        return bytes.length;
    };
    const putText = (s: string): number => put(encoder.encode(s));

    return {
        /** 0 = 無い・1 = ファイル・2 = ディレクトリ。 */
        host_stat(ptr: number, len: number): number {
            try {
                return fs.statSync(hostPath(str(ptr, len))).isDirectory() ? 2 : 1;
            } catch {
                return 0;
            }
        },
        /** ファイルの中身（バイト列）。 */
        host_read(ptr: number, len: number): number {
            try {
                return put(fs.readFileSync(hostPath(str(ptr, len))));
            } catch (e) {
                return errorCode(e);
            }
        },
        /** ディレクトリの中の項目の名前（`\n` 区切り）。 */
        host_read_dir(ptr: number, len: number): number {
            try {
                return putText(fs.readdirSync(hostPath(str(ptr, len))).join('\n'));
            } catch (e) {
                return errorCode(e);
            }
        },
        /** テキストに書かれたパス（設定ファイルの値など）を wasm の形にする（`import_fs::path_from_text`）。 */
        host_path(ptr: number, len: number): number {
            return putText(toWasmPath(str(ptr, len)));
        },
        /** 絶対パスにし、リンクを解いた形。 */
        host_realpath(ptr: number, len: number): number {
            try {
                return putText(toWasmPath(fs.realpathSync.native(hostPath(str(ptr, len)))));
            } catch (e) {
                return errorCode(e);
            }
        },
        /**
         * 環境変数をパスとして読む（`list` が 0 以外ならパスの並び・`\n` 区切りで返す）。
         * 値が無ければ負。
         */
        host_env_path(ptr: number, len: number, list: number): number {
            const value = process.env[str(ptr, len)];
            if (value === undefined) return NOT_FOUND;
            const items = list ? value.split(path.delimiter) : [value];
            return putText(items.map(toWasmPath).join('\n'));
        },
        /** カレントディレクトリ（`setHostCwd`）。 */
        host_cwd(): number {
            return putText(toWasmPath(cwd ?? process.cwd()));
        },
        /** Python の標準ライブラリと site-packages の場所（`\n` 区切り・無ければ空）。 */
        host_python_lib_dirs(): number {
            if (pythonDirs === null) pythonDirs = queryPythonDirs();
            return putText(pythonDirs.map(toWasmPath).join('\n'));
        },
        /** 保留中の値を wasm の `dst` へ写して捨てる。 */
        host_take(dst: number): void {
            const bytes = pending;
            pending = null;
            if (!bytes || bytes.length === 0) return;
            new Uint8Array(memory().buffer, dst, bytes.length).set(bytes);
        },
    };
}

"use strict";
/**
 * extension.ts — VS Code extension entry point for the Arrow language.
 *
 * Responsibilities:
 * - Initialize the extension on activation (`activate`)
 * - Load the Arrow frontend (wasm) and register all language-feature providers
 *   defined in `wasm_providers.ts`
 * - Implement the "Send to REPL" command and REPL terminal management
 * - Schedule debounced diagnostics on document open/change events
 * - Watch files that can be import targets and make the frontend re-read them
 */
Object.defineProperty(exports, "__esModule", { value: true });
exports.deactivate = exports.activate = void 0;
const vscode = require("vscode");
const path = require("path");
const wasm_providers_1 = require("./wasm_providers");
const frontend_1 = require("./frontend");
// ===== REPL terminal =====
const REPL_SENTINEL = '##REPL_EXEC##';
let replTerminal;
function getReplTerminal(projectRoot) {
    if (replTerminal && vscode.window.terminals.includes(replTerminal))
        return replTerminal;
    replTerminal = vscode.window.createTerminal({ name: 'Arrow REPL', cwd: projectRoot });
    replTerminal.sendText('cargo run -- --repl');
    return replTerminal;
}
/**
 * Walk up from `dir` until a directory containing `Cargo.toml` is found.
 * Returns `undefined` when no Cargo project root exists above the given path.
 */
function findCargoRoot(dir) {
    const { root } = path.parse(dir);
    let current = dir;
    while (true) {
        try {
            require('fs').accessSync(path.join(current, 'Cargo.toml'));
            return current;
        }
        catch {
            if (current === root)
                return undefined;
            current = path.dirname(current);
        }
    }
}
// ===== Cell helpers =====
const CELL_MARKER = /^#%%/;
/**
 * Return the text of the `#%%`-delimited cell that contains the cursor line.
 * If no `#%%` marker surrounds the cursor, the entire document is treated as one cell.
 */
function getCellAtCursor(editor) {
    const doc = editor.document;
    const cursorLine = editor.selection.active.line;
    const lineCount = doc.lineCount;
    let startLine = 0;
    for (let i = cursorLine; i >= 0; i--) {
        if (CELL_MARKER.test(doc.lineAt(i).text)) {
            startLine = i + 1; // skip the #%% marker line itself
            break;
        }
    }
    let endLine = lineCount - 1;
    for (let i = cursorLine + 1; i < lineCount; i++) {
        if (CELL_MARKER.test(doc.lineAt(i).text)) {
            endLine = i - 1;
            break;
        }
    }
    const range = new vscode.Range(startLine, 0, endLine, doc.lineAt(endLine).text.length);
    return doc.getText(range);
}
// ===== Language selectors =====
/**
 * `.ar` and `.ars` are registered as separate languages so the file explorer can
 * give them different icons; both get the full set of language features.
 * (`.arc` is a compiled binary module — icon only, no providers.)
 */
const ARROW_SELECTOR = [
    { language: 'arrow' },
    { language: 'arrow-stub' },
];
/** True for documents the language features apply to (`.ar` / `.ars`). */
function isArrowDocument(document) {
    return document.languageId === 'arrow' || document.languageId === 'arrow-stub';
}
// ===== Activation =====
function activate(context) {
    // 解析は wasm 版フロントエンド（= `cargo run` と同一のソース）が担う。
    // 読み込めない環境では言語機能を諦める。旧正規表現実装へは**戻さない**:
    // 二重実装を残すと「拡張だけ解釈がずれる」問題がそのまま生き延びるため。
    if (!(0, frontend_1.loadFrontend)(context.extensionPath)) {
        vscode.window.showErrorMessage(`Arrow: failed to load the language frontend — code intelligence is disabled. ` +
            `(${(0, frontend_1.frontendLoadError)() ?? 'unknown error'})`);
        return;
    }
    // 組み込み関数（print / len / …）も同じフロントエンドで解析して取り込む。宣言の本文は
    // wasm が持っている（型検査器と同じ `builtins.ars`）。
    // 読めなくても言語機能は動く（組み込みが候補に出なくなるだけ）。
    (0, wasm_providers_1.loadPrelude)();
    context.subscriptions.push(vscode.window.onDidCloseTerminal(t => { if (t === replTerminal)
        replTerminal = undefined; }), vscode.languages.registerInlayHintsProvider(ARROW_SELECTOR, { provideInlayHints: wasm_providers_1.provideInlayHints }), vscode.languages.registerHoverProvider(ARROW_SELECTOR, { provideHover: wasm_providers_1.provideHover }), vscode.languages.registerDocumentSemanticTokensProvider(ARROW_SELECTOR, { provideDocumentSemanticTokens: wasm_providers_1.provideDocumentSemanticTokens }, wasm_providers_1.SEMANTIC_TOKENS_LEGEND), vscode.languages.registerCompletionItemProvider(ARROW_SELECTOR, { provideCompletionItems: wasm_providers_1.provideCompletionItems }, '.'), vscode.languages.registerDocumentSymbolProvider(ARROW_SELECTOR, { provideDocumentSymbols: wasm_providers_1.provideDocumentSymbols }), vscode.languages.registerSignatureHelpProvider(ARROW_SELECTOR, { provideSignatureHelp: wasm_providers_1.provideSignatureHelp }, '(', ','), vscode.languages.registerDefinitionProvider(ARROW_SELECTOR, { provideDefinition: wasm_providers_1.provideDefinition }));
    // ---- Send-to-REPL command ----
    context.subscriptions.push(vscode.commands.registerCommand('arrow.sendToRepl', () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || !isArrowDocument(editor.document)) {
            vscode.window.showWarningMessage('Arrow REPL: open a .ar file first.');
            return;
        }
        const fileDir = path.dirname(editor.document.uri.fsPath);
        const projectRoot = findCargoRoot(fileDir);
        if (!projectRoot) {
            vscode.window.showErrorMessage('Arrow REPL: could not find Cargo.toml above this file.');
            return;
        }
        const sel = editor.selection;
        let code;
        if (!sel.isEmpty) {
            code = editor.document.getText(sel);
        }
        else {
            code = getCellAtCursor(editor);
        }
        if (!code.trim())
            return;
        const terminal = getReplTerminal(projectRoot);
        terminal.show(true);
        terminal.sendText(code, false);
        terminal.sendText('\n' + REPL_SENTINEL);
    }));
    // ---- Diagnostics ----
    const diagCollection = vscode.languages.createDiagnosticCollection('arrow');
    const debounceMap = new Map();
    /** Debounce diagnostics so rapid edits don't trigger a rebuild on every keystroke. */
    function scheduleDiagnostics(document) {
        if (!isArrowDocument(document))
            return;
        const key = document.uri.toString();
        const existing = debounceMap.get(key);
        if (existing)
            clearTimeout(existing);
        debounceMap.set(key, setTimeout(() => {
            debounceMap.delete(key);
            // wasm 解析は 400 行のファイルで 1 ms 未満なので同期で足りる（import 先は初回だけ読み、
            // 以降は wasm が保持したものを使う）。
            try {
                diagCollection.set(document.uri, (0, wasm_providers_1.provideDiagnostics)(document));
            }
            catch { /* 解析に失敗しても拡張は生かす */ }
        }, 200));
    }
    context.subscriptions.push(diagCollection, vscode.workspace.onDidOpenTextDocument(doc => scheduleDiagnostics(doc)), vscode.workspace.onDidChangeTextDocument(e => scheduleDiagnostics(e.document)), vscode.workspace.onDidCloseTextDocument(doc => {
        diagCollection.delete(doc.uri);
        const key = doc.uri.toString();
        const t = debounceMap.get(key);
        if (t) {
            clearTimeout(t);
            debounceMap.delete(key);
        }
        (0, wasm_providers_1.forgetDocument)(doc);
    }));
    // ---- Imported files ----
    // 解析は import 先のモジュールを wasm の中に保持し、打鍵ごとには読み直さない（`invalidateModules`）。
    // import 先になりうるファイルが変わったら（保存・作成・削除）捨てて、開いているドキュメントを解析し直す。
    // ⚠ 拡張子は import の型の出所（`.ar` / `.arc` / `.ars` / `.py` / `.pyi` / `.h` / `.dll` / `.js` / `.rs`）と
    //   設定（`ar_config.json`）。ビルドなどで大量に変わるので、まとめて 1 回にする。
    const watcher = vscode.workspace.createFileSystemWatcher('**/*.{ar,arc,ars,py,pyi,h,hpp,dll,js,rs,json}');
    const reloadImports = () => {
        (0, frontend_1.invalidateModules)();
        (0, wasm_providers_1.clearAnalysisCache)();
        vscode.workspace.textDocuments.forEach(scheduleDiagnostics);
    };
    let reloadTimer;
    const onImportedFileChanged = () => {
        if (reloadTimer)
            clearTimeout(reloadTimer);
        reloadTimer = setTimeout(() => {
            reloadTimer = undefined;
            reloadImports();
        }, 300);
    };
    context.subscriptions.push(watcher, watcher.onDidChange(onImportedFileChanged), watcher.onDidCreate(onImportedFileChanged), watcher.onDidDelete(onImportedFileChanged), 
    // ワークスペースの外（Python の site-packages など）は見張らないので、手で読み直す口を残す。
    vscode.commands.registerCommand('arrow.reloadImports', reloadImports));
    vscode.workspace.textDocuments.forEach(scheduleDiagnostics);
}
exports.activate = activate;
function deactivate() { }
exports.deactivate = deactivate;
//# sourceMappingURL=extension.js.map
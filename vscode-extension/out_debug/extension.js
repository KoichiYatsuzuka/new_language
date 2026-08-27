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
    var _a;
    // 解析は wasm 版フロントエンド（= `cargo run` と同一のソース）が担う。
    // 読み込めない環境では言語機能を諦める。旧正規表現実装へは**戻さない**:
    // 二重実装を残すと「拡張だけ解釈がずれる」問題がそのまま生き延びるため。
    if (!(0, frontend_1.loadFrontend)(context.extensionPath)) {
        vscode.window.showErrorMessage(`Arrow: failed to load the language frontend — code intelligence is disabled. ` +
            `(${(_a = (0, frontend_1.frontendLoadError)()) !== null && _a !== void 0 ? _a : 'unknown error'})`);
        return;
    }
    // 組み込み関数（print / len / …）も同じフロントエンドで解析して取り込む。
    // 読めなくても言語機能は動く（組み込みが候補に出なくなるだけ）。
    (0, wasm_providers_1.loadPrelude)(path.join(context.extensionPath, 'builtins.ars'));
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
            // wasm 解析は 400 行のファイルで 1 ms 未満なので同期で足りる。
            try {
                diagCollection.set(document.uri, (0, wasm_providers_1.provideDiagnostics)(document));
            }
            catch { /* 解析に失敗しても拡張は生かす */ }
        }, 200));
    }
    context.subscriptions.push(diagCollection, vscode.workspace.onDidOpenTextDocument(scheduleDiagnostics), vscode.workspace.onDidChangeTextDocument(e => scheduleDiagnostics(e.document)), vscode.workspace.onDidCloseTextDocument(doc => {
        diagCollection.delete(doc.uri);
        const key = doc.uri.toString();
        const t = debounceMap.get(key);
        if (t) {
            clearTimeout(t);
            debounceMap.delete(key);
        }
        (0, wasm_providers_1.forgetDocument)(doc);
    }));
    vscode.workspace.textDocuments.forEach(scheduleDiagnostics);
}
exports.activate = activate;
function deactivate() { }
exports.deactivate = deactivate;
//# sourceMappingURL=extension.js.map
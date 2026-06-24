"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.deactivate = exports.activate = void 0;
const vscode = require("vscode");
const path = require("path");
const type_infer_1 = require("./type_infer");
const analysis_1 = require("./analysis");
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
/** Walk up from `dir` to find the folder containing Cargo.toml. */
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
/** Return the text of the #%%-delimited cell that contains the cursor line. */
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
// ===== Activation =====
function activate(context) {
    (0, type_infer_1.initBuiltinStub)(path.join(context.extensionPath, 'builtins.ars'));
    context.subscriptions.push(vscode.window.onDidCloseTerminal(t => { if (t === replTerminal)
        replTerminal = undefined; }), vscode.languages.registerInlayHintsProvider({ language: 'arrow' }, { provideInlayHints: type_infer_1.provideInlayHints }), vscode.languages.registerHoverProvider({ language: 'arrow' }, { provideHover: type_infer_1.provideHover }), vscode.languages.registerDocumentSemanticTokensProvider({ language: 'arrow' }, { provideDocumentSemanticTokens: type_infer_1.provideDocumentSemanticTokens }, type_infer_1.SEMANTIC_TOKENS_LEGEND), vscode.languages.registerCompletionItemProvider({ language: 'arrow' }, { provideCompletionItems: type_infer_1.provideCompletionItems }, '.'), vscode.languages.registerDocumentSymbolProvider({ language: 'arrow' }, { provideDocumentSymbols: type_infer_1.provideDocumentSymbols }), vscode.languages.registerSignatureHelpProvider({ language: 'arrow' }, { provideSignatureHelp: type_infer_1.provideSignatureHelp }, '(', ','), vscode.languages.registerDefinitionProvider({ language: 'arrow' }, { provideDefinition: type_infer_1.provideDefinition }));
    // ---- Send-to-REPL command ----
    context.subscriptions.push(vscode.commands.registerCommand('arrow.sendToRepl', () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || editor.document.languageId !== 'arrow') {
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
    function scheduleDiagnostics(document) {
        if (document.languageId !== 'arrow')
            return;
        const key = document.uri.toString();
        const existing = debounceMap.get(key);
        if (existing)
            clearTimeout(existing);
        debounceMap.set(key, setTimeout(() => {
            debounceMap.delete(key);
            (0, type_infer_1.provideDiagnostics)(document).then(diags => diagCollection.set(document.uri, diags));
        }, 400));
    }
    context.subscriptions.push(diagCollection, vscode.workspace.onDidOpenTextDocument(scheduleDiagnostics), vscode.workspace.onDidChangeTextDocument(e => scheduleDiagnostics(e.document)), vscode.workspace.onDidCloseTextDocument(doc => {
        diagCollection.delete(doc.uri);
        const key = doc.uri.toString();
        const t = debounceMap.get(key);
        if (t) {
            clearTimeout(t);
            debounceMap.delete(key);
        }
        analysis_1.DocumentAnalysis.evict(doc.uri);
    }));
    vscode.workspace.textDocuments.forEach(scheduleDiagnostics);
}
exports.activate = activate;
function deactivate() { }
exports.deactivate = deactivate;
//# sourceMappingURL=extension.js.map
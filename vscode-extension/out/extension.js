"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.deactivate = exports.activate = void 0;
const vscode = require("vscode");
const path = require("path");
const type_infer_1 = require("./type_infer");
// ---------------------------------------------------------------------------
// REPL terminal management
// ---------------------------------------------------------------------------
const REPL_SENTINEL = '##REPL_EXEC##';
let replTerminal;
/** Return the live REPL terminal, creating (and starting the interpreter) if needed. */
function getReplTerminal(projectRoot) {
    if (replTerminal && vscode.window.terminals.includes(replTerminal)) {
        return replTerminal;
    }
    replTerminal = vscode.window.createTerminal({ name: 'Havakyrie REPL', cwd: projectRoot });
    replTerminal.sendText('cargo run -- --repl');
    return replTerminal;
}
function activate(context) {
    // Load built-in function stubs for hover / completion / signature-help intelligence.
    (0, type_infer_1.initBuiltinStub)(path.join(context.extensionPath, 'builtins.hvs'));
    // Clear the cached terminal handle when the user closes it.
    context.subscriptions.push(vscode.window.onDidCloseTerminal(t => { if (t === replTerminal)
        replTerminal = undefined; }));
    const inlayProvider = vscode.languages.registerInlayHintsProvider({ language: 'havakyrie' }, {
        provideInlayHints(document, range) {
            return (0, type_infer_1.provideInlayHints)(document, range);
        },
    });
    const hoverProvider = vscode.languages.registerHoverProvider({ language: 'havakyrie' }, {
        provideHover(document, position) {
            return (0, type_infer_1.provideHover)(document, position);
        },
    });
    const semanticProvider = vscode.languages.registerDocumentSemanticTokensProvider({ language: 'havakyrie' }, {
        provideDocumentSemanticTokens(document) {
            return (0, type_infer_1.provideDocumentSemanticTokens)(document);
        },
    }, type_infer_1.SEMANTIC_TOKENS_LEGEND);
    const completionProvider = vscode.languages.registerCompletionItemProvider({ language: 'havakyrie' }, {
        provideCompletionItems(document, position) {
            return (0, type_infer_1.provideCompletionItems)(document, position);
        },
    }, '.' // trigger member completion on dot
    );
    const symbolProvider = vscode.languages.registerDocumentSymbolProvider({ language: 'havakyrie' }, {
        provideDocumentSymbols(document) {
            return (0, type_infer_1.provideDocumentSymbols)(document);
        },
    });
    const signatureProvider = vscode.languages.registerSignatureHelpProvider({ language: 'havakyrie' }, {
        provideSignatureHelp(document, position) {
            return (0, type_infer_1.provideSignatureHelp)(document, position);
        },
    }, '(', ',');
    const definitionProvider = vscode.languages.registerDefinitionProvider({ language: 'havakyrie' }, {
        provideDefinition(document, position) {
            return (0, type_infer_1.provideDefinition)(document, position);
        },
    });
    // Send selected code (or current line) to the persistent REPL terminal.
    const sendToRepl = vscode.commands.registerCommand('havakyrie.sendToRepl', () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || editor.document.languageId !== 'havakyrie') {
            vscode.window.showWarningMessage('Havakyrie REPL: open a .hv file first.');
            return;
        }
        // Resolve the project root: walk up from the file to find Cargo.toml.
        const fileDir = path.dirname(editor.document.uri.fsPath);
        const projectRoot = findCargoRoot(fileDir);
        if (!projectRoot) {
            vscode.window.showErrorMessage('Havakyrie REPL: could not find Cargo.toml above this file.');
            return;
        }
        const sel = editor.selection;
        const code = editor.document.getText(sel.isEmpty ? editor.document.lineAt(sel.active).range : sel);
        if (!code.trim()) {
            return;
        }
        const terminal = getReplTerminal(projectRoot);
        terminal.show(true); // reveal panel without stealing editor focus
        terminal.sendText(code, false); // send code without auto-newline
        terminal.sendText('\n' + REPL_SENTINEL); // sentinel on its own line
    });
    // ---- Diagnostics (red underlines) ----
    const diagCollection = vscode.languages.createDiagnosticCollection('havakyrie');
    const debounceMap = new Map();
    function scheduleDiagnostics(document) {
        if (document.languageId !== 'havakyrie')
            return;
        const key = document.uri.toString();
        const existing = debounceMap.get(key);
        if (existing)
            clearTimeout(existing);
        debounceMap.set(key, setTimeout(() => {
            debounceMap.delete(key);
            diagCollection.set(document.uri, (0, type_infer_1.provideDiagnostics)(document));
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
    }));
    // Run on documents already open when the extension activates
    vscode.workspace.textDocuments.forEach(scheduleDiagnostics);
    context.subscriptions.push(inlayProvider, hoverProvider, semanticProvider, completionProvider, symbolProvider, signatureProvider, definitionProvider, sendToRepl);
}
exports.activate = activate;
/** Walk up the directory tree from `dir` to find the folder containing Cargo.toml. */
function findCargoRoot(dir) {
    const { root } = path.parse(dir);
    let current = dir;
    while (true) {
        const candidate = path.join(current, 'Cargo.toml');
        try {
            require('fs').accessSync(candidate);
            return current;
        }
        catch {
            if (current === root) {
                return undefined;
            }
            current = path.dirname(current);
        }
    }
}
function deactivate() { }
exports.deactivate = deactivate;
//# sourceMappingURL=extension.js.map
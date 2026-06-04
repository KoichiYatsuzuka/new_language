import * as vscode from 'vscode';
import * as path from 'path';
import {
    provideHover,
    provideInlayHints,
    provideDocumentSemanticTokens,
    SEMANTIC_TOKENS_LEGEND,
    provideCompletionItems,
    provideDocumentSymbols,
    provideSignatureHelp,
    provideDefinition,
    provideDiagnostics,
    initBuiltinStub,
} from './type_infer';
import { DocumentAnalysis } from './analysis';

// ===== REPL terminal =====

const REPL_SENTINEL = '##REPL_EXEC##';
let replTerminal: vscode.Terminal | undefined;

function getReplTerminal(projectRoot: string): vscode.Terminal {
    if (replTerminal && vscode.window.terminals.includes(replTerminal)) return replTerminal;
    replTerminal = vscode.window.createTerminal({ name: 'Havakyrie REPL', cwd: projectRoot });
    replTerminal.sendText('cargo run -- --repl');
    return replTerminal;
}

/** Walk up from `dir` to find the folder containing Cargo.toml. */
function findCargoRoot(dir: string): string | undefined {
    const { root } = path.parse(dir);
    let current = dir;
    while (true) {
        try {
            require('fs').accessSync(path.join(current, 'Cargo.toml'));
            return current;
        } catch {
            if (current === root) return undefined;
            current = path.dirname(current);
        }
    }
}

// ===== Activation =====

export function activate(context: vscode.ExtensionContext) {
    initBuiltinStub(path.join(context.extensionPath, 'builtins.hvs'));

    context.subscriptions.push(
        vscode.window.onDidCloseTerminal(t => { if (t === replTerminal) replTerminal = undefined; }),

        vscode.languages.registerInlayHintsProvider(
            { language: 'havakyrie' },
            { provideInlayHints }
        ),
        vscode.languages.registerHoverProvider(
            { language: 'havakyrie' },
            { provideHover }
        ),
        vscode.languages.registerDocumentSemanticTokensProvider(
            { language: 'havakyrie' },
            { provideDocumentSemanticTokens },
            SEMANTIC_TOKENS_LEGEND
        ),
        vscode.languages.registerCompletionItemProvider(
            { language: 'havakyrie' },
            { provideCompletionItems },
            '.'
        ),
        vscode.languages.registerDocumentSymbolProvider(
            { language: 'havakyrie' },
            { provideDocumentSymbols }
        ),
        vscode.languages.registerSignatureHelpProvider(
            { language: 'havakyrie' },
            { provideSignatureHelp },
            '(', ','
        ),
        vscode.languages.registerDefinitionProvider(
            { language: 'havakyrie' },
            { provideDefinition }
        ),
    );

    // ---- Send-to-REPL command ----
    context.subscriptions.push(
        vscode.commands.registerCommand('havakyrie.sendToRepl', () => {
            const editor = vscode.window.activeTextEditor;
            if (!editor || editor.document.languageId !== 'havakyrie') {
                vscode.window.showWarningMessage('Havakyrie REPL: open a .hv file first.');
                return;
            }
            const fileDir = path.dirname(editor.document.uri.fsPath);
            const projectRoot = findCargoRoot(fileDir);
            if (!projectRoot) {
                vscode.window.showErrorMessage('Havakyrie REPL: could not find Cargo.toml above this file.');
                return;
            }
            const sel = editor.selection;
            const code = editor.document.getText(
                sel.isEmpty ? editor.document.lineAt(sel.active).range : sel
            );
            if (!code.trim()) return;
            const terminal = getReplTerminal(projectRoot);
            terminal.show(true);
            terminal.sendText(code, false);
            terminal.sendText('\n' + REPL_SENTINEL);
        })
    );

    // ---- Diagnostics ----
    const diagCollection = vscode.languages.createDiagnosticCollection('havakyrie');
    const debounceMap = new Map<string, ReturnType<typeof setTimeout>>();

    function scheduleDiagnostics(document: vscode.TextDocument): void {
        if (document.languageId !== 'havakyrie') return;
        const key = document.uri.toString();
        const existing = debounceMap.get(key);
        if (existing) clearTimeout(existing);
        debounceMap.set(key, setTimeout(() => {
            debounceMap.delete(key);
            provideDiagnostics(document).then(diags => diagCollection.set(document.uri, diags));
        }, 400));
    }

    context.subscriptions.push(
        diagCollection,
        vscode.workspace.onDidOpenTextDocument(scheduleDiagnostics),
        vscode.workspace.onDidChangeTextDocument(e => scheduleDiagnostics(e.document)),
        vscode.workspace.onDidCloseTextDocument(doc => {
            diagCollection.delete(doc.uri);
            const key = doc.uri.toString();
            const t = debounceMap.get(key);
            if (t) { clearTimeout(t); debounceMap.delete(key); }
            DocumentAnalysis.evict(doc.uri);
        })
    );

    vscode.workspace.textDocuments.forEach(scheduleDiagnostics);
}

export function deactivate() {}

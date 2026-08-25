/**
 * extension.ts — VS Code extension entry point for the Arrow language.
 *
 * Responsibilities:
 * - Initialize the extension on activation (`activate`)
 * - Register all language-feature providers (hover, inlay hints, completions, …)
 *   defined in `type_infer.ts`
 * - Implement the "Send to REPL" command and REPL terminal management
 * - Schedule debounced diagnostics on document open/change events
 */

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
    replTerminal = vscode.window.createTerminal({ name: 'Arrow REPL', cwd: projectRoot });
    replTerminal.sendText('cargo run -- --repl');
    return replTerminal;
}

/**
 * Walk up from `dir` until a directory containing `Cargo.toml` is found.
 * Returns `undefined` when no Cargo project root exists above the given path.
 */
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

// ===== Cell helpers =====

const CELL_MARKER = /^#%%/;

/**
 * Return the text of the `#%%`-delimited cell that contains the cursor line.
 * If no `#%%` marker surrounds the cursor, the entire document is treated as one cell.
 */
function getCellAtCursor(editor: vscode.TextEditor): string {
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
const ARROW_SELECTOR: vscode.DocumentSelector = [
    { language: 'arrow' },
    { language: 'arrow-stub' },
];

/** True for documents the language features apply to (`.ar` / `.ars`). */
function isArrowDocument(document: vscode.TextDocument): boolean {
    return document.languageId === 'arrow' || document.languageId === 'arrow-stub';
}

// ===== Activation =====

export function activate(context: vscode.ExtensionContext) {
    initBuiltinStub(path.join(context.extensionPath, 'builtins.ars'));

    context.subscriptions.push(
        vscode.window.onDidCloseTerminal(t => { if (t === replTerminal) replTerminal = undefined; }),

        vscode.languages.registerInlayHintsProvider(
            ARROW_SELECTOR,
            { provideInlayHints }
        ),
        vscode.languages.registerHoverProvider(
            ARROW_SELECTOR,
            { provideHover }
        ),
        vscode.languages.registerDocumentSemanticTokensProvider(
            ARROW_SELECTOR,
            { provideDocumentSemanticTokens },
            SEMANTIC_TOKENS_LEGEND
        ),
        vscode.languages.registerCompletionItemProvider(
            ARROW_SELECTOR,
            { provideCompletionItems },
            '.'
        ),
        vscode.languages.registerDocumentSymbolProvider(
            ARROW_SELECTOR,
            { provideDocumentSymbols }
        ),
        vscode.languages.registerSignatureHelpProvider(
            ARROW_SELECTOR,
            { provideSignatureHelp },
            '(', ','
        ),
        vscode.languages.registerDefinitionProvider(
            ARROW_SELECTOR,
            { provideDefinition }
        ),
    );

    // ---- Send-to-REPL command ----
    context.subscriptions.push(
        vscode.commands.registerCommand('arrow.sendToRepl', () => {
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
            let code: string;
            if (!sel.isEmpty) {
                code = editor.document.getText(sel);
            } else {
                code = getCellAtCursor(editor);
            }
            if (!code.trim()) return;
            const terminal = getReplTerminal(projectRoot);
            terminal.show(true);
            terminal.sendText(code, false);
            terminal.sendText('\n' + REPL_SENTINEL);
        })
    );

    // ---- Diagnostics ----
    const diagCollection = vscode.languages.createDiagnosticCollection('arrow');
    const debounceMap = new Map<string, ReturnType<typeof setTimeout>>();

    /** Debounce diagnostics so rapid edits don't trigger a rebuild on every keystroke. */
    function scheduleDiagnostics(document: vscode.TextDocument): void {
        if (!isArrowDocument(document)) return;
        const key = document.uri.toString();
        const existing = debounceMap.get(key);
        if (existing) clearTimeout(existing);
        debounceMap.set(key, setTimeout(() => {
            debounceMap.delete(key);
            provideDiagnostics(document)
                .then(diags => diagCollection.set(document.uri, diags))
                .catch(_err => { /* suppress unhandled rejection — extension stays alive */ });
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

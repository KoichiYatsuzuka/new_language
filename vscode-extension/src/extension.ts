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
    initBuiltinStub,
} from './type_infer';

// ---------------------------------------------------------------------------
// REPL terminal management
// ---------------------------------------------------------------------------

const REPL_SENTINEL = '##REPL_EXEC##';
let replTerminal: vscode.Terminal | undefined;

/** Return the live REPL terminal, creating (and starting the interpreter) if needed. */
function getReplTerminal(projectRoot: string): vscode.Terminal {
    if (replTerminal && vscode.window.terminals.includes(replTerminal)) {
        return replTerminal;
    }
    replTerminal = vscode.window.createTerminal({ name: 'test_lang REPL', cwd: projectRoot });
    replTerminal.sendText('cargo run -- --repl');
    return replTerminal;
}

export function activate(context: vscode.ExtensionContext) {
    // Load built-in function stubs for hover / completion / signature-help intelligence.
    initBuiltinStub(path.join(context.extensionPath, 'builtins.tls'));

    // Clear the cached terminal handle when the user closes it.
    context.subscriptions.push(
        vscode.window.onDidCloseTerminal(t => { if (t === replTerminal) replTerminal = undefined; })
    );
    const inlayProvider = vscode.languages.registerInlayHintsProvider(
        { language: 'test_lang' },
        {
            provideInlayHints(
                document: vscode.TextDocument,
                range: vscode.Range
            ): vscode.InlayHint[] {
                return provideInlayHints(document, range);
            },
        }
    );
    const hoverProvider = vscode.languages.registerHoverProvider(
        { language: 'test_lang' },
        {
            provideHover(
                document: vscode.TextDocument,
                position: vscode.Position
            ): vscode.Hover | undefined {
                return provideHover(document, position);
            },
        }
    );
    const semanticProvider = vscode.languages.registerDocumentSemanticTokensProvider(
        { language: 'test_lang' },
        {
            provideDocumentSemanticTokens(
                document: vscode.TextDocument
            ): vscode.SemanticTokens {
                return provideDocumentSemanticTokens(document);
            },
        },
        SEMANTIC_TOKENS_LEGEND
    );
    const completionProvider = vscode.languages.registerCompletionItemProvider(
        { language: 'test_lang' },
        {
            provideCompletionItems(
                document: vscode.TextDocument,
                position: vscode.Position
            ): vscode.CompletionItem[] {
                return provideCompletionItems(document, position);
            },
        },
        '.'  // trigger member completion on dot
    );
    const symbolProvider = vscode.languages.registerDocumentSymbolProvider(
        { language: 'test_lang' },
        {
            provideDocumentSymbols(
                document: vscode.TextDocument
            ): vscode.DocumentSymbol[] {
                return provideDocumentSymbols(document);
            },
        }
    );
    const signatureProvider = vscode.languages.registerSignatureHelpProvider(
        { language: 'test_lang' },
        {
            provideSignatureHelp(
                document: vscode.TextDocument,
                position: vscode.Position
            ): vscode.SignatureHelp | undefined {
                return provideSignatureHelp(document, position);
            },
        },
        '(', ','
    );
    const definitionProvider = vscode.languages.registerDefinitionProvider(
        { language: 'test_lang' },
        {
            provideDefinition(
                document: vscode.TextDocument,
                position: vscode.Position
            ): vscode.Location | undefined {
                return provideDefinition(document, position);
            },
        }
    );
    // Send selected code (or current line) to the persistent REPL terminal.
    const sendToRepl = vscode.commands.registerCommand('test_lang.sendToRepl', () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || editor.document.languageId !== 'test_lang') {
            vscode.window.showWarningMessage('test_lang REPL: open a .tl file first.');
            return;
        }

        // Resolve the project root: walk up from the file to find Cargo.toml.
        const fileDir = path.dirname(editor.document.uri.fsPath);
        const projectRoot = findCargoRoot(fileDir);
        if (!projectRoot) {
            vscode.window.showErrorMessage('test_lang REPL: could not find Cargo.toml above this file.');
            return;
        }

        const sel = editor.selection;
        const code = editor.document.getText(
            sel.isEmpty ? editor.document.lineAt(sel.active).range : sel
        );
        if (!code.trim()) { return; }

        const terminal = getReplTerminal(projectRoot);
        terminal.show(true); // reveal panel without stealing editor focus
        terminal.sendText(code, false); // send code without auto-newline
        terminal.sendText('\n' + REPL_SENTINEL); // sentinel on its own line
    });

    context.subscriptions.push(
        inlayProvider, hoverProvider, semanticProvider,
        completionProvider, symbolProvider, signatureProvider, definitionProvider,
        sendToRepl
    );
}

/** Walk up the directory tree from `dir` to find the folder containing Cargo.toml. */
function findCargoRoot(dir: string): string | undefined {
    const { root } = path.parse(dir);
    let current = dir;
    while (true) {
        const candidate = path.join(current, 'Cargo.toml');
        try {
            require('fs').accessSync(candidate);
            return current;
        } catch {
            if (current === root) { return undefined; }
            current = path.dirname(current);
        }
    }
}

export function deactivate() {}

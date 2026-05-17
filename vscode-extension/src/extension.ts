import * as vscode from 'vscode';
import {
    provideHover,
    provideInlayHints,
    provideDocumentSemanticTokens,
    SEMANTIC_TOKENS_LEGEND,
    provideCompletionItems,
    provideDocumentSymbols,
    provideSignatureHelp,
    provideDefinition,
} from './type_infer';

export function activate(context: vscode.ExtensionContext) {
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
    context.subscriptions.push(
        inlayProvider, hoverProvider, semanticProvider,
        completionProvider, symbolProvider, signatureProvider, definitionProvider
    );
}

export function deactivate() {}

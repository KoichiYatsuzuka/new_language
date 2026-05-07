import * as vscode from 'vscode';
import { provideHover, provideInlayHints } from './type_infer';

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
    context.subscriptions.push(inlayProvider, hoverProvider);
}

export function deactivate() {}

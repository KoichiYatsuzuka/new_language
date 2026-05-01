import * as vscode from 'vscode';
import { provideInlayHints } from './type_infer';

export function activate(context: vscode.ExtensionContext) {
    const provider = vscode.languages.registerInlayHintsProvider(
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
    context.subscriptions.push(provider);
}

export function deactivate() {}

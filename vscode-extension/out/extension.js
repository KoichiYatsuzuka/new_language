"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.deactivate = exports.activate = void 0;
const vscode = require("vscode");
const type_infer_1 = require("./type_infer");
function activate(context) {
    const inlayProvider = vscode.languages.registerInlayHintsProvider({ language: 'test_lang' }, {
        provideInlayHints(document, range) {
            return (0, type_infer_1.provideInlayHints)(document, range);
        },
    });
    const hoverProvider = vscode.languages.registerHoverProvider({ language: 'test_lang' }, {
        provideHover(document, position) {
            return (0, type_infer_1.provideHover)(document, position);
        },
    });
    const semanticProvider = vscode.languages.registerDocumentSemanticTokensProvider({ language: 'test_lang' }, {
        provideDocumentSemanticTokens(document) {
            return (0, type_infer_1.provideDocumentSemanticTokens)(document);
        },
    }, type_infer_1.SEMANTIC_TOKENS_LEGEND);
    const completionProvider = vscode.languages.registerCompletionItemProvider({ language: 'test_lang' }, {
        provideCompletionItems(document, position) {
            return (0, type_infer_1.provideCompletionItems)(document, position);
        },
    }, '.' // trigger member completion on dot
    );
    const symbolProvider = vscode.languages.registerDocumentSymbolProvider({ language: 'test_lang' }, {
        provideDocumentSymbols(document) {
            return (0, type_infer_1.provideDocumentSymbols)(document);
        },
    });
    const signatureProvider = vscode.languages.registerSignatureHelpProvider({ language: 'test_lang' }, {
        provideSignatureHelp(document, position) {
            return (0, type_infer_1.provideSignatureHelp)(document, position);
        },
    }, '(', ',');
    const definitionProvider = vscode.languages.registerDefinitionProvider({ language: 'test_lang' }, {
        provideDefinition(document, position) {
            return (0, type_infer_1.provideDefinition)(document, position);
        },
    });
    context.subscriptions.push(inlayProvider, hoverProvider, semanticProvider, completionProvider, symbolProvider, signatureProvider, definitionProvider);
}
exports.activate = activate;
function deactivate() { }
exports.deactivate = deactivate;
//# sourceMappingURL=extension.js.map
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
    context.subscriptions.push(inlayProvider, hoverProvider, semanticProvider);
}
exports.activate = activate;
function deactivate() { }
exports.deactivate = deactivate;
//# sourceMappingURL=extension.js.map
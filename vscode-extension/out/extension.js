"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
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
    context.subscriptions.push(inlayProvider, hoverProvider);
}
function deactivate() { }
//# sourceMappingURL=extension.js.map
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
const vscode = require("vscode");
const type_infer_1 = require("./type_infer");
function activate(context) {
    const provider = vscode.languages.registerInlayHintsProvider({ language: 'test_lang' }, {
        provideInlayHints(document, range) {
            return (0, type_infer_1.provideInlayHints)(document, range);
        },
    });
    context.subscriptions.push(provider);
}
function deactivate() { }
//# sourceMappingURL=extension.js.map
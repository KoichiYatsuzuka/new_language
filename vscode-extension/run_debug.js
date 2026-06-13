#!/usr/bin/env node
'use strict';
/**
 * Bootstrap for the standalone Arrow extension debugger.
 *
 * Intercepts require('vscode') BEFORE any extension module is loaded,
 * so that analysis.ts / type_infer.ts receive the vscode_mock implementations
 * instead of the real VS Code extension host API.
 *
 * Usage:
 *   node run_debug.js <path/to/file.ar>
 *
 * Compile first with:
 *   npx tsc -p tsconfig.debug.json
 */

// Polyfill Array.prototype.at for Node.js < 16.6
if (!Array.prototype.at) {
    // eslint-disable-next-line no-extend-native
    Array.prototype.at = function (index) {
        const i = index < 0 ? this.length + index : index;
        return this[i];
    };
}

// Intercept require('vscode') → vscode_mock
const Module = require('module');
const path   = require('path');

const originalLoad = Module._load.bind(Module);
Module._load = function (request, parent, isMain) {
    if (request === 'vscode') {
        return require(path.join(__dirname, 'out_debug', 'vscode_mock'));
    }
    return originalLoad(request, parent, isMain);
};

// Run the debug runner (out_debug/ compiled with ES2019 target for older Node.js)
require('./out_debug/debug_runner');

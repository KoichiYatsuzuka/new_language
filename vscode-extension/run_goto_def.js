#!/usr/bin/env node
'use strict';
/**
 * Bootstrap for the Go-to-Definition test runner.
 * Intercepts require('vscode') → vscode_mock, then runs test_goto_def.
 *
 * Usage:
 *   node run_goto_def.js <path/to/file.ar>
 *
 * Compile first:
 *   npm run compile:debug
 */

if (!Array.prototype.at) {
    Array.prototype.at = function (index) {
        const i = index < 0 ? this.length + index : index;
        return this[i];
    };
}

const Module = require('module');
const path   = require('path');

const originalLoad = Module._load.bind(Module);
Module._load = function (request, parent, isMain) {
    if (request === 'vscode') {
        return require(path.join(__dirname, 'out_debug', 'vscode_mock'));
    }
    return originalLoad(request, parent, isMain);
};

require('./out_debug/test_goto_def');

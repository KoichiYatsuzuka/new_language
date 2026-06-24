'use strict';
// js_bridge.cjs — Node.js named-pipe IPC server for Arrow import[js-proc].
//
// Usage (called by Arrow runtime):
//   node js_bridge.cjs <pipe_name> <bridge_root>
//
// Protocol: newline-delimited JSON over a Windows named pipe.
//   Request:  {"id":N,"op":"call"|"list"|"quit","module":"a/b","fn":"fnName","args":[...]}
//   Response: {"id":N,"ok":{t,v}} | {"id":N,"err":"message"}
//
// Arg/result type tags:
//   "i" = int    "f" = float    "b" = bool
//   "s" = string "n" = null/undefined
//   "a" = array  "o" = object (encoded as {key: {t,v}, ...})

const net  = require('net');
const path = require('path');
const fs   = require('fs');

const PIPE_NAME  = process.argv[2];
const BRIDGE_ROOT = path.resolve(process.argv[3] || process.cwd());

if (!PIPE_NAME) {
    process.stderr.write('js_bridge: missing pipe_name argument\n');
    process.exit(1);
}

// ── vscode mock intercept ─────────────────────────────────────────────────────
// Redirect require('vscode') to out_debug/vscode_mock so that VS Code extension
// modules (analysis.js, type_infer.js, etc.) work outside the extension host.
const Module = require('module');
const _origLoad = Module._load.bind(Module);
Module._load = function (request, parent, isMain) {
    if (request === 'vscode') {
        const mockPath = path.join(BRIDGE_ROOT, 'out_debug', 'vscode_mock');
        try { return _origLoad(mockPath, parent, isMain); } catch (_) { return {}; }
    }
    return _origLoad(request, parent, isMain);
};

// ── Module registry ───────────────────────────────────────────────────────────
const loadedModules = {};

function loadModule(moduleName) {
    if (loadedModules[moduleName]) return loadedModules[moduleName];

    // Convert dot/slash separators to OS path separators and resolve.
    const relPath = moduleName.replace(/\./g, '/');
    const candidates = [
        path.resolve(BRIDGE_ROOT, relPath),
        path.resolve(BRIDGE_ROOT, relPath + '.js'),
        path.resolve(BRIDGE_ROOT, relPath + '.cjs'),
        // Also search in the bridge script's own directory
        path.resolve(__dirname, relPath),
        path.resolve(__dirname, relPath + '.js'),
        path.resolve(__dirname, relPath + '.cjs'),
        moduleName,   // bare npm package / Node.js built-in
    ];

    let mod, lastErr;
    for (const c of candidates) {
        try { mod = require(c); break; } catch (e) { lastErr = e; }
    }
    if (mod === undefined) throw lastErr;

    loadedModules[moduleName] = mod;
    return mod;
}

// ── Encoding / decoding ───────────────────────────────────────────────────────
function encodeValue(val) {
    if (val === null || val === undefined) return { t: 'n' };
    if (typeof val === 'boolean') return { t: 'b', v: val };
    if (typeof val === 'number') {
        return Number.isInteger(val) ? { t: 'i', v: val } : { t: 'f', v: val };
    }
    if (typeof val === 'string') return { t: 's', v: val };
    if (Array.isArray(val)) return { t: 'a', v: val.map(encodeValue) };
    if (typeof val === 'object') {
        const obj = {};
        for (const [k, v] of Object.entries(val)) obj[k] = encodeValue(v);
        return { t: 'o', v: obj };
    }
    return { t: 's', v: String(val) };
}

function decodeArg(a) {
    if (!a || a.t === 'n') return null;
    if (a.t === 'b') return Boolean(a.v);
    if (a.t === 'i' || a.t === 'f') return Number(a.v);
    if (a.t === 's') return String(a.v);
    if (a.t === 'a') return (a.v || []).map(decodeArg);
    if (a.t === 'o') {
        const obj = {};
        for (const [k, v] of Object.entries(a.v || {})) obj[k] = decodeArg(v);
        return obj;
    }
    return null;
}

// ── Request handler ───────────────────────────────────────────────────────────
async function handleRequest(req) {
    switch (req.op) {
        case 'list': {
            const mod = loadModule(req.module);
            const fns = Object.keys(mod).filter(k => typeof mod[k] === 'function');
            return { t: 'a', v: fns.map(f => ({ t: 's', v: f })) };
        }
        case 'call': {
            const mod = loadModule(req.module);
            const fn  = mod[req.fn];
            if (typeof fn !== 'function')
                throw new Error(`'${req.fn}' is not a function in module '${req.module}'`);
            const args   = (req.args || []).map(decodeArg);
            const result = await Promise.resolve(fn.apply(mod, args));
            return encodeValue(result);
        }
        case 'quit':
            return { t: 'n' };
        default:
            throw new Error(`unknown op: ${req.op}`);
    }
}

// ── Named-pipe server ─────────────────────────────────────────────────────────
const server = net.createServer((socket) => {
    let buf = '';

    socket.on('data', (chunk) => {
        buf += chunk.toString('utf8');
        let nl;
        while ((nl = buf.indexOf('\n')) !== -1) {
            const line = buf.slice(0, nl).trim();
            buf = buf.slice(nl + 1);
            if (!line) continue;

            let req;
            try { req = JSON.parse(line); }
            catch (e) {
                socket.write(JSON.stringify({ id: 0, err: `JSON parse: ${e.message}` }) + '\n');
                continue;
            }

            handleRequest(req)
                .then(result => {
                    socket.write(JSON.stringify({ id: req.id, ok: result }) + '\n');
                    if (req.op === 'quit') {
                        socket.end();
                        server.close();
                        process.exit(0);
                    }
                })
                .catch(err => {
                    socket.write(JSON.stringify({ id: req.id, err: String(err) }) + '\n');
                });
        }
    });

    socket.on('error', (e) => process.stderr.write(`js_bridge socket error: ${e}\n`));
});

server.on('error', (e) => {
    process.stderr.write(`js_bridge server error: ${e}\n`);
    process.exit(1);
});

server.listen(PIPE_NAME, () => {
    // Signal Arrow runtime that the pipe is ready.
    process.stdout.write('READY\n');
});

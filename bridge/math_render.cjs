'use strict';
// math_render.cjs — TeX 数式を PNG/SVG 画像にレンダリングするブリッジモジュール。
//
// LaTeX Workshop が採用するのと同じパイプライン:
//   latex -interaction=nonstopmode  →  .dvi
//   dvipng  (PNG) / dvisvgm  (SVG)
//
// 依存: latex, dvipng, dvisvgm が PATH に存在すること（TeX Live 等）

const { spawnSync } = require('child_process');
const { mkdtempSync, writeFileSync, readFileSync, mkdirSync, existsSync } = require('fs');
const { rmSync }   = require('fs');
const os   = require('os');
const path = require('path');

// ── LaTeX ドキュメントテンプレート ──────────────────────────────────────────────
// standalone + preview クラスで余白なし・数式のみのページを生成する。
// LaTeX Workshop の hover preview と同じアプローチ。

function buildLatex(formula, displayMode) {
    const body = displayMode
        ? `\\[${formula}\\]`
        : `$${formula}$`;
    return [
        '\\documentclass[preview]{standalone}',
        '\\usepackage{amsmath,amssymb,amsfonts,bm}',
        '\\begin{document}',
        body,
        '\\end{document}',
    ].join('\n');
}

// ── 内部: コンパイル → .dvi ───────────────────────────────────────────────────

function compileToDvi(formula, displayMode) {
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), 'arrow_math_'));
    const texFile = path.join(tmpDir, 'formula.tex');
    writeFileSync(texFile, buildLatex(formula, displayMode), 'utf8');

    const res = spawnSync('latex', [
        '-interaction=nonstopmode',
        '-halt-on-error',
        '-output-directory', tmpDir,
        texFile,
    ], { encoding: 'utf8', timeout: 30000 });

    if (res.status !== 0) {
        const log = res.stdout || res.stderr || '';
        // LaTeX エラーメッセージから ! で始まる行を抜き出す
        const errLine = log.split('\n').find(l => l.startsWith('!')) || log.slice(-200);
        rmSync(tmpDir, { recursive: true, force: true });
        throw new Error('latex: ' + errLine.trim());
    }

    return { tmpDir, dviFile: path.join(tmpDir, 'formula.dvi') };
}

// ── 公開 API ─────────────────────────────────────────────────────────────────

/**
 * TeX 数式を PNG 画像にレンダリングしてファイルに保存する。
 * @param {string} formula     - LaTeX 数式（バックスラッシュエスケープ済み）
 * @param {boolean} displayMode - true = \[ \] (display), false = $ $ (inline)
 * @param {string} outputPath  - 保存先パス (.png)
 * @param {number} dpi         - 解像度 (推奨: 150–300)
 * @returns {string} "" = 成功, それ以外 = エラーメッセージ
 */
function renderPNG(formula, displayMode, outputPath, dpi) {
    dpi = dpi || 200;
    displayMode = (displayMode !== false);
    let tmpDir;
    try {
        const { tmpDir: td, dviFile } = compileToDvi(formula, displayMode);
        tmpDir = td;

        mkdirSync(path.dirname(path.resolve(outputPath)), { recursive: true });

        const res = spawnSync('dvipng', [
            '-D', String(dpi),
            '-T', 'tight',
            '-bg', 'Transparent',
            '-fg', 'Black',
            '-o', path.resolve(outputPath),
            dviFile,
        ], { encoding: 'utf8', timeout: 15000 });

        if (res.status !== 0) {
            throw new Error('dvipng: ' + (res.stderr || res.stdout || '').trim());
        }
        return '';
    } catch (e) {
        return String(e.message || e);
    } finally {
        if (tmpDir) rmSync(tmpDir, { recursive: true, force: true });
    }
}

/**
 * TeX 数式を SVG にレンダリングしてファイルに保存する。
 * @param {string} formula     - LaTeX 数式
 * @param {boolean} displayMode - true = display mode
 * @param {string} outputPath  - 保存先パス (.svg)
 * @param {number} scale       - スケール倍率 (デフォルト 2.0)
 * @returns {string} "" = 成功, それ以外 = エラーメッセージ
 */
function renderSVG(formula, displayMode, outputPath, scale) {
    scale = scale || 2.0;
    displayMode = (displayMode !== false);
    let tmpDir;
    try {
        const { tmpDir: td, dviFile } = compileToDvi(formula, displayMode);
        tmpDir = td;

        mkdirSync(path.dirname(path.resolve(outputPath)), { recursive: true });

        const res = spawnSync('dvisvgm', [
            '--no-fonts',
            `--scale=${scale}`,
            '--bbox=min',
            '-o', path.resolve(outputPath),
            dviFile,
        ], { encoding: 'utf8', timeout: 15000 });

        if (res.status !== 0) {
            throw new Error('dvisvgm: ' + (res.stderr || res.stdout || '').trim());
        }
        return '';
    } catch (e) {
        return String(e.message || e);
    } finally {
        if (tmpDir) rmSync(tmpDir, { recursive: true, force: true });
    }
}

/**
 * SVG 文字列として返す（ファイル保存しない）。
 * @param {string} formula
 * @param {boolean} displayMode
 * @param {number} scale
 * @returns {string} SVG XML 文字列（エラー時は "ERROR: ..." で始まる文字列）
 */
function renderSVGString(formula, displayMode, scale) {
    const tmpOut = path.join(os.tmpdir(), `arrow_math_out_${Date.now()}.svg`);
    const err = renderSVG(formula, displayMode, tmpOut, scale);
    if (err) return 'ERROR: ' + err;
    try {
        const svg = readFileSync(tmpOut, 'utf8');
        try { rmSync(tmpOut); } catch (_) {}
        return svg;
    } catch (e) {
        return 'ERROR: ' + e.message;
    }
}

module.exports = { renderPNG, renderSVG, renderSVGString };

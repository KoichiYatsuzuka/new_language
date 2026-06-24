'use strict';
// lw_math.cjs — LaTeX Workshop の MathJax を流用して TeX 数式を SVG にレンダリングする。
//
// LaTeX Workshop の hover preview (onmath.js) と同じパイプラインを再現:
//   TeX string  →  mathjax-full (TeX → SVG)  →  SVG/HTML gallery file
//
// LaTeX Workshop がインストールされている環境で動作する（mathjax-full を借用）。

const path = require('path');
const fs   = require('fs');
const os   = require('os');

// ── LaTeX Workshop のインストールパスを自動検出 ───────────────────────────────

function findLWNodeModules() {
    const extDir = path.join(os.homedir(), '.vscode', 'extensions');
    if (!fs.existsSync(extDir)) throw new Error('VS Code extensions dir not found: ' + extDir);
    const lw = fs.readdirSync(extDir)
        .filter(d => d.startsWith('james-yu.latex-workshop-'))
        .sort()
        .pop();
    if (!lw) throw new Error('LaTeX Workshop is not installed in ' + extDir);
    return path.join(extDir, lw, 'node_modules');
}

const LW_MODULES = findLWNodeModules();
const MJ = path.join(LW_MODULES, 'mathjax-full', 'js');

// ── MathJax セットアップ（LaTeX Workshop の mathjax/mathjax.js と同じ設定） ──

const { mathjax }           = require(path.join(MJ, 'mathjax.js'));
const { TeX }               = require(path.join(MJ, 'input', 'tex.js'));
const { SVG }               = require(path.join(MJ, 'output', 'svg.js'));
const { liteAdaptor }       = require(path.join(MJ, 'adaptors', 'liteAdaptor.js'));
const { RegisterHTMLHandler }= require(path.join(MJ, 'handlers', 'html.js'));
require(path.join(MJ, 'input', 'tex', 'AllPackages.js'));

const adaptor = liteAdaptor();
RegisterHTMLHandler(adaptor);

// LaTeX Workshop の baseExtensions と同一
const BASE_EXT = ['ams', 'base', 'boldsymbol', 'color', 'configmacros',
                  'mathtools', 'newcommand', 'noerrors', 'noundefined'];

function makeDoc(extensions) {
    return mathjax.document('', {
        InputJax: new TeX({
            packages: extensions,
            macros: { bm: ['\\boldsymbol{#1}', 1] },
            formatError: (_jax, e) => { throw new Error(e.message); },
        }),
        OutputJax: new SVG({ fontCache: 'local' }),
    });
}

let doc = makeDoc(BASE_EXT);

// ── コア: LaTeX Workshop の typeset() 相当 ────────────────────────────────────

function typeset(formula, displayMode, scale, color) {
    const node = doc.convert(formula, {
        display:        displayMode !== false,
        em:             18,
        ex:             9,
        containerWidth: 80 * 18,
    });
    const css = `svg { font-size: ${100 * (scale || 1.0)}%; } * { color: ${color || '#000000'} }`;
    let svg = adaptor.innerHTML(node);
    svg = svg.replace(/<defs>/, '<defs><style>' + css + '</style>');
    return svg;
}

// ── 公開 API ─────────────────────────────────────────────────────────────────

/**
 * TeX 数式を SVG 文字列として返す。
 * LaTeX Workshop の hover preview と同じ MathJax パイプラインを使用。
 *
 * @param {string}  formula     - LaTeX 数式（バックスラッシュ 1 個）
 * @param {boolean} displayMode - true = display (\[…\]), false = inline ($…$)
 * @param {number}  scale       - フォントスケール (1.0 = 100%)
 * @param {string}  color       - 文字色 CSS 値 (例: "#1a1a2e")
 * @returns {string} SVG XML 文字列（エラー時は "ERROR: …" で始まる文字列）
 */
function renderSVG(formula, displayMode, scale, color) {
    try {
        return typeset(formula, displayMode, scale, color);
    } catch (e) {
        return 'ERROR: ' + String(e.message || e);
    }
}

/**
 * TeX 数式を SVG ファイルに保存する。
 *
 * @param {string}  formula
 * @param {boolean} displayMode
 * @param {string}  outputPath  - 保存先 (.svg)
 * @param {number}  scale
 * @param {string}  color
 * @returns {string} "" = 成功, それ以外 = エラーメッセージ
 */
function renderSVGToFile(formula, displayMode, outputPath, scale, color) {
    try {
        fs.mkdirSync(path.dirname(path.resolve(outputPath)), { recursive: true });
        const svg = typeset(formula, displayMode, scale, color);
        // スタンドアロン SVG として有効な XML 宣言を先頭に付ける
        const xmlHeader = '<?xml version="1.0" encoding="UTF-8"?>\n';
        fs.writeFileSync(outputPath, xmlHeader + svg, 'utf8');
        return '';
    } catch (e) {
        return String(e.message || e);
    }
}

/**
 * 出力ディレクトリ内の .svg ファイルを一覧にした HTML ギャラリーを生成する。
 * LaTeX Workshop の数式ホバープレビューと同じ見た目をブラウザで再現する。
 *
 * @param {string} outputDir - SVG ファイルが保存されているディレクトリ
 * @returns {string} "" = 成功, それ以外 = エラーメッセージ
 */
function renderGalleryHTML(outputDir) {
    try {
        const absDir = path.resolve(outputDir);
        const svgFiles = fs.readdirSync(absDir).filter(f => f.endsWith('.svg')).sort();

        const items = svgFiles.map(f => {
            const name = f.replace(/\.svg$/, '');
            const svg  = fs.readFileSync(path.join(absDir, f), 'utf8');
            // SVG の XML 宣言を除いて <svg> タグだけ埋め込む
            const svgBody = svg.replace(/<\?xml[^>]*\?>/, '').trim();
            return `
  <div class="card">
    <div class="label">${escapeHtml(name)}</div>
    <div class="formula">${svgBody}</div>
  </div>`;
        }).join('\n');

        const html = `<!DOCTYPE html>
<html lang="ja">
<head>
<meta charset="UTF-8">
<title>TeX Math Preview (LaTeX Workshop MathJax)</title>
<style>
  body { background: #1e1e2e; color: #cdd6f4; font-family: 'Segoe UI', sans-serif; padding: 2rem; }
  h1   { font-size: 1.4rem; color: #89b4fa; border-bottom: 1px solid #313244; padding-bottom: .5rem; }
  .grid { display: flex; flex-wrap: wrap; gap: 1.5rem; margin-top: 1.5rem; }
  .card { background: #181825; border: 1px solid #313244; border-radius: 8px;
          padding: 1.2rem 1.5rem; min-width: 200px; box-shadow: 0 2px 8px rgba(0,0,0,.4); }
  .label   { font-size: .75rem; color: #6c7086; margin-bottom: .8rem; letter-spacing: .05em; }
  .formula { display: flex; justify-content: center; align-items: center; min-height: 60px; }
  .formula svg { max-width: 100%; height: auto; }
  .formula svg * { color: #cdd6f4 !important; }
</style>
</head>
<body>
<h1>TeX Math Preview — LaTeX Workshop MathJax pipeline</h1>
<p style="font-size:.85rem;color:#6c7086">
  Rendered with MathJax from <code>james-yu.latex-workshop</code> VS Code extension.
</p>
<div class="grid">
${items}
</div>
</body>
</html>`;

        fs.writeFileSync(path.join(absDir, 'gallery.html'), html, 'utf8');
        return '';
    } catch (e) {
        return String(e.message || e);
    }
}

function escapeHtml(s) {
    return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}

module.exports = { renderSVG, renderSVGToFile, renderGalleryHTML };

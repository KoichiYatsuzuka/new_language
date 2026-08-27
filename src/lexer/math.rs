/// 文字を上付き Unicode 文字列に変換する。対応しない文字は `None` を返す。
fn to_superscript_char(c: char) -> Option<&'static str> {
    Some(match c {
        '0' => "⁰",
        '1' => "¹",
        '2' => "²",
        '3' => "³",
        '4' => "⁴",
        '5' => "⁵",
        '6' => "⁶",
        '7' => "⁷",
        '8' => "⁸",
        '9' => "⁹",
        'a' => "ᵃ",
        'b' => "ᵇ",
        'c' => "ᶜ",
        'd' => "ᵈ",
        'e' => "ᵉ",
        'f' => "ᶠ",
        'g' => "ᵍ",
        'h' => "ʰ",
        'i' => "ⁱ",
        'j' => "ʲ",
        'k' => "ᵏ",
        'l' => "ˡ",
        'm' => "ᵐ",
        'n' => "ⁿ",
        'o' => "ᵒ",
        'p' => "ᵖ",
        'r' => "ʳ",
        's' => "ˢ",
        't' => "ᵗ",
        'u' => "ᵘ",
        'v' => "ᵛ",
        'w' => "ʷ",
        'x' => "ˣ",
        'y' => "ʸ",
        'z' => "ᶻ",
        'A' => "ᴬ",
        'B' => "ᴮ",
        'D' => "ᴰ",
        'E' => "ᴱ",
        'G' => "ᴳ",
        'H' => "ᴴ",
        'I' => "ᴵ",
        'J' => "ᴶ",
        'K' => "ᴷ",
        'L' => "ᴸ",
        'M' => "ᴹ",
        'N' => "ᴺ",
        'O' => "ᴼ",
        'P' => "ᴾ",
        'R' => "ᴿ",
        'T' => "ᵀ",
        'U' => "ᵁ",
        'V' => "ⱽ",
        'W' => "ᵂ",
        '+' => "⁺",
        '-' => "⁻",
        '=' => "⁼",
        '(' => "⁽",
        ')' => "⁾",
        _ => return None,
    })
}

/// 文字を下付き Unicode 文字列に変換する。対応しない文字は `None` を返す。
fn to_subscript_char(c: char) -> Option<&'static str> {
    Some(match c {
        '0' => "₀",
        '1' => "₁",
        '2' => "₂",
        '3' => "₃",
        '4' => "₄",
        '5' => "₅",
        '6' => "₆",
        '7' => "₇",
        '8' => "₈",
        '9' => "₉",
        'a' => "ₐ",
        'e' => "ₑ",
        'i' => "ᵢ",
        'j' => "ⱼ",
        'n' => "ₙ",
        'o' => "ₒ",
        'p' => "ₚ",
        'r' => "ᵣ",
        's' => "ₛ",
        't' => "ₜ",
        'u' => "ᵤ",
        'v' => "ᵥ",
        'x' => "ₓ",
        '+' => "₊",
        '-' => "₋",
        '=' => "₌",
        '(' => "₍",
        ')' => "₎",
        _ => return None,
    })
}

/// LaTeX コマンド名をギリシャ文字・数学記号の Unicode 文字列に変換する。
/// 未知のコマンド名に対しては空文字列を返す。
fn math_command_to_str(name: &str) -> &'static str {
    match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" => "ρ",
        "sigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Alpha" => "Α",
        "Beta" => "Β",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Epsilon" => "Ε",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "times" => "×",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "neq" | "ne" => "≠",
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "approx" => "≈",
        "equiv" => "≡",
        "propto" => "∝",
        "sqrt" => "√",
        "infty" => "∞",
        "partial" => "∂",
        "cdot" => "·",
        "ldots" => "…",
        "cdots" => "⋯",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "in" => "∈",
        "notin" => "∉",
        "subset" => "⊂",
        "supset" => "⊃",
        "cup" => "∪",
        "cap" => "∩",
        "emptyset" => "∅",
        "nabla" => "∇",
        "forall" => "∀",
        "exists" => "∃",
        "rightarrow" | "to" => "→",
        "leftarrow" | "gets" => "←",
        "Rightarrow" | "implies" => "⇒",
        "Leftrightarrow" | "iff" => "⟺",
        "langle" => "⟨",
        "rangle" => "⟩",
        _ => "",
    }
}

/// LaTeX-like 数式表記文字列を Unicode に変換して返す。
///
/// 以下の記法を処理する:
/// - `^N` / `^{...}` — 上付き文字（`N` は 1 文字、`{...}` は複数文字）
/// - `_N` / `_{...}` — 下付き文字
/// - `\name`         — ギリシャ文字・数学記号（`\alpha` → `α` など）
/// - `^{}` / `_{}` 内の `\name` も再帰的に処理する
///
/// # 引数
/// - `source` — 変換する LaTeX-like 数式文字列
///
/// # 戻り値
/// Unicode に変換された文字列
/// 上付き／下付きの写像（[`to_superscript_char`] / [`to_subscript_char`]）。
///
/// ⚠⚠ **#78 以前は `^` と `_` の処理が 69 行ずつ 2 アームに書かれていた**。
/// 2 つの差は**マーカー文字（`^` / `_`）とこの写像だけ**（正規化して diff を取ると
/// 69 行中 4 行しか違わなかった）。⇒ 引数にして 1 実装へ畳んである。
type ScriptMap = fn(char) -> Option<&'static str>;

/// `\` の直後から英字が続く限り読み、コマンド名を返す（`i` は名前の直後へ進む）。
fn read_command_name(chars: &[char], i: &mut usize) -> String {
    let start = *i;
    while *i < chars.len() && chars[*i].is_alphabetic() {
        *i += 1;
    }
    chars[start..*i].iter().collect()
}

/// 通常文字 1 つを添字化して積む。対応が無ければ **`marker` ＋ 生の文字**。
fn push_script_char(out: &mut String, c: char, marker: char, map: ScriptMap) {
    match map(c) {
        Some(s) => out.push_str(s),
        None => {
            out.push(marker);
            out.push(c);
        }
    }
}

/// `^{...}` / `_{...}`（波括弧グループ）。`i` が `{` の**次**を指している状態で呼ぶ。
fn render_script_group(
    out: &mut String,
    chars: &[char],
    i: &mut usize,
    marker: char,
    map: ScriptMap,
) {
    while *i < chars.len() && chars[*i] != '}' {
        if chars[*i] == '\\' {
            *i += 1;
            let name = read_command_name(chars, i);
            let sym = math_command_to_str(&name);
            if sym.is_empty() {
                out.push('\\');
                out.push_str(&name);
            } else {
                // ⚠ 未対応文字は**マーカーを付けずに**そのまま積む（単独形と違う）。
                for c in sym.chars() {
                    match map(c) {
                        Some(s) => out.push_str(s),
                        None => out.push(c),
                    }
                }
            }
        } else {
            let c = chars[*i];
            push_script_char(out, c, marker, map);
            *i += 1;
        }
    }
    if *i < chars.len() {
        *i += 1; // 閉じ `}` を読み飛ばす
    }
}

/// `^x` / `_x`（波括弧なしの単一要素）。`i` が対象文字を指している状態で呼ぶ。
///
/// ⚠⚠ **グループ形と 2 点だけ意図的に違う**（#78 で畳むときにそのまま保存した）:
/// ① 未知コマンドのとき `marker` を先に積む、② 既知コマンドの未対応文字の
/// フォールバックが `c` ではなく **`sym`（記号全体）**。
/// ②は多文字シンボルが入ると挙動が割れるが、`math_command_to_str` の **71 個はすべて 1 文字**
/// なので現状は観測できない（#78 で全件確認）。**畳むついでに直さない**（挙動不変が本タスクの前提）。
fn render_script_single(
    out: &mut String,
    chars: &[char],
    i: &mut usize,
    marker: char,
    map: ScriptMap,
) {
    if chars[*i] == '\\' {
        *i += 1;
        let name = read_command_name(chars, i);
        let sym = math_command_to_str(&name);
        if sym.is_empty() {
            out.push(marker);
            out.push('\\');
            out.push_str(&name);
        } else {
            for c in sym.chars() {
                out.push_str(map(c).unwrap_or(sym));
            }
        }
    } else {
        let c = chars[*i];
        push_script_char(out, c, marker, map);
        *i += 1;
    }
}

/// `^` / `_` のアーム全体。`i` が `^` / `_` を指している状態で呼ぶ。
fn render_script(out: &mut String, chars: &[char], i: &mut usize, marker: char, map: ScriptMap) {
    *i += 1;
    if *i < chars.len() && chars[*i] == '{' {
        *i += 1;
        render_script_group(out, chars, i, marker, map);
    } else if *i < chars.len() {
        render_script_single(out, chars, i, marker, map);
    }
}

pub fn render_math_str(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            // ⚠ 2 つの違いは**マーカー文字と写像だけ**（#78）。
            '^' => render_script(&mut result, &chars, &mut i, '^', to_superscript_char),
            '_' => render_script(&mut result, &chars, &mut i, '_', to_subscript_char),
            '\\' => {
                i += 1;
                let name = read_command_name(&chars, &mut i);
                let sym = math_command_to_str(&name);
                if sym.is_empty() {
                    result.push('\\');
                    result.push_str(&name);
                } else {
                    result.push_str(sym);
                }
            }
            c => {
                result.push(c);
                i += 1;
            }
        }
    }
    result
}

/// 文字を上付き Unicode 文字列に変換する。対応しない文字は `None` を返す。
fn to_superscript_char(c: char) -> Option<&'static str> {
    Some(match c {
        '0' => "⁰", '1' => "¹", '2' => "²", '3' => "³",
        '4' => "⁴", '5' => "⁵", '6' => "⁶", '7' => "⁷",
        '8' => "⁸", '9' => "⁹",
        'a' => "ᵃ", 'b' => "ᵇ", 'c' => "ᶜ", 'd' => "ᵈ",
        'e' => "ᵉ", 'f' => "ᶠ", 'g' => "ᵍ", 'h' => "ʰ",
        'i' => "ⁱ", 'j' => "ʲ", 'k' => "ᵏ", 'l' => "ˡ",
        'm' => "ᵐ", 'n' => "ⁿ", 'o' => "ᵒ", 'p' => "ᵖ",
        'r' => "ʳ", 's' => "ˢ", 't' => "ᵗ", 'u' => "ᵘ",
        'v' => "ᵛ", 'w' => "ʷ", 'x' => "ˣ", 'y' => "ʸ", 'z' => "ᶻ",
        'A' => "ᴬ", 'B' => "ᴮ", 'D' => "ᴰ", 'E' => "ᴱ", 'G' => "ᴳ",
        'H' => "ᴴ", 'I' => "ᴵ", 'J' => "ᴶ", 'K' => "ᴷ", 'L' => "ᴸ",
        'M' => "ᴹ", 'N' => "ᴺ", 'O' => "ᴼ", 'P' => "ᴾ", 'R' => "ᴿ",
        'T' => "ᵀ", 'U' => "ᵁ", 'V' => "ⱽ", 'W' => "ᵂ",
        '+' => "⁺", '-' => "⁻", '=' => "⁼", '(' => "⁽", ')' => "⁾",
        _ => return None,
    })
}

/// 文字を下付き Unicode 文字列に変換する。対応しない文字は `None` を返す。
fn to_subscript_char(c: char) -> Option<&'static str> {
    Some(match c {
        '0' => "₀", '1' => "₁", '2' => "₂", '3' => "₃",
        '4' => "₄", '5' => "₅", '6' => "₆", '7' => "₇",
        '8' => "₈", '9' => "₉",
        'a' => "ₐ", 'e' => "ₑ", 'i' => "ᵢ", 'j' => "ⱼ",
        'n' => "ₙ", 'o' => "ₒ", 'p' => "ₚ", 'r' => "ᵣ",
        's' => "ₛ", 't' => "ₜ", 'u' => "ᵤ", 'v' => "ᵥ", 'x' => "ₓ",
        '+' => "₊", '-' => "₋", '=' => "₌", '(' => "₍", ')' => "₎",
        _ => return None,
    })
}

/// LaTeX コマンド名をギリシャ文字・数学記号の Unicode 文字列に変換する。
/// 未知のコマンド名に対しては空文字列を返す。
fn math_command_to_str(name: &str) -> &'static str {
    match name {
        "alpha" => "α", "beta" => "β", "gamma" => "γ", "delta" => "δ",
        "epsilon" => "ε", "zeta" => "ζ", "eta" => "η", "theta" => "θ",
        "iota" => "ι", "kappa" => "κ", "lambda" => "λ", "mu" => "μ",
        "nu" => "ν", "xi" => "ξ", "pi" => "π", "rho" => "ρ",
        "sigma" => "σ", "tau" => "τ", "upsilon" => "υ", "phi" => "φ",
        "chi" => "χ", "psi" => "ψ", "omega" => "ω",
        "Alpha" => "Α", "Beta" => "Β", "Gamma" => "Γ", "Delta" => "Δ",
        "Epsilon" => "Ε", "Theta" => "Θ", "Lambda" => "Λ", "Pi" => "Π",
        "Sigma" => "Σ", "Phi" => "Φ", "Psi" => "Ψ", "Omega" => "Ω",
        "times" => "×", "div" => "÷", "pm" => "±", "mp" => "∓",
        "neq" | "ne" => "≠", "leq" | "le" => "≤", "geq" | "ge" => "≥",
        "approx" => "≈", "equiv" => "≡", "propto" => "∝",
        "sqrt" => "√", "infty" => "∞", "partial" => "∂",
        "cdot" => "·", "ldots" => "…", "cdots" => "⋯",
        "sum" => "∑", "prod" => "∏", "int" => "∫",
        "in" => "∈", "notin" => "∉", "subset" => "⊂", "supset" => "⊃",
        "cup" => "∪", "cap" => "∩", "emptyset" => "∅",
        "nabla" => "∇", "forall" => "∀", "exists" => "∃",
        "rightarrow" | "to" => "→", "leftarrow" | "gets" => "←",
        "Rightarrow" | "implies" => "⇒", "Leftrightarrow" | "iff" => "⟺",
        "langle" => "⟨", "rangle" => "⟩",
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
pub fn render_math_str(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '^' => {
                i += 1;
                if i < chars.len() && chars[i] == '{' {
                    i += 1;
                    while i < chars.len() && chars[i] != '}' {
                        if chars[i] == '\\' {
                            i += 1;
                            let name_start = i;
                            while i < chars.len() && chars[i].is_alphabetic() { i += 1; }
                            let name: String = chars[name_start..i].iter().collect();
                            let sym = math_command_to_str(&name);
                            if sym.is_empty() { result.push('\\'); result.push_str(&name); }
                            else { for c in sym.chars() { if let Some(s) = to_superscript_char(c) { result.push_str(s); } else { result.push(c); } } }
                        } else {
                            let c = chars[i];
                            if let Some(s) = to_superscript_char(c) { result.push_str(s); }
                            else { result.push('^'); result.push(c); }
                            i += 1;
                        }
                    }
                    if i < chars.len() { i += 1; }
                } else if i < chars.len() {
                    if chars[i] == '\\' {
                        i += 1;
                        let name_start = i;
                        while i < chars.len() && chars[i].is_alphabetic() { i += 1; }
                        let name: String = chars[name_start..i].iter().collect();
                        let sym = math_command_to_str(&name);
                        if sym.is_empty() { result.push('^'); result.push('\\'); result.push_str(&name); }
                        else { for c in sym.chars() { result.push_str(to_superscript_char(c).unwrap_or(sym)); } }
                    } else {
                        let c = chars[i];
                        if let Some(s) = to_superscript_char(c) { result.push_str(s); }
                        else { result.push('^'); result.push(c); }
                        i += 1;
                    }
                }
            }
            '_' => {
                i += 1;
                if i < chars.len() && chars[i] == '{' {
                    i += 1;
                    while i < chars.len() && chars[i] != '}' {
                        if chars[i] == '\\' {
                            i += 1;
                            let name_start = i;
                            while i < chars.len() && chars[i].is_alphabetic() { i += 1; }
                            let name: String = chars[name_start..i].iter().collect();
                            let sym = math_command_to_str(&name);
                            if sym.is_empty() { result.push('\\'); result.push_str(&name); }
                            else { for c in sym.chars() { if let Some(s) = to_subscript_char(c) { result.push_str(s); } else { result.push(c); } } }
                        } else {
                            let c = chars[i];
                            if let Some(s) = to_subscript_char(c) { result.push_str(s); }
                            else { result.push('_'); result.push(c); }
                            i += 1;
                        }
                    }
                    if i < chars.len() { i += 1; }
                } else if i < chars.len() {
                    if chars[i] == '\\' {
                        i += 1;
                        let name_start = i;
                        while i < chars.len() && chars[i].is_alphabetic() { i += 1; }
                        let name: String = chars[name_start..i].iter().collect();
                        let sym = math_command_to_str(&name);
                        if sym.is_empty() { result.push('_'); result.push('\\'); result.push_str(&name); }
                        else { for c in sym.chars() { result.push_str(to_subscript_char(c).unwrap_or(sym)); } }
                    } else {
                        let c = chars[i];
                        if let Some(s) = to_subscript_char(c) { result.push_str(s); }
                        else { result.push('_'); result.push(c); }
                        i += 1;
                    }
                }
            }
            '\\' => {
                i += 1;
                let name_start = i;
                while i < chars.len() && chars[i].is_alphabetic() {
                    i += 1;
                }
                let name: String = chars[name_start..i].iter().collect();
                let sym = math_command_to_str(&name);
                if sym.is_empty() { result.push('\\'); result.push_str(&name); }
                else { result.push_str(sym); }
            }
            c => { result.push(c); i += 1; }
        }
    }
    result
}

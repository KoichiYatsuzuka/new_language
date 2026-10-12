// ops/format_spec.rs — 書式指定の小言語（`format(v, spec)` / `str.format` の `{:spec}` / f-string の `{x:spec}`・
// python_builtins_plan.md のタスク 3-4）。
//
// CPython 3.12 の `Python/formatter_unicode.c` と同じ規則・同じ文言:
//   format_spec ::= [[fill]align][sign]["z"]["#"]["0"][width][grouping]["." precision][type]
// 入口は `Interpreter::format_value`（`eval/py_builtins.rs`）。インスタンスの `__format__` はそちらで呼ぶ。
// ⚠ 解析の順序と、どの誤りを先に出すかも CPython と同じにしてある（`parse` → 型ごとの検査）。

/// 桁区切り（`,` / `_`）。`_` は 2 / 8 / 16 進では 4 桁ごと（PEP 515）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Grouping {
    None,
    Comma,
    Underscore,
    /// `_` を 2 / 8 / 16 進に付けた形（4 桁ごと）。
    UnderscoreFour,
}

impl Grouping {
    fn sep(self) -> Option<(char, usize)> {
        match self {
            Grouping::None => None,
            Grouping::Comma => Some((',', 3)),
            Grouping::Underscore => Some(('_', 3)),
            Grouping::UnderscoreFour => Some(('_', 4)),
        }
    }

    fn ch(self) -> char {
        match self {
            Grouping::Comma => ',',
            _ => '_',
        }
    }
}

/// 解析した書式指定。`align` と `ty` は既定値（数は `>`・文字列は `<`／int は `d`・str は `s`）を埋めてある。
#[derive(Debug, Clone)]
pub(crate) struct FormatSpec {
    pub fill: char,
    pub align: char,
    pub sign: Option<char>,
    pub no_neg_0: bool,
    pub alternate: bool,
    pub width: usize,
    pub grouping: Grouping,
    pub precision: Option<usize>,
    /// `None` は型の文字を書かなかった float（repr か、精度があれば `g` の変形）。
    pub ty: Option<char>,
}

/// `spec` を解析する（CPython の `parse_internal_render_format_spec`）。`type_name` は誤りの文面に入れる型の名前。
pub(crate) fn parse(spec: &str, type_name: &str, default_type: Option<char>, default_align: char) -> Result<FormatSpec, String> {
    let c: Vec<char> = spec.chars().collect();
    let end = c.len();
    let mut pos = 0;
    let is_align = |ch: char| matches!(ch, '<' | '>' | '=' | '^');
    let mut fill = ' ';
    let mut fill_given = false;
    let mut align = default_align;
    let mut align_given = false;
    if end - pos >= 2 && is_align(c[pos + 1]) {
        fill = c[pos];
        align = c[pos + 1];
        fill_given = true;
        align_given = true;
        pos += 2;
    } else if end - pos >= 1 && is_align(c[pos]) {
        align = c[pos];
        align_given = true;
        pos += 1;
    }
    let mut sign = None;
    if pos < end && matches!(c[pos], '+' | '-' | ' ') {
        sign = Some(c[pos]);
        pos += 1;
    }
    let mut no_neg_0 = false;
    if pos < end && c[pos] == 'z' {
        no_neg_0 = true;
        pos += 1;
    }
    let mut alternate = false;
    if pos < end && c[pos] == '#' {
        alternate = true;
        pos += 1;
    }
    if !fill_given && pos < end && c[pos] == '0' {
        fill = '0';
        if !align_given && default_align == '>' {
            align = '=';
        }
        pos += 1;
    }
    let width = read_integer(&c, &mut pos)?.unwrap_or(0);
    let mut grouping = Grouping::None;
    if pos < end && c[pos] == ',' {
        grouping = Grouping::Comma;
        pos += 1;
    }
    if pos < end && c[pos] == '_' {
        if grouping != Grouping::None {
            return Err("ValueError: Cannot specify both ',' and '_'.".to_string());
        }
        grouping = Grouping::Underscore;
        pos += 1;
    }
    if pos < end && c[pos] == ',' && grouping == Grouping::Underscore {
        return Err("ValueError: Cannot specify both ',' and '_'.".to_string());
    }
    let mut precision = None;
    if pos < end && c[pos] == '.' {
        pos += 1;
        precision = read_integer(&c, &mut pos)?;
        if precision.is_none() {
            return Err("ValueError: Format specifier missing precision".to_string());
        }
    }
    if end - pos > 1 {
        return Err(format!("ValueError: Invalid format specifier '{spec}' for object of type '{type_name}'"));
    }
    let ty = if end - pos == 1 { Some(c[pos]) } else { default_type };
    if grouping != Grouping::None {
        match ty {
            None | Some('d' | 'e' | 'f' | 'g' | 'E' | 'G' | '%' | 'F') => {}
            Some('b' | 'o' | 'x' | 'X') if grouping == Grouping::Underscore => grouping = Grouping::UnderscoreFour,
            Some(t) => return Err(format!("ValueError: Cannot specify '{}' with '{}'.", grouping.ch(), show_code(t))),
        }
    }
    Ok(FormatSpec { fill, align, sign, no_neg_0, alternate, width, grouping, precision, ty })
}

/// 幅・精度の整数。桁が無ければ `None`。`isize` に収まらなければ CPython と同じ誤り。
fn read_integer(c: &[char], pos: &mut usize) -> Result<Option<usize>, String> {
    let start = *pos;
    while *pos < c.len() && c[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if start == *pos {
        return Ok(None);
    }
    let s: String = c[start..*pos].iter().collect();
    s.parse::<isize>()
        .map(|n| Some(n as usize))
        .map_err(|_| "ValueError: Too many decimal digits in format string".to_string())
}

/// 誤りの文面に出す型の文字（印字できない文字は `\x..`）。
fn show_code(t: char) -> String {
    if (t as u32) > 32 && (t as u32) < 128 { t.to_string() } else { format!("\\x{:x}", t as u32) }
}

fn unknown_code(t: char, type_name: &str) -> String {
    format!("ValueError: Unknown format code '{}' for object of type '{type_name}'", show_code(t))
}

/// 整数（`int` / `bool`）。float の型の文字（`e` / `f` / `g` / `%` …）は float にして書く（CPython と同じ）。
pub(crate) fn format_int(n: i128, spec: &FormatSpec, type_name: &str) -> Result<String, String> {
    let ty = spec.ty.unwrap_or('d');
    match ty {
        'b' | 'c' | 'd' | 'o' | 'x' | 'X' | 'n' => {}
        'e' | 'E' | 'f' | 'F' | 'g' | 'G' | '%' => return format_float(n as f64, spec, type_name),
        other => return Err(unknown_code(other, type_name)),
    }
    if spec.precision.is_some() {
        return Err("ValueError: Precision not allowed in integer format specifier".to_string());
    }
    if spec.no_neg_0 {
        return Err("ValueError: Negative zero coercion (z) not allowed in integer format specifier".to_string());
    }
    if ty == 'c' {
        if spec.sign.is_some() {
            return Err("ValueError: Sign not allowed with integer format specifier 'c'".to_string());
        }
        if spec.alternate {
            return Err("ValueError: Alternate form (#) not allowed with integer format specifier 'c'".to_string());
        }
        let ch = u32::try_from(n)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| "OverflowError: %c arg not in range(0x110000)".to_string())?;
        return Ok(fill_number("", "", &ch.to_string(), "", spec));
    }
    let neg = n < 0;
    let m = n.unsigned_abs();
    let (digits, prefix) = match ty {
        'b' => (format!("{m:b}"), "0b"),
        'o' => (format!("{m:o}"), "0o"),
        'x' => (format!("{m:x}"), "0x"),
        'X' => (format!("{m:X}"), "0X"),
        _ => (m.to_string(), ""),
    };
    let prefix = if spec.alternate { prefix } else { "" };
    Ok(fill_number(sign_str(neg, spec.sign), prefix, &digits, "", spec))
}

/// 浮動小数点数（CPython の `format_float_internal`）。
pub(crate) fn format_float(x: f64, spec: &FormatSpec, type_name: &str) -> Result<String, String> {
    let mut ty = spec.ty;
    match ty {
        None | Some('e' | 'E' | 'f' | 'F' | 'g' | 'G' | 'n' | '%') => {}
        Some(other) => return Err(unknown_code(other, type_name)),
    }
    let mut val = x;
    let mut percent = false;
    if ty == Some('n') {
        ty = Some('g');
    }
    if ty == Some('%') {
        val *= 100.0;
        ty = Some('f');
        percent = true;
    }
    let upper = matches!(ty, Some('E' | 'F' | 'G'));
    let nan = val.is_nan();
    let mut neg = !nan && val.is_sign_negative();
    let abs = val.abs();
    let mut body = if nan || abs.is_infinite() {
        let s = if nan { "nan" } else { "inf" };
        if upper { s.to_uppercase() } else { s.to_string() }
    } else {
        match (ty, spec.precision) {
            // 型の文字も精度も無い float は repr と同じ。
            (None, None) => super::py_float_repr(abs),
            // 精度だけ: `g` の変形（固定小数点なら少なくとも 1 桁の小数を付ける）。
            (None, Some(p)) => float_general(abs, p, false, spec.alternate, true),
            (Some('f' | 'F'), p) => float_fixed(abs, p.unwrap_or(6), spec.alternate),
            (Some('e' | 'E'), p) => float_exp(abs, p.unwrap_or(6), upper, spec.alternate),
            (Some(_), p) => float_general(abs, p.unwrap_or(6), upper, spec.alternate, false),
        }
    };
    // `z`: 丸めた結果が 0 なら負の符号を付けない。
    if neg && spec.no_neg_0 && !body.bytes().any(|b| b.is_ascii_digit() && b != b'0') && !nan && abs.is_finite() {
        neg = false;
    }
    if percent {
        body.push('%');
    }
    let int_len = body.bytes().take_while(|b| b.is_ascii_digit()).count();
    let (digits, rest) = body.split_at(int_len);
    Ok(fill_number(sign_str(neg, spec.sign), "", digits, rest, spec))
}

/// 文字列（CPython の `format_string_internal`）。
pub(crate) fn format_str(s: &str, spec: &FormatSpec, type_name: &str) -> Result<String, String> {
    match spec.ty {
        Some('s') => {}
        Some(other) => return Err(unknown_code(other, type_name)),
        None => {}
    }
    if spec.sign.is_some() {
        return Err("ValueError: Sign not allowed in string format specifier".to_string());
    }
    if spec.no_neg_0 {
        return Err("ValueError: Negative zero coercion (z) not allowed in string format specifier".to_string());
    }
    if spec.alternate {
        return Err("ValueError: Alternate form (#) not allowed in string format specifier".to_string());
    }
    if spec.align == '=' {
        return Err("ValueError: '=' alignment not allowed in string format specifier".to_string());
    }
    let body: String = match spec.precision {
        Some(p) => s.chars().take(p).collect(),
        None => s.to_string(),
    };
    Ok(pad(&body, "", spec))
}

fn sign_str(neg: bool, sign: Option<char>) -> &'static str {
    match (neg, sign) {
        (true, _) => "-",
        (false, Some('+')) => "+",
        (false, Some(' ')) => " ",
        _ => "",
    }
}

/// `{:.pf}`（Rust の書式は CPython と同じく正確な値の最近接偶数丸め）。`#` で精度 0 なら小数点を残す。
pub(crate) fn float_fixed(x: f64, p: usize, alternate: bool) -> String {
    let mut s = format!("{x:.p$}");
    if alternate && p == 0 {
        s.push('.');
    }
    s
}

/// `{:.pe}` を CPython の綴りにする（指数は符号つき 2 桁以上: `1.5e+04` / `1e-07`）。
pub(crate) fn float_exp(x: f64, p: usize, upper: bool, alternate: bool) -> String {
    let s = format!("{x:.p$e}");
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let mut mant = mant.to_string();
    if alternate && p == 0 {
        mant.push('.');
    }
    let e = if upper { 'E' } else { 'e' };
    let sign = if exp < 0 { '-' } else { '+' };
    format!("{mant}{e}{sign}{:02}", exp.abs())
}

/// `g`（有効数字 `p` 桁。指数が -4 以上 `p` 未満なら固定小数点、それ以外は指数表記。`#` が無ければ末尾の 0 を消す）。
/// `add_dot_0` は型の文字の無い float の精度つき（固定小数点で小数点が無ければ `.0` を付ける）。
pub(crate) fn float_general(x: f64, p: usize, upper: bool, alternate: bool, add_dot_0: bool) -> String {
    let p = p.max(1);
    let exp: i32 = if x == 0.0 {
        0
    } else {
        let s = format!("{x:.*e}", p - 1);
        s.split_once('e').and_then(|(_, e)| e.parse().ok()).unwrap_or(0)
    };
    let mut s = if -4 <= exp && (exp as i64) < p as i64 {
        let decimals = (p as i64 - 1 - exp as i64).max(0) as usize;
        let mut s = format!("{x:.decimals$}");
        if !alternate && s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        if alternate && !s.contains('.') {
            s.push('.');
        }
        if add_dot_0 && !s.contains('.') {
            s.push_str(".0");
        }
        s
    } else {
        let s = float_exp(x, p - 1, upper, alternate);
        if alternate {
            s
        } else {
            let (mant, rest) = s.split_at(s.find(['e', 'E']).unwrap_or(s.len()));
            let mant = if mant.contains('.') { mant.trim_end_matches('0').trim_end_matches('.') } else { mant };
            format!("{mant}{rest}")
        }
    };
    if upper {
        s = s.to_uppercase();
    }
    s
}

/// 符号・接頭辞（`0x`）・整数部の数字・残り（小数部・指数・`%`）を並べて幅に詰める（CPython の `fill_number`）。
/// 桁区切りは整数部に入れる。`0` で詰める `=` の形は、詰める 0 にも桁区切りを入れる（`format(1234, "010,")` は `00,001,234`）。
fn fill_number(sign: &str, prefix: &str, digits: &str, rest: &str, spec: &FormatSpec) -> String {
    let grouped = match spec.grouping.sep() {
        Some((sep, size)) => {
            let min_width = if spec.fill == '0' && spec.align == '=' {
                spec.width.saturating_sub(sign.len() + prefix.len() + rest.chars().count())
            } else {
                0
            };
            group_digits(digits, sep, size, min_width)
        }
        None => digits.to_string(),
    };
    pad(&format!("{grouped}{rest}"), &format!("{sign}{prefix}"), spec)
}

/// 桁区切りを右から入れる。`min_width` まで 0 で伸ばす（CPython の `_PyUnicode_InsertThousandsGrouping`。
/// 区切りが先頭に来るときは 0 を 1 つ足して止める）。
fn group_digits(digits: &str, sep: char, size: usize, min_width: usize) -> String {
    let d: Vec<char> = digits.chars().collect();
    let mut remaining = d.len() as isize;
    let mut min_width = min_width as isize;
    let mut groups: Vec<String> = Vec::new();
    let mut end = d.len();
    loop {
        let len = (size as isize).min(remaining.max(min_width).max(1));
        let n_chars = remaining.min(len).max(0);
        let n_zeros = (len - remaining).max(0);
        let start = end - n_chars as usize;
        let mut g: String = "0".repeat(n_zeros as usize);
        g.extend(&d[start..end]);
        groups.push(g);
        end = start;
        remaining -= n_chars;
        min_width -= len;
        if remaining <= 0 && min_width <= 0 {
            break;
        }
        min_width -= 1;
    }
    groups.reverse();
    groups.join(&sep.to_string())
}

/// 幅まで `fill` で詰める。`=` は符号・接頭辞（`lead`）と本体の間に詰める。幅は文字の数。
fn pad(body: &str, lead: &str, spec: &FormatSpec) -> String {
    let n = lead.chars().count() + body.chars().count();
    if spec.width <= n {
        return format!("{lead}{body}");
    }
    let total = spec.width - n;
    let fill = |k: usize| spec.fill.to_string().repeat(k);
    match spec.align {
        '<' => format!("{lead}{body}{}", fill(total)),
        '^' => format!("{}{lead}{body}{}", fill(total / 2), fill(total - total / 2)),
        '=' => format!("{lead}{}{body}", fill(total)),
        _ => format!("{}{lead}{body}", fill(total)),
    }
}

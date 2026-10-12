//! ffi_probe_rs — `import[rs]` で値の受け渡しを検査するための crate
//! （implementation_plans/ffi_value_exchange_plan.md のタスク 1-1）。
//!
//! 今の実装で正しく受け渡せないもの（計画書の §0.2）を含む。例題は直すタスク（2-5）で足す。
//! ⚠ `Person`（`pub String` フィールド）があるため、2-5 の前はこの crate の import 自体が失敗する。

/// 数値フィールドだけの構造体（今の実装で import できる形）。
pub struct Counter {
    pub n: i64,
}

impl Counter {
    pub fn label(&self) -> String {
        format!("count={}", self.n)
    }
    /// ネイティブ側でフィールドを書き換える（今は Arrow 側の `c.n` が古い値のまま）。
    pub fn add_len(&mut self, s: &str) {
        self.n += s.len() as i64;
    }
    pub fn plus(&self, by: i64) -> Counter {
        Counter { n: self.n + by }
    }
}

/// `pub String` フィールドを持つ（今はラッパーのセッターがコンパイルエラーになる）。
pub struct Person {
    pub name: String,
    pub age: i32,
}

impl Person {
    pub fn new(name: String, age: i32) -> Self {
        Person { name, age }
    }
    pub fn hello(&self) -> String {
        format!("I am {} ({})", self.name, self.age)
    }
}

/// private フィールドを持つ（今は `new()` が使われず、構造体リテラルでの構築がコンパイルエラーになる）。
pub struct Account {
    id: i64,
    pub balance: i64,
}

impl Account {
    pub fn new(id: i64, balance: i64) -> Self {
        Account { id, balance }
    }
    pub fn id(&self) -> i64 {
        self.id
    }
}

pub fn greet() -> String {
    "hello from Rust".to_string()
}
pub fn echo(s: &str) -> String {
    s.to_string()
}
pub fn utf8() -> String {
    "日本語".to_string()
}
pub fn byte_len(s: &str) -> usize {
    s.len()
}
/// 今は 16 進文字列（"6869"）で返る。
pub fn bytes_out() -> Vec<u8> {
    vec![104, 105]
}

// ── 今の実装が対応していないシグネチャ（import 時に黙って消える） ──────────────
pub fn words(s: &str) -> Vec<String> {
    s.split(' ').map(|w| w.to_string()).collect()
}
pub fn maybe(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.to_string()) }
}
pub fn sum(xs: &[i64]) -> i64 {
    xs.iter().sum()
}
pub fn pair() -> (i32, String) {
    (1, "one".to_string())
}

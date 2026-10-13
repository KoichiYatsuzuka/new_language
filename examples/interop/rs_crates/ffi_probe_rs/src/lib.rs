//! ffi_probe_rs — `import[rs]` で値の受け渡しを検査するための crate
//! （implementation_plans/ffi_value_exchange_plan.md のタスク 1-1・2-5）。
//!
//! 2-5 より前は `Person`（`pub String` フィールド）と `Account`（private フィールド）のせいで中継 DLL のビルドが
//! 失敗し、この crate の import 自体ができなかった。今の Arrow から呼べない関数・メソッドは、import 時に消さずに
//! 型スタブに載せ、使った箇所を静的エラーにする（rs_unsupported_signature_error.ar）。

/// 数値フィールドだけの構造体（`new` が無いので構造体リテラルで作る）。
pub struct Counter {
    pub n: i64,
}

impl Counter {
    pub fn label(&self) -> String {
        format!("count={}", self.n)
    }
    /// ネイティブ側でフィールドを書き換える（2-5 より前は Arrow 側の `c.n` が古い値のままだった）。
    pub fn add_len(&mut self, s: &str) {
        self.n += s.len() as i64;
    }
    pub fn plus(&self, by: i64) -> Counter {
        Counter { n: self.n + by }
    }
    /// `-> Self`（2-5 より前は黙って消えていた）。
    pub fn doubled(&self) -> Self {
        Counter { n: self.n * 2 }
    }
    /// Arrow から構築できない構造体を返す（返ってきた値は使える）。
    pub fn seal(&self) -> Sealed {
        Sealed { v: self.n }
    }
    /// `self` の無い関連関数（`new` 以外）: まだ呼べない。
    pub fn zero() -> Self {
        Counter { n: 0 }
    }
    /// `self` を値で受ける（消費する）: まだ呼べない。
    pub fn into_n(self) -> i64 {
        self.n
    }
}

/// `pub String` フィールドを持つ（2-5 より前はラッパーのセッターがコンパイルエラーになった）。
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

/// private フィールドを持つ（`new()` で作る。2-5 より前は `new()` が使われず、構造体リテラルがコンパイルエラーになった）。
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
    pub fn deposit(&mut self, amount: i64) {
        self.balance += amount;
    }
}

/// `pub` フィールドが 1 つも無い（2-5 より前は型ごと黙って消えていた）。
pub struct Secret {
    code: i64,
}

impl Secret {
    pub fn new(code: i64) -> Self {
        Secret { code }
    }
    pub fn reveal(&self) -> i64 {
        self.code
    }
}

/// private フィールドで `new` も無い: Arrow からは作れない（`Counter::seal` が返す値は使える）。
pub struct Sealed {
    v: i64,
}

impl Sealed {
    pub fn value(&self) -> i64 {
        self.v
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
/// 今は 16 進文字列（"6869"）で返る（bytes にするのは 5-4）。
pub fn bytes_out() -> Vec<u8> {
    vec![104, 105]
}
/// rustfmt が折り返したシグネチャ（2-5 より前は黙って消えていた）。
pub fn add_three(
    a: i64,
    b: i64,
    c: i64,
) -> i64 {
    a + b + c
}

// ── 整数の範囲（2-5: 以前は `as` で黙って折り返した）──────────────────────────
pub fn add_u8(a: u8, b: u8) -> u8 {
    a.wrapping_add(b)
}
pub fn small_u64() -> u64 {
    7
}
pub fn huge_u64() -> u64 {
    u64::MAX
}

// ── 今の Arrow から呼べないシグネチャ（印付きで型スタブに載り、使うと静的エラー）──────
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
pub fn count_bytes(v: Vec<u8>) -> usize {
    v.len()
}
pub fn ident<T>(x: T) -> T {
    x
}
pub fn bump(n: &mut i64) {
    *n += 1;
}

// ── 引数の検査（タスク 7-12）: count_* の呼び出しを数えて、誤った引数で呼ばれていないかを確かめる ──
static CALLS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

fn counted() {
    CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}
pub fn count_i64(v: i64) -> i64 {
    counted();
    v
}
pub fn count_flag(b: bool) -> i64 {
    counted();
    b as i64
}
pub fn count_f64(d: f64) -> i64 {
    counted();
    d as i64
}
pub fn count_text(s: &str) -> i64 {
    counted();
    s.len() as i64
}
pub fn call_count() -> i64 {
    CALLS.load(std::sync::atomic::Ordering::SeqCst)
}

/// 文字アクセスメソッド群。
///
/// `Lexer` の現在読み取り位置（`pos`）周辺の文字を参照・消費する低レベルヘルパー。
/// すべてのメソッドは `scan.rs` で定義された `Lexer` に対する `impl` ブロックとして提供する。
use super::scan::Lexer;

impl Lexer {
    /// 現在位置の文字を返す（消費しない）。
    ///
    /// # 戻り値
    /// `Some(char)` または入力終端の場合 `None`
    pub(super) fn ch(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// 現在位置の 1 つ先の文字を返す（消費しない）。
    ///
    /// # 戻り値
    /// `Some(char)` または範囲外の場合 `None`
    pub(super) fn ch1(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    /// 現在位置の 2 つ先の文字を返す（消費しない）。
    ///
    /// トリプルクォート終端の判定などで使用する。
    ///
    /// # 戻り値
    /// `Some(char)` または範囲外の場合 `None`
    pub(super) fn ch2(&self) -> Option<char> {
        self.chars.get(self.pos + 2).copied()
    }

    /// 現在位置の文字を返しつつ `pos` を 1 進める。
    ///
    /// # 戻り値
    /// `Some(char)` または入力終端の場合 `None`（`pos` は変化しない）
    pub(super) fn bump(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }
}

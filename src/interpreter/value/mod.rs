// value/mod.rs — インタープリタが扱う実行時値の型定義サブシステムの束ね。
// 役割別サブモジュールを宣言し、全公開型を再エクスポートする（`interpreter` からは
// `pub use value::*` で従来どおり参照可能）。

mod core;
mod exceptions;
mod callables;
mod instance;
mod collections;
mod objects;
mod native;
mod flat;
pub use core::*;
pub use exceptions::*;
pub use callables::*;
pub use instance::*;
pub use collections::*;
pub use objects::*;
pub use native::*;
pub use flat::*;

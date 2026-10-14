// proc_values.rs — 別プロセスのブリッジ（cs-proc・js-proc）の値の表現: encode と、コンテナを組み立てる関数
// （ffi_value_exchange_plan.md 5-2・D-5）。
//
// 別プロセスではアドレスに意味が無いので、中身をコピーして JSON で運ぶ。タグ（両ブリッジ共通）:
//   "i" = int64   "f" = float64（NaN / ±Infinity は文字列 "NaN" / "Infinity" / "-Infinity"）   "b" = bool
//   "s" = string  "n" = null    "a" = list   "u" = tuple   "d" = dict（[[キー, 値], …]）   "e" = set
//   "y" = bytes（base64）   "h" = C# のオブジェクトのハンドル（cs-proc だけ）
// 受け取りだけのタグ（"o" / "x" など）は各ブリッジの decode が読む。
//
// ⚠ 以前は数値・文字列・bool・None・リスト（js-proc だけ）しか運べず、dict / tuple / set / bytes は TypeError
//   （2-1・2-3 で黙って null にしていたのを止めた段階）だった。`Ptr` は別プロセスでは意味が無いので TypeError。

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use super::value::{DictData, TupleData};
use super::Value;

/// 値を送る相手（運べるものが少し違う）。
pub(crate) enum Peer<'a> {
    /// cs-proc のホスト（そのホストの `CsObject` はハンドルで送れる）
    Cs(&'a Path),
    /// cs-dll の DLL（list を JSON で渡す・5-5。その DLL の `CsObject` はハンドルで送れる）
    CsDll(&'a Path),
    Js,
}

/// 送れない値。
#[derive(Debug, PartialEq)]
pub(crate) enum EncodeError {
    /// 運べない種類（中身は実行時の型名）
    Unsupported(String),
    /// `Ptr`（別プロセスでは意味が無い・D-5）
    Pointer,
    /// 別のブリッジの C# のオブジェクト（中身はそのブリッジのパス）
    ForeignObject(String),
}

impl EncodeError {
    /// 引数 `index`（0 始まり）のエラーの文。`what` は `C# method 'P.m'` / `JS function 'f'` の形。
    pub(crate) fn message(&self, index: usize, what: &str, bridge: &str) -> String {
        let n = index + 1;
        match self {
            EncodeError::Unsupported(ty) => format!(
                "TypeError: argument {n} of {what} is (or contains) a '{ty}', which cannot cross the {bridge} bridge"
            ),
            EncodeError::Pointer => format!(
                "TypeError: argument {n} of {what} is (or contains) a Ptr, which has no meaning in another process \
                 (copy what it points to with to_list() / to_bytes())"
            ),
            EncodeError::ForeignObject(path) => {
                format!("TypeError: argument {n} of {what} is a C# object from another bridge ('{path}')")
            }
        }
    }
}

/// 値を JSON にする（入れ子もコピーする）。
pub(crate) fn encode(v: &Value, peer: &Peer) -> Result<serde_json::Value, EncodeError> {
    let all = |items: &[Value]| items.iter().map(|x| encode(x, peer)).collect::<Result<Vec<_>, _>>();
    Ok(match v {
        Value::Int(n) => serde_json::json!({"t": "i", "v": n}),
        Value::UInt(n) => serde_json::json!({"t": "i", "v": n}),
        Value::Float(f) if f.is_finite() => serde_json::json!({"t": "f", "v": f}),
        // JSON に NaN / ±Infinity は無い（serde_json は null にしてしまう）ので文字列で送る
        Value::Float(f) => serde_json::json!({"t": "f", "v": non_finite_name(*f)}),
        Value::Bool(b) => serde_json::json!({"t": "b", "v": b}),
        Value::Str(s) => serde_json::json!({"t": "s", "v": &**s}),
        Value::None => serde_json::json!({"t": "n"}),
        Value::List(rc) => serde_json::json!({"t": "a", "v": all(&rc.borrow())?}),
        Value::Tuple(td) => serde_json::json!({"t": "u", "v": all(td.all_values())?}),
        Value::Set(rc) => serde_json::json!({"t": "e", "v": all(&rc.borrow())?}),
        Value::Dict(rc) => {
            let pairs = rc
                .borrow()
                .all_pairs()
                .iter()
                .map(|(k, x)| Ok(serde_json::Value::Array(vec![encode(k, peer)?, encode(x, peer)?])))
                .collect::<Result<Vec<_>, EncodeError>>()?;
            serde_json::json!({"t": "d", "v": pairs})
        }
        Value::Bytes(b) => serde_json::json!({"t": "y", "v": base64_encode(b)}),
        Value::CsObject(o) => match peer {
            Peer::Cs(path) if o.is_proc && same_path(&o.bridge_path, path) => {
                serde_json::json!({"t": "h", "v": o.handle})
            }
            Peer::CsDll(path) if !o.is_proc && same_path(&o.bridge_path, path) => {
                serde_json::json!({"t": "h", "v": o.handle})
            }
            _ => return Err(EncodeError::ForeignObject(o.bridge_path.display().to_string())),
        },
        Value::Ptr(_) => return Err(EncodeError::Pointer),
        other => return Err(EncodeError::Unsupported(crate::interpreter::ops::typecheck::element_type_name(other))),
    })
}

fn same_path(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

fn non_finite_name(f: f64) -> &'static str {
    if f.is_nan() {
        "NaN"
    } else if f > 0.0 {
        "Infinity"
    } else {
        "-Infinity"
    }
}

/// `"f"` の中身（数か "NaN" / "Infinity" / "-Infinity"）。
pub(crate) fn decode_float(v: Option<&serde_json::Value>) -> Option<f64> {
    match v? {
        serde_json::Value::String(s) => match s.as_str() {
            "NaN" => Some(f64::NAN),
            "Infinity" => Some(f64::INFINITY),
            "-Infinity" => Some(f64::NEG_INFINITY),
            _ => None,
        },
        n => n.as_f64(),
    }
}

/// 受け取った要素から tuple を作る。
pub(crate) fn make_tuple(items: Vec<Value>) -> Value {
    let types = items.iter().map(crate::interpreter::ops::typecheck::element_type_name).collect();
    Value::Tuple(Rc::new(TupleData::new(items, types)))
}

/// 受け取ったキーと値の組から dict を作る。キーにできない値（None / NaN …）は TypeError。
pub(crate) fn make_dict(pairs: Vec<(Value, Value)>) -> Result<Value, String> {
    let mut dict = DictData::new("Any".to_string(), "Any".to_string());
    for (k, v) in pairs {
        crate::interpreter::Interpreter::dict_set_pure(&mut dict, k, v)?;
    }
    Ok(Value::Dict(Rc::new(RefCell::new(dict))))
}

/// 受け取った要素から set を作る（等しい要素は 1 つにする）。set に入れられない値は TypeError。
pub(crate) fn make_set(items: Vec<Value>) -> Result<Value, String> {
    let mut out: Vec<Value> = Vec::new();
    for v in items {
        if let Some(why) = DictData::reject_key(&v) {
            return Err(why.replace("dict key", "set element"));
        }
        let mut found = false;
        for e in &out {
            if crate::interpreter::Interpreter::values_eq_pure(e, &v)? {
                found = true;
                break;
            }
        }
        if !found {
            out.push(v);
        }
    }
    Ok(Value::Set(Rc::new(RefCell::new(out))))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// base64（標準の表・`=` で埋める）。
pub(crate) fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// base64 を読む。⚠ 表に無い文字・長さの誤りは黙って読み飛ばさずにエラー。
pub(crate) fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let bytes = s.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(format!("malformed base64 (length {})", bytes.len()));
    }
    let val = |c: u8| -> Result<u32, String> {
        B64.iter().position(|&x| x == c).map(|p| p as u32).ok_or_else(|| format!("malformed base64 (byte {c:#04x})"))
    };
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (ci, chunk) in bytes.chunks(4).enumerate() {
        let last = ci + 1 == bytes.len() / 4;
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return Err("malformed base64 (padding)".to_string());
        }
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let d = if i < 4 - pad { val(c)? } else { 0 };
            n |= d << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0u8, 255, 128, 7]] {
            let s = base64_encode(data);
            assert_eq!(base64_decode(&s).unwrap(), data, "{s}");
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert!(base64_decode("Zm9").is_err(), "length");
        assert!(base64_decode("Zm9!").is_err(), "alphabet");
        assert!(base64_decode("Zg==Zm9v").is_err(), "padding in the middle");
    }

    /// コンテナは中身ごとコピーし、Ptr は別プロセスでは意味が無いので送らない（5-2・D-5）。
    #[test]
    fn encodes_containers_and_rejects_pointers() {
        let list = Value::List(Rc::new(RefCell::new(vec![Value::Int(1), Value::Float(f64::NAN)])));
        let j = encode(&list, &Peer::Js).unwrap();
        assert_eq!(j, serde_json::json!({"t": "a", "v": [{"t": "i", "v": 1}, {"t": "f", "v": "NaN"}]}));
        let t = make_tuple(vec![Value::Int(1), Value::str("x")]);
        assert_eq!(encode(&t, &Peer::Js).unwrap()["t"], "u");
        let d = make_dict(vec![(Value::str("k"), Value::Int(2))]).unwrap();
        assert_eq!(
            encode(&d, &Peer::Js).unwrap(),
            serde_json::json!({"t": "d", "v": [[{"t": "s", "v": "k"}, {"t": "i", "v": 2}]]})
        );
        let s = make_set(vec![Value::Int(1), Value::Int(1), Value::Int(2)]).unwrap();
        assert_eq!(encode(&s, &Peer::Js).unwrap()["v"].as_array().unwrap().len(), 2, "equal elements are merged");
        assert_eq!(encode(&Value::bytes(b"hi".to_vec()), &Peer::Js).unwrap(), serde_json::json!({"t": "y", "v": "aGk="}));
        assert!(make_dict(vec![(Value::None, Value::Int(1))]).is_err(), "None cannot be a key");
        let e = EncodeError::Pointer.message(0, "JS function 'f'", "js-proc");
        assert!(e.starts_with("TypeError: argument 1 of JS function 'f' is (or contains) a Ptr"), "{e}");
    }
}

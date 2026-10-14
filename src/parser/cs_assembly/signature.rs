// cs_assembly/signature.rs — CIL 型シグネチャ blob のデコード関数: SigReader のメソッドと C#→Arrow 型マッピング。

use super::*;


impl<'a> SigReader<'a> {
    pub(crate) fn peek(&self) -> u8 {
        if self.pos < self.data.len() { self.data[self.pos] } else { 0 }
    }

    pub(crate) fn eat(&mut self) -> u8 {
        let b = self.peek();
        self.pos += 1;
        b
    }

    pub(crate) fn eat_uint(&mut self) -> u32 {
        let (v, n) = decompress_uint(self.data, self.pos);
        self.pos += n;
        v
    }

    // Skip custom modifiers (CMOD_REQD / CMOD_OPT + compressed token)
    pub(crate) fn skip_cmods(&mut self) {
        while self.peek() == ET_CMOD_REQD || self.peek() == ET_CMOD_OPT {
            self.eat();
            self.eat_uint(); // TypeDefOrRef token
        }
    }

    /// 型を 1 つ読み、Arrow の型の綴りで返す（[`CsTy::arrow`]）。
    pub(crate) fn parse_type(&mut self) -> String {
        self.parse_ty().arrow()
    }

    /// 型を 1 つ読み、C# の型のまま返す（5-1）。型スタブ（[`CsTy::arrow`]）と export の生成器（`export_gen`）が
    /// 同じデコード結果を読む（⚠ 型の読み方を 2 つ持つと、スタブと export の型がずれる）。
    pub(crate) fn parse_ty(&mut self) -> CsTy {
        self.skip_cmods();
        match self.eat() {
            ET_VOID    => CsTy::Void,
            ET_BOOLEAN => CsTy::Bool,
            ET_CHAR    => CsTy::Char,
            ET_I1 => CsTy::Int { bits: 8, signed: true },
            ET_U1 => CsTy::Int { bits: 8, signed: false },
            ET_I2 => CsTy::Int { bits: 16, signed: true },
            ET_U2 => CsTy::Int { bits: 16, signed: false },
            ET_I4 => CsTy::Int { bits: 32, signed: true },
            ET_U4 => CsTy::Int { bits: 32, signed: false },
            ET_I8 => CsTy::Int { bits: 64, signed: true },
            ET_U8 => CsTy::Int { bits: 64, signed: false },
            ET_I => CsTy::NInt { signed: true },
            ET_U => CsTy::NInt { signed: false },
            ET_R4 => CsTy::Float { bits: 32 },
            ET_R8 => CsTy::Float { bits: 64 },
            ET_STRING  => CsTy::Str,
            ET_OBJECT  => CsTy::Object,
            ET_BYREF   => {
                // byref → same type (out/ref handled at param level)
                self.parse_ty()
            }
            kind @ (ET_VALUETYPE | ET_CLASS) => {
                let token = self.eat_uint();
                match self.type_names.get(&token) {
                    Some(name) => CsTy::Named { name: name.clone(), value_type: kind == ET_VALUETYPE },
                    None => CsTy::Unknown,
                }
            }
            ET_GENERICINST => {
                let kind = self.eat(); // CLASS or VALUETYPE
                let token = self.eat_uint();
                let base = self.type_names.get(&token).cloned().unwrap_or_default();
                let argc = self.eat_uint() as usize;
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(self.parse_ty());
                }
                CsTy::Generic { base, args, value_type: kind == ET_VALUETYPE }
            }
            ET_SZARRAY => {
                self.skip_cmods();
                CsTy::Array(Box::new(self.parse_ty()))
            }
            ET_ARRAY => {
                // multi-dim array: Type ArrayShape
                let elem = self.parse_ty();
                // skip ArrayShape (rank + sizes + lobounds)
                let _rank = self.eat_uint();
                let nsizes = self.eat_uint();
                for _ in 0..nsizes { self.eat_uint(); }
                let nlb = self.eat_uint();
                for _ in 0..nlb { self.eat_uint(); }
                CsTy::MdArray(Box::new(elem))
            }
            ET_VAR => {
                let idx = self.eat_uint() as usize;
                CsTy::Var(self.type_params.get(idx).cloned().unwrap_or_else(|| format!("T{idx}")))
            }
            ET_MVAR => {
                let idx = self.eat_uint() as usize;
                CsTy::Var(self.method_params.get(idx).cloned().unwrap_or_else(|| format!("M{idx}")))
            }
            ET_FNPTR => {
                // skip entire MethodDefSig
                self.skip_method_sig();
                CsTy::FnPtr
            }
            ET_PTR => {
                self.skip_cmods();
                CsTy::RawPtr(Box::new(self.parse_ty()))
            }
            ET_SENTINEL | ET_PINNED => self.parse_ty(),
            _ => CsTy::Unknown,
        }
    }

    pub(crate) fn skip_method_sig(&mut self) {
        let _calling = self.eat();
        // if GENERIC bit set, eat generic param count
        if _calling & 0x10 != 0 { self.eat_uint(); }
        let param_count = self.eat_uint();
        self.parse_type(); // return type
        for _ in 0..param_count {
            if self.peek() == ET_SENTINEL { self.eat(); }
            self.parse_type();
        }
    }

    // Parse MethodDefSig: calling convention, gen params, return type, params
    #[cfg(test)]
    pub(crate) fn parse_method_sig(&mut self, generic_param_names: &[String]) -> (String, Vec<(String, bool)>) {
        let (ret, params) = self.parse_method_sig_refs(generic_param_names);
        (ret.0, params.into_iter().map(|(ty, byref, _)| (ty, byref)).collect())
    }

    /// [`Self::parse_method_sig`] に、各型が**参照型か**を添えたもの（2-1）。
    /// 戻り値は `((型, 参照型か), [(型, BYREF か, 参照型か)])`。nullable 注釈は参照型にだけ意味がある。
    #[cfg(test)]
    pub(crate) fn parse_method_sig_refs(
        &mut self,
        _generic_param_names: &[String],
    ) -> ((String, bool), Vec<(String, bool, bool)>) {
        let ((ret, ret_is_ref), params) = self.parse_method_sig_tys();
        ((ret.arrow(), ret_is_ref), params.into_iter().map(|(t, byref, is_ref)| (t.arrow(), byref, is_ref)).collect())
    }

    /// [`Self::parse_method_sig_refs`] の C# の型のまま版（5-1）。
    pub(crate) fn parse_method_sig_tys(&mut self) -> ((CsTy, bool), Vec<(CsTy, bool, bool)>) {
        let calling = self.eat();
        let is_generic = calling & 0x10 != 0;
        let _is_instance = calling & 0x20 != 0;
        if is_generic {
            let _gen_count = self.eat_uint();
        }
        let param_count = self.eat_uint() as usize;
        let ret_is_ref = self.top_is_reference();
        let ret = self.parse_ty();
        let mut params = Vec::with_capacity(param_count);
        for _ in 0..param_count {
            if self.peek() == ET_SENTINEL { self.eat(); continue; }
            let is_byref = self.peek() == ET_BYREF;
            let is_ref = self.top_is_reference();
            let ty = self.parse_ty();
            params.push((ty, is_byref, is_ref));
        }
        ((ret, ret_is_ref), params)
    }

    /// 次の型が参照型（`string` / クラス / `object` / 配列 / 参照型のジェネリック）か。読み進めない。
    /// BYREF（`ref` / `out`）は中の型で決める。型変数（`T`）は分からないので偽（nullable 注釈を当てない）。
    pub(crate) fn top_is_reference(&self) -> bool {
        let mut pos = self.pos;
        loop {
            match self.data.get(pos).copied().unwrap_or(0) {
                ET_CMOD_REQD | ET_CMOD_OPT => {
                    let (_, n) = decompress_uint(self.data, pos + 1);
                    pos += 1 + n;
                }
                ET_BYREF => pos += 1,
                ET_STRING | ET_CLASS | ET_OBJECT | ET_SZARRAY | ET_ARRAY => return true,
                ET_GENERICINST => return self.data.get(pos + 1).copied() == Some(ET_CLASS),
                _ => return false,
            }
        }
    }
}


/// メタデータのシグネチャから読んだ C# の型（5-1）。
///
/// 型スタブは [`CsTy::arrow`] で Arrow の型の綴りにし、export の生成器（`export_gen`）は C# の型のまま読む。
/// ⚠ 以前はデコーダが Arrow の綴り（`int` など）を直接返していたので、`int` と `long` や `IntPtr` の区別が
///   残らず、C# のコードを生成できなかった。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CsTy {
    Void,
    Bool,
    Char,
    /// `sbyte` … `ulong`
    Int { bits: u8, signed: bool },
    /// `IntPtr` / `nint`（signed）・`UIntPtr` / `nuint`
    NInt { signed: bool },
    /// `float`（32）・`double`（64）
    Float { bits: u8 },
    Str,
    Object,
    /// 名前で引いた型（このアセンブリのクラス・構造体・enum、他のアセンブリの型）。`value_type` は構造体・enum
    Named { name: String, value_type: bool },
    /// ジェネリック型の具体化（`List<int>` / `Nullable<long>` / `Task<string>` …）。`base` は名前空間つき・アリティなし
    Generic { base: String, args: Vec<CsTy>, value_type: bool },
    /// 1 次元・0 始まりの配列（`T[]`）
    Array(Box<CsTy>),
    /// 多次元の配列（`T[,]`）
    MdArray(Box<CsTy>),
    /// 型変数（`T`）
    Var(String),
    FnPtr,
    RawPtr(Box<CsTy>),
    /// 読めなかった型
    Unknown,
}

impl CsTy {
    /// Arrow の型の綴り（型スタブ・`.ars`）。⚠ 5-1 より前のデコーダが返していた綴りと同じ（`byte[]` だけ 5-2 から `bytes`）。
    pub(crate) fn arrow(&self) -> String {
        match self {
            CsTy::Void => "None".to_string(),
            CsTy::Bool => "bool".to_string(),
            CsTy::Char | CsTy::Str => "str".to_string(),
            CsTy::Int { .. } | CsTy::NInt { .. } => "int".to_string(),
            CsTy::Float { .. } => "float".to_string(),
            CsTy::Object | CsTy::Unknown => "Any".to_string(),
            CsTy::Named { name, .. } => name.clone(),
            CsTy::Generic { base, args, .. } => map_generic(base, args.iter().map(CsTy::arrow).collect()),
            // `byte[]` は `bytes`（D-9・5-2）
            CsTy::Array(elem) if **elem == CsTy::Int { bits: 8, signed: false } => "bytes".to_string(),
            CsTy::Array(elem) | CsTy::MdArray(elem) => format!("list[{}]", elem.arrow()),
            CsTy::Var(name) => name.clone(),
            CsTy::FnPtr => "function".to_string(),
            CsTy::RawPtr(_) => "int".to_string(), // raw pointer → int handle
        }
    }
}


// Map well-known generic C# types to Arrow equivalents
pub(crate) fn map_generic(base: &str, args: Vec<String>) -> String {
    let simple = base.rsplit('.').next().unwrap_or(base);
    // strip arity suffix like `List`1` → `List`
    let simple = simple.split('`').next().unwrap_or(simple);
    match simple {
        "List" | "IList" | "ICollection" | "IEnumerable" | "IReadOnlyList"
        | "IReadOnlyCollection" | "ObservableCollection" | "Collection"
        | "Queue" | "Stack" | "LinkedList" | "ImmutableList" => {
            if args.len() == 1 { format!("list[{}]", args[0]) } else { "list".to_string() }
        }
        "Dictionary" | "IDictionary" | "IReadOnlyDictionary" | "SortedDictionary"
        | "ConcurrentDictionary" => {
            if args.len() == 2 {
                format!("dict[{},{}]", args[0], args[1])
            } else {
                "dict".to_string()
            }
        }
        "HashSet" | "SortedSet" | "ISet" | "ImmutableHashSet" => {
            if args.len() == 1 { format!("set[{}]", args[0]) } else { "set".to_string() }
        }
        "Tuple" | "ValueTuple" => {
            format!("tuple[{}]", args.join(","))
        }
        "Nullable" => {
            if args.len() == 1 { format!("Option[{}]", args[0]) } else { "Any".to_string() }
        }
        "Task" | "ValueTask" => {
            // Treat async as blocking in bridge; strip wrapper
            if args.len() == 1 { args[0].clone() } else { "None".to_string() }
        }
        "Action" | "Func" | "Predicate" | "EventHandler" | "Delegate" => {
            "function".to_string()
        }
        "KeyValuePair" => {
            if args.len() == 2 {
                format!("tuple[{},{}]", args[0], args[1])
            } else {
                "tuple".to_string()
            }
        }
        other => {
            if args.is_empty() {
                other.to_string()
            } else {
                format!("{}[{}]", other, args.join(","))
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn read_sig(data: &[u8]) -> (String, Vec<(String, bool)>) {
        let type_names = HashMap::new();
        let mut r = SigReader {
            data,
            pos: 0,
            type_names: &type_names,
            type_params: &[],
            method_params: &[],
        };
        r.parse_method_sig(&[])
    }

    /// `static void M(ref int a, int b)` — BYREF パラメータは is_byref=true。
    #[test]
    fn method_sig_byref_param_detected() {
        // calling=DEFAULT(0x00), count=2, ret=VOID, [BYREF I4], [I4]
        let (ret, params) = read_sig(&[0x00, 0x02, ET_VOID, ET_BYREF, ET_I4, ET_I4]);
        assert_eq!(ret, "None");
        assert_eq!(
            params,
            vec![("int".to_string(), true), ("int".to_string(), false)]
        );
    }

    /// `void M(out double d)`（インスタンスメソッド）— out も BYREF として検出。
    #[test]
    fn method_sig_out_double_detected() {
        // calling=HASTHIS(0x20), count=1, ret=VOID, [BYREF R8]
        let (ret, params) = read_sig(&[0x20, 0x01, ET_VOID, ET_BYREF, ET_R8]);
        assert_eq!(ret, "None");
        assert_eq!(params, vec![("float".to_string(), true)]);
    }

    /// C# の型のまま読む（5-1）。Arrow の綴りは以前と同じ（`ulong` / `IntPtr` は `int`、`float` は `float`）。
    #[test]
    fn method_sig_keeps_cs_types() {
        let type_names = HashMap::new();
        let mut r = SigReader { data: &[0x00, 0x02, ET_U8, ET_I, ET_R4], pos: 0, type_names: &type_names, type_params: &[], method_params: &[] };
        let ((ret, _), params) = r.parse_method_sig_tys();
        assert_eq!(ret, CsTy::Int { bits: 64, signed: false });
        assert_eq!(params.iter().map(|(t, _, _)| t.clone()).collect::<Vec<_>>(), vec![CsTy::NInt { signed: true }, CsTy::Float { bits: 32 }]);
        assert_eq!((ret.arrow(), params[0].0.arrow(), params[1].0.arrow()), ("int".to_string(), "int".to_string(), "float".to_string()));
    }

    /// BYREF フラグが `make_param` の `mutable`（Arrow の `mut` パラメータ）へ伝播する。
    #[test]
    fn make_param_propagates_byref_as_mut() {
        let p = crate::parser::cs_assembly::stub_gen::make_param("x", "int", true);
        assert!(p.mutable);
        assert_eq!(p.type_ann.as_deref(), Some("int"));
        let q = crate::parser::cs_assembly::stub_gen::make_param("y", "int", false);
        assert!(!q.mutable);
    }
}

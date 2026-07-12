// cs_assembly/signature.rs — CIL 型シグネチャ blob のデコード関数: SigReader のメソッドと C#→Arrow 型マッピング。

#[allow(unused_imports)]
use {
    std::collections::HashMap, std::path::Path,
    crate::ast::{Accessibility, Param, Stmt, TemplateParam},
};
#[allow(unused_imports)]
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

    // Parse one Type element; returns Arrow type string.
    pub(crate) fn parse_type(&mut self) -> String {
        self.skip_cmods();
        match self.eat() {
            ET_VOID    => "None".to_string(),
            ET_BOOLEAN => "bool".to_string(),
            ET_CHAR    => "str".to_string(),
            ET_I1 | ET_U1 | ET_I2 | ET_U2 | ET_I4 | ET_U4 |
            ET_I8 | ET_U8 | ET_I | ET_U => "int".to_string(),
            ET_R4 | ET_R8 => "float".to_string(),
            ET_STRING  => "str".to_string(),
            ET_OBJECT  => "Any".to_string(),
            ET_BYREF   => {
                // byref → same type (out/ref handled at param level)
                self.parse_type()
            }
            ET_VALUETYPE | ET_CLASS => {
                let token = self.eat_uint();
                self.type_names.get(&token).cloned().unwrap_or_else(|| "Any".to_string())
            }
            ET_GENERICINST => {
                self.eat(); // CLASS or VALUETYPE
                let token = self.eat_uint();
                let base = self.type_names.get(&token).cloned().unwrap_or_default();
                let argc = self.eat_uint() as usize;
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(self.parse_type());
                }
                map_generic(&base, args)
            }
            ET_SZARRAY => {
                self.skip_cmods();
                let elem = self.parse_type();
                format!("list[{elem}]")
            }
            ET_ARRAY => {
                // multi-dim array: Type ArrayShape — just return list[T]
                let elem = self.parse_type();
                // skip ArrayShape (rank + sizes + lobounds)
                let rank = self.eat_uint();
                let nsizes = self.eat_uint();
                for _ in 0..nsizes { self.eat_uint(); }
                let nlb = self.eat_uint();
                for _ in 0..nlb { self.eat_uint(); }
                let _ = rank;
                format!("list[{elem}]")
            }
            ET_VAR => {
                let idx = self.eat_uint() as usize;
                self.type_params.get(idx).cloned().unwrap_or_else(|| format!("T{idx}"))
            }
            ET_MVAR => {
                let idx = self.eat_uint() as usize;
                self.method_params.get(idx).cloned().unwrap_or_else(|| format!("M{idx}"))
            }
            ET_FNPTR => {
                // skip entire MethodDefSig
                self.skip_method_sig();
                "function".to_string()
            }
            ET_PTR => {
                self.skip_cmods();
                self.parse_type();
                "int".to_string() // raw pointer → int handle
            }
            ET_SENTINEL | ET_PINNED => self.parse_type(),
            _ => "Any".to_string(),
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
    pub(crate) fn parse_method_sig(&mut self, generic_param_names: &[String]) -> (String, Vec<(String, bool)>) {
        let calling = self.eat();
        let is_generic = calling & 0x10 != 0;
        let _is_instance = calling & 0x20 != 0;

        if is_generic {
            let _gen_count = self.eat_uint();
        }
        let param_count = self.eat_uint() as usize;
        let ret = self.parse_type();
        let mut params = Vec::with_capacity(param_count);
        for _ in 0..param_count {
            if self.peek() == ET_SENTINEL { self.eat(); continue; }
            let is_byref = self.peek() == ET_BYREF;
            let ty = self.parse_type();
            params.push((ty, is_byref));
        }
        let _ = generic_param_names;
        (ret, params)
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

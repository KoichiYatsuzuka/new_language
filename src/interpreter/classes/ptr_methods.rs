// classes/ptr_methods.rs — 外部のメモリを指す値(Ptr)のメソッドのディスパッチ: eval_ptr_method（ffi_value_exchange_plan.md 3-2・3-3）。
//
// `Ptr[void]` からの変換は `p mustbe Ptr[T]`（`ptr_ann_matches`）。

use {
    std::{cell::RefCell, rc::Rc, sync::Arc},
    crate::interpreter::{Interpreter, PtrHandle, Value},
};

impl Interpreter {
    /// `Ptr` のメソッドを評価済み引数で呼ぶ。
    ///
    /// - `release()` → `None`。所有しているメモリを今すぐ解放する（最後の参照を待たない）。借用・STATIC・
    ///   解放済み・C の側で解放済みのものは ValueError（黙って何もしない、にはしない）
    /// - `to_list()` → `list[T]`、`to_bytes()` → `bytes`（3-3）。指す先をコピーする（`PtrHandle::read_list` / `read_bytes`）
    pub(crate) fn eval_ptr_method(
        &mut self,
        p: Arc<PtrHandle>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        if matches!(method_name, "release" | "to_list" | "to_bytes") && !evaled.is_empty() {
            return Err(format!("TypeError: Ptr.{method_name}() takes no arguments ({} given)", evaled.len()));
        }
        match method_name {
            "release" => {
                p.release()?;
                Ok(Value::None)
            }
            "to_list" => {
                // 数値は幅で読む。構造体は C と同じレイアウトのクラスのインスタンスにする（コンストラクタは呼ばない）:
                // 数値だけの構造体は C のバイト列を写し（4-6）、ポインタのフィールドを持つ構造体はフィールドごとに D-7 の規則で
                // 変換する（7-14・値返しの構造体と同じ）
                let elem = p.elem_type().to_string();
                let items = if crate::interpreter::value::is_num_elem(&elem) || elem == "void" {
                    p.read_list()?
                } else {
                    let class = match self.get_val(&elem) {
                        Some(Value::Class(c)) if c.raw_layout.is_some() || c.c_struct.is_some() => c,
                        _ => {
                            return Err(format!(
                                "TypeError: Ptr[{elem}].to_list() needs '{elem}' to be a struct declared in a C header"
                            ))
                        }
                    };
                    if let Some(layout) = class.raw_layout.as_ref() {
                        p.read_struct_elements(layout.c_size())?
                            .into_iter()
                            .map(|b| {
                                crate::interpreter::InstanceData::from_c_bytes(class.clone(), &b)
                                    .map(|i| Value::Instance(Rc::new(RefCell::new(i))))
                            })
                            .collect::<Result<Vec<_>, _>>()?
                    } else {
                        let layout = match class.c_struct.as_deref() {
                            Some(Ok(l)) => l,
                            Some(Err(why)) => {
                                return Err(format!("TypeError: Ptr[{elem}].to_list() cannot expand struct '{elem}': {why}"))
                            }
                            None => unreachable!("checked above: a class with neither layout was rejected"),
                        };
                        let mut items = Vec::new();
                        for b in p.read_struct_elements(layout.size)? {
                            // SAFETY: 要素は `PtrProtocol` が並べた C の構造体（D-15）。ポインタのフィールドは C の側の規約で NULL か
                            // 読めるメモリ（生のヘッダの戻り値と同じ前提・D-7）
                            let values = unsafe { crate::interpreter::value::read_c_struct_fields(&b, layout)? };
                            let mut inst = crate::interpreter::InstanceData::new_empty(class.clone(), 0);
                            for (i, v) in values.into_iter().enumerate() {
                                inst.store_field(i, v, true);
                            }
                            items.push(Value::Instance(Rc::new(RefCell::new(inst))));
                        }
                        items
                    }
                };
                Ok(Value::List(Rc::new(RefCell::new(items))))
            }
            "to_bytes" => Ok(Value::bytes(p.read_bytes()?)),
            _ => Err(format!("AttributeError: 'Ptr[{}]' object has no method '{method_name}'", p.elem_type())),
        }
    }
}

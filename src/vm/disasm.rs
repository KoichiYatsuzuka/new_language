// vm/disasm.rs — Chunk の逆アセンブラ（開発・デバッグ用, Phase V）。

use super::chunk::Chunk;
use super::op::Op;

/// Chunk を人間可読な逆アセンブル文字列にする。
#[allow(dead_code)]
pub fn disassemble(chunk: &Chunk, name: &str) -> String {
    let mut out = format!("== chunk {name} (n_locals={}) ==\n", chunk.n_locals);
    for (i, op) in chunk.code.iter().enumerate() {
        out.push_str(&format!("{i:4}  {}\n", fmt_op(op, chunk)));
    }
    out
}

#[allow(dead_code)]
fn fmt_op(op: &Op, chunk: &Chunk) -> String {
    match op {
        Op::Const(i) => format!("CONST {i} = {:?}", chunk.consts.get(*i as usize)),
        Op::Nil => "NIL".to_string(),
        Op::LoadLocal(s) => format!("LOAD_LOCAL {s}"),
        Op::StoreLocal(s) => format!("STORE_LOCAL {s}"),
        Op::Pop => "POP".to_string(),
        Op::Bin(o) => format!("BIN {o:?}"),
        Op::Un(o) => format!("UN {o:?}"),
        Op::GetAttr(n, _) => format!("GET_ATTR {:?}", chunk.names.get(*n as usize)),
        Op::Jump(t) => format!("JUMP {t}"),
        Op::JumpIfFalse(t) => format!("JUMP_IF_FALSE {t}"),
        Op::JumpIfFalseOrPop(t) => format!("JUMP_IF_FALSE_OR_POP {t}"),
        Op::JumpIfTrueOrPop(t) => format!("JUMP_IF_TRUE_OR_POP {t}"),
        Op::Return => "RETURN".to_string(),
        Op::ReturnNil => "RETURN_NIL".to_string(),
    }
}

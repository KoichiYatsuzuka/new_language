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
        Op::LoadGlobal(n) => format!("LOAD_GLOBAL {:?}", chunk.names.get(*n as usize)),
        Op::Call(argc, mask, n, _) => {
            format!("CALL {:?} argc={argc} mut_mask={mask:#x}", chunk.names.get(*n as usize))
        }
        Op::CallMethod(n, argc, mask) => {
            format!("CALL_METHOD {:?} argc={argc} mut_mask={mask:#x}", chunk.names.get(*n as usize))
        }
        Op::StoreLocal(s) => format!("STORE_LOCAL {s}"),
        Op::StoreLocalDeepCopy(s) => format!("STORE_LOCAL_DEEPCOPY {s}"),
        Op::StoreLocalCopyFreeze(s) => format!("STORE_LOCAL_COPYFREEZE {s}"),
        Op::StoreLocalFreezeInstance(s) => format!("STORE_LOCAL_FREEZE_INST {s}"),
        Op::Pop => "POP".to_string(),
        Op::Bin(o) => format!("BIN {o:?}"),
        Op::Un(o) => format!("UN {o:?}"),
        Op::GetAttr(n, _) => format!("GET_ATTR {:?}", chunk.names.get(*n as usize)),
        Op::SetAttr(n) => format!("SET_ATTR {:?}", chunk.names.get(*n as usize)),
        Op::Swap => "SWAP".to_string(),
        Op::IsType(n) => format!("IS_TYPE {:?}", chunk.names.get(*n as usize)),
        Op::CallBuiltin(n, argc) => {
            format!("CALL_BUILTIN {:?} argc={argc}", chunk.names.get(*n as usize))
        }
        Op::GetIter => "GET_ITER".to_string(),
        Op::ForIter(it, tgt, exit) => format!("FOR_ITER iter={it} target={tgt} exit={exit}"),
        Op::Jump(t) => format!("JUMP {t}"),
        Op::JumpIfFalse(t) => format!("JUMP_IF_FALSE {t}"),
        Op::JumpIfFalseOrPop(t) => format!("JUMP_IF_FALSE_OR_POP {t}"),
        Op::JumpIfTrueOrPop(t) => format!("JUMP_IF_TRUE_OR_POP {t}"),
        Op::Return => "RETURN".to_string(),
        Op::ReturnNil => "RETURN_NIL".to_string(),
        Op::SetupTry(h) => format!("SETUP_TRY {h}"),
        Op::PopTry => "POP_TRY".to_string(),
        Op::Raise(s) => format!("RAISE span={s}"),
        Op::Reraise => "RERAISE".to_string(),
        Op::Dup => "DUP".to_string(),
        Op::ExcMatch(n) => format!("EXC_MATCH {:?}", chunk.names.get(*n as usize)),
        Op::BuildEmptyList => "BUILD_EMPTY_LIST".to_string(),
        Op::ListAppendLocal(s) => format!("LIST_APPEND_LOCAL {s}"),
        Op::ListOrNone => "LIST_OR_NONE".to_string(),
    }
}

# CLAUDE.md

Guidelines for Claude Code when working in this repository.

## Project Overview

**Arrow** is a custom scripting language targeting LLVM IR.
It aims to provide a Python-based syntax with static type checking and additional custom extensions.

- File extension: `.ar`
- Implementation language: **Rust** (main, `src/`) and **Python** (`impl_python/`)
- Indentation-based block structure (Python-style)

For the full directory layout and what each file is responsible for, use the `codebase-map` skill.
For a map of which subsystem (lexer/parser/type-checker/interpreter/VS Code extension) supports what, use the `architecture-overview` skill.

## Commands

### Rust implementation (primary)

```bash
cargo build                        # Compile
cargo run -- -src <file.ar>        # Execute a .ar file
cargo run -- <file.ar>             # Positional argument also supported
cargo run -- --repl                # Start interactive REPL
cargo test                         # Run all tests
cargo test <name>                  # Filter tests by partial name match
cargo clippy                       # Lint
cargo fmt                          # Format
cargo run -- --compile <file.ar>   # Partially compile a module (see partial-compile skill)
cargo run -- --compile-cs <file.dll> # Generate .ars stub from a .NET DLL (import[cs-dll]/import[cs-proc])
```

### Python implementation (`impl_python/`)

```bash
# Run from the repository root
python -m impl_python <file.ar>

# Examples
python -m impl_python examples/basics/variable.ar
python -m impl_python examples/basics/control_flow.ar
```

## Verification / measurement scripts (`scripts/`, PowerShell)

**All live scripts are in [scripts/](scripts/)** — run them from the repository root as `./scripts/<name>.ps1`. Every one of them is still in use — anything superseded lives in
[_archive/](_archive/) with the reason. Full details (arguments, known traps) are in the script table of
[BYTECODE_VM_PLAN.md](implementation_logs/BYTECODE_VM_PLAN.md); **which gate to run for which kind of change** is in the
`language-dev-principles` skill.

**Regression gates — run these after changing the language**

| Script | What it checks | Run it when |
|---|---|---|
| `scan_examples.ps1` | 全例題が失敗しないか（タイムアウト付き） | **毎回** |
| `force_gate.ps1` | VM に載らない構文が無いか（`VmForceError` 0 件） | **毎回** |
| `compare_python_impl.ps1` | 参照実装 `impl_python` との stdout 差分 | **毎回**（意味論を守る唯一の網） |
| `stale_doc_refs.ps1` | コメント中の識別子が src に実在するか | **識別子を改名・削除したら必ず** |
| `compare_bytecode.ps1 -A <base.exe>` | 2 バイナリのバイトコードが同一か | **VM コンパイラ**を触ったとき（「挙動不変」の主張はこれ） |
| `compare_outputs.ps1 -A <base.exe>` | 全例題の stdout/stderr/exit が同一か | **解釈側**を触ったとき（bytecode は自明に一致してしまう） |
| `compare_import_paths.ps1 -A <base.exe>` | import 系 13 例題の出力が同一か | **import / FFI** を触ったとき（他ゲートの対象外を埋める） |
| `dump_native_ir.ps1 -OutDir <dir>` | 生成 LLVM IR が byte-identical か | **ネイティブ codegen** を触ったとき（最強の検査） |
| `repl_session.ps1` | 対話 REPL の golden | **REPL / 最上位実行**を触ったとき |
| `debug_session.ps1` | デバッガのステッピングの golden | **デバッガ / 行テーブル**を触ったとき |
| `tw_stats.ps1` | ツリーウォークが定義文だけかの内訳 | **VM の適格範囲**を触ったとき |
| `syntax_cov.ps1` | **例題が一度も書いていない構文**（`NESTED-GAP`） | **新しい構文・文脈**を扱うとき |
| `generate-codebase-map.ps1` | `codebase-map` skill のファイル木を再生成 | **ファイルを作成・移動・削除したら必ず** |

⚠ A/B 系（`-A`）は **「直前のタスクのコミット」からビルドしたバイナリ**を基準にすること。
⚠ **使う前に同一 exe 同士で負の対照**（差分 0 になること）を取る。
⚠ ゲートは `target/release` を見る。**自分で走らせて緑**を確かめる（報告を信じない）。

**Measurement — before filing a speed task**

| Script | What it measures |
|---|---|
| `prof_dist.ps1` | 段別（parse/type_check/resolve/exec…）・op 別の実行時間分布（要 `--features prof`） |
| `ab_bench.ps1 -A <a.exe> -B <b.exe>` | 2 バイナリを**交互実行**して経過時間を比較 |
| `ab_bench_modes.ps1 -A <a> -B <b>` | 実行モード別（非コンパイル / native / C DLL）の A/B |
| `annot_unresolved.ps1` | 型注釈が `Unresolved` になる**発生源の内訳**（#19 の着手前に取る） |
| `annot_diff.ps1` | ネイティブ codegen の自前型導出と AST 注釈の一致状況 |

⚠ **推測せず先に計測する。** 命令数は当たりを付ける用で、速度の予測には使えない。
⚠ ノイズ帯は測るたび違う。**負の対照は A/B と同じセッションで取る**（詳細は `vm-pitfalls` §1）。

## Rules

@.claude/rules/regulations.md

@.claude/rules/language-differences.md

## Related Skills

Detailed, situational reference material lives in `.claude/skills/` and is loaded on demand — invoke by name or let it trigger automatically when relevant:

- `codebase-map` — directory-level module roles + auto-generated file tree with line counts (regenerate with `./scripts/generate-codebase-map.ps1` after file create/move/delete)
- `architecture-overview` — what the lexer/parser/type-checker/interpreter/VS Code extension currently support, with pointers into the deep-dive skills below
- `add-syntax` — layer-by-layer touchpoint checklist for adding a new keyword/statement/expression (read this FIRST for new language features, instead of the subsystem references below)
- `parser-internals` — full `src/parser/` implementation reference (module map, precedence chain, parse-time validations)
- `interpreter-internals` — full `src/interpreter/` implementation reference (exec/eval dispatch, closures, classes, native ABI handle table)
- `type-checking` — full `src/type_check/` implementation reference (`InferredType`, inference/compatibility rules, every `TypeErrorKind`)
- `partial-compile` — `--compile` workflow, `.arc`/`.ars` output, canonical `physics.ar` demo, and the full `src/partial_compiler/` codegen reference
- `importation` — `import[lang]` tag reference (`.ar`, `.py`, `.dll`/`.lib`, `.rs`, C#, Node.js) and the full `src/parser/imports.rs` / `exec_module` implementation reference
- `c-abi-interop` — C/C++ struct-passing design spec (raw layout, zero-copy vs. shadow conversion, write-back) for `import[cpp-dll]`/`import[cpp-lib]`
- `vscode-extension-dev` — adding/modifying VS Code extension features (highlighting, hover, inlay hints, completions, commands, settings) and packaging the VSIX
- `language-dev-principles` — durable design principles + working method for evolving the language (annotations are hints not semantics, the 4 storage kinds, stopping walker drift with exhaustive 2-stage forcing, which gate to run for which change, how to decide a task is worth doing); read when **designing or reviewing** a language change, before claiming "behaviour-preserving", and when filing tasks in a plan document
- `vm-pitfalls` — pitfalls hit while building the resolver + bytecode VM (benchmarking/A-B, opcode + peephole changes, why a green gate may be lying, PowerShell child-process and encoding traps); read before measuring or trusting a gate
- `vscode-debug-runner` — standalone CLI for exercising the VS Code extension's analysis code without launching VS Code

## Next Features to Implement (Priority Order)

⚠ The full backlog — every remaining task with its rationale, prerequisites and implementation
caveats — lives in [FUTURE_FEATURE.md](implementation_logs/FUTURE_FEATURE.md). Read it before picking up work.

1. **Expand native compilation** — support closures, generators, and `block_return`/`loop_yield` in compiled functions
2. **Async enhancements** — `async` blocks inside native-compiled functions; shared mutable state via explicit `Mutex`-style primitives

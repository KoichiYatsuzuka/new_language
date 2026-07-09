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
python -m impl_python examples/variable.ar
python -m impl_python examples/control_flow.ar
```

## Rules

@.claude/rules/regulations.md

@.claude/rules/language-differences.md

## Related Skills

Detailed, situational reference material lives in `.claude/skills/` and is loaded on demand — invoke by name or let it trigger automatically when relevant:

- `codebase-map` — full repository directory tree with per-file responsibilities
- `architecture-overview` — what the lexer/parser/type-checker/interpreter/VS Code extension currently support, with pointers into the deep-dive skills below
- `parser-internals` — full `src/parser/` implementation reference (module map, precedence chain, parse-time validations)
- `interpreter-internals` — full `src/interpreter/` implementation reference (exec/eval dispatch, closures, classes, native ABI handle table)
- `type-checking` — full `src/type_check/` implementation reference (`InferredType`, inference/compatibility rules, every `TypeErrorKind`)
- `partial-compile` — `--compile` workflow, `.arc`/`.ars` output, canonical `physics.ar` demo, and the full `src/partial_compiler/` codegen reference
- `importation` — `import[lang]` tag reference (`.ar`, `.py`, `.dll`/`.lib`, `.rs`, C#, Node.js) and the full `src/parser/imports.rs` / `exec_module` implementation reference
- `c-abi-interop` — C/C++ struct-passing design spec (raw layout, zero-copy vs. shadow conversion, write-back) for `import[cpp-dll]`/`import[cpp-lib]`
- `vscode-extension-dev` — adding/modifying VS Code extension features (highlighting, hover, inlay hints, completions, commands, settings) and packaging the VSIX
- `vscode-debug-runner` — standalone CLI for exercising the VS Code extension's analysis code without launching VS Code

## Next Features to Implement (Priority Order)

1. **Expand native compilation** — support closures, generators, and `block_return`/`loop_yield` in compiled functions
2. **Async enhancements** — `async` blocks inside native-compiled functions; shared mutable state via explicit `Mutex`-style primitives

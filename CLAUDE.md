# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build          # compile
cargo run            # build and run
cargo test           # run all tests
cargo test <name>    # run a single test by name (substring match)
cargo clippy         # lint
cargo fmt            # format code
```

## Architecture

This is a new Rust project (`edition = "2021"`) with a single binary entry point at `src/main.rs`. No dependencies have been added yet.

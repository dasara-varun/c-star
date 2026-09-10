# C* Programming Language

C* is a statically-typed, LLVM-backed systems programming language with compiler-inferred ownership, zero undefined behavior, and structured diagnostics.

## Toolchain

| Binary | Description |
|--------|-------------|
| `starc` | Compiler: `.cx` source → LLVM IR → native binary |
| `orbit` | Package manager: init, add deps, build |
| `cstar-bind` | C header → C* FFI bindings generator |
| `starfmt` | Canonical code formatter |

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs/) (for building the toolchain)
- [LLVM 18+](https://releases.llvm.org/) with `clang` on your PATH

### Build

```powershell
cargo build --release
```

### Hello World

```powershell
$env:PATH = "C:\Program Files\LLVM\bin;" + $env:PATH
cargo run -p starc -- build examples/hello.cx -o hello.exe
.\hello.exe
```

### Multi-module Project

```powershell
cargo run -p starc -- build examples/main.cx -o main.exe
.\main.exe
```

### Package Manager

```powershell
orbit init
orbit add geometry
orbit build
```

## Language Features

- **Memory safety by default** — ownership inference with move/borrow checking
- **No undefined behavior** — overflow traps in `--profile=safe`, wraps in `--profile=fast`
- **Structured diagnostics** — `starc build --error-format=json`
- **Modules** — `use geometry` resolves across files
- **Generics** — monomorphized at compile time (`max<T>`)
- **Result/Option** — with `match` and `?` operator
- **Raw blocks** — `raw { }` for unchecked FFI and pointer operations
- **Coroutines** — fiber-based producer/consumer pattern (Windows)

## Examples

```
examples/
  hello.cx          — Hello, World
  geometry.cx       — Struct + impl + methods
  main.cx           — Multi-module import
  coro_example.cx   — Coroutine producer/consumer
```

## Project Layout

```
compiler/     starc compiler (lexer, parser, typeck, codegen)
orbit/        Package manager
cstar-bind/   C header binding generator
starfmt/      Code formatter
stdlib/       Standard library sources
tests/        compile-pass, compile-fail, run test suites
docs/spec/    Language specification and design decisions
examples/     Example programs
```

## Compiler Options

```
starc build <file> [options]

  --error-format=json|human   Diagnostic output format (default: human)
  --profile=safe|fast         Overflow behavior (default: safe)
  --emit=llvm-ir              Emit LLVM IR instead of binary
  -o <output>                 Output file path
```

## Running Tests

```powershell
.\scripts\run_tests.ps1
```

Or via cargo (requires LLVM on PATH):

```powershell
cargo test -p starc
```

## Specification

See [docs/spec/full_spec.md](docs/spec/full_spec.md) for the complete language specification and phased build plan.

## License

See individual component licenses. Compiler toolchain is under development.

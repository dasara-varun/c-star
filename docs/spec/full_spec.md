<USER_REQUEST>
C* Language — Build Specification
Purpose of this document: hand this to an AI coding agent (e.g. Claude Code) as the starting spec to begin implementing the C* compiler and toolchain. It contains concrete syntax, semantics, architecture, and a phased build plan with acceptance criteria. Where a decision is genuinely open, it's marked `[DECIDE DURING IMPLEMENTATION]` — the agent should make a reasonable choice, document it, and move on rather than blocking.
---
0. One-paragraph brief for the agent
Build C*, a small, statically-typed, LLVM-backed systems programming language. Core properties: memory-safe by default via compiler-inferred ownership (no explicit lifetime annotations required in common code), zero undefined behavior (every operation is either well-defined or a compile error), a built-in package manager and module system (no textual preprocessor), and a compiler that emits structured machine-readable diagnostics alongside human-readable ones. Bootstrap the compiler in Rust using LLVM bindings; self-host in C* itself only once the language is stable (v1.0+ milestone, not before).
---
1. Non-negotiable design constraints
These are the rules the agent should treat as hard constraints, not suggestions:
No undefined behavior, ever. Every language construct must have fully defined semantics. If a case can't be defined safely, it must be a compile error — not "implementation-defined" or "unspecified."
Safe by default, unsafe by explicit block. Only code inside a `raw { }` block may perform unchecked pointer arithmetic, raw memory access, or call into C ABI without safety wrapping.
No textual preprocessor. No `#include`, no textual macro substitution. Modules are compiled units resolved by the module system.
One official toolchain. Compiler, package manager, and formatter ship together, versioned together, from the first working build.
Diagnostics are structured. Every compiler error/warning must be emittable as JSON (`--error-format=json`) with: error code, severity, file, line, column span, human message, and (where possible) a suggested fix. Human-readable text output is a rendering of the same structured data, not a separate code path.
LLVM is the backend. Do not write a custom codegen/optimizer. Use LLVM (via `inkwell` or direct `llvm-sys` bindings if the host language is Rust) from the first working compiler.
Reuse existing ecosystems by default — do not force reinvention. A new language with no libraries is not useful, regardless of how good the language itself is. C* must be able to call into existing C, Rust, and (via a bridge) C++ code from the earliest practical milestone — see §9E for the concrete mechanism. This is treated as core to the language's usefulness, not an optional nice-to-have bolted on later.
---
0.5 Evidence base — the specific problems this design answers
Every design choice below traces to a specific, researched problem, not a preference. Restated here so the agent building this understands why, not just what:
Problem (evidenced)	Where it's addressed in this spec
~70% of Microsoft/Google/Mozilla CVEs are memory safety bugs; Android's memory-safety CVE share dropped 76%→24% after adopting a safer language	§1.1–1.2 (no UB), §4.2/§6.4 (ownership)
Rust is the most-admired language, but its ownership ceremony is the top complaint against it	§4.2 (inferred ownership, explicit annotation only as fallback)
Zig is highly admired specifically for pragmatic simplicity despite offering no compile-time safety	Small core language, single `raw{}` escape hatch, no template-metaprogramming complexity
No standard C/C++ package manager; developers resort to copy-pasting source	§2, §8 v0.2 (`orbit` with lockfiles, built in from day one)
C preprocessor causes redundant re-parsing and poor tooling	§1.3 (no textual preprocessor, real modules)
#1 developer frustration in 2025/2026 surveys is "AI code that's almost right" and hard to debug	§1.1, §1.5, §9B (no UB + structured diagnostics turn silent failures into compile-time, machine-legible ones)
AI-generated C/C++ specifically fails by compiling and running while still being wrong (races, hallucinated APIs, UB)	§5 (no-UB table), §9E (closed type system rejects hallucinated signatures at compile time)
New languages fail to get adopted because they have no libraries, regardless of language quality	§1.7, §9E (C/Rust/C++ interop as a non-negotiable, not an afterthought)
Compile-time speed is a stated value in most new languages but rarely has a real mechanism	§9A (query-based incremental compilation, concrete acceptance criteria)
Tooling/IDE support lags language design and kills adoption independently of the language itself	§9B (compiler-as-library, LSP pulled forward to v0.2)
Rust's async/concurrency model is a major source of complexity complaints	§9C (channels + `Shared<T>` as the primary model, coroutines without function coloring)
Existing C/C++/Rust programmers face real onboarding friction with a from-scratch syntax	§9D (C-family surface syntax retained deliberately, migration guides, raw-heavy on-ramp)
---
2. Toolchain overview
Binary	Role
`starc`	Compiler: source → LLVM IR → native binary or object file
`orbit`	Package manager: init, add/remove deps, lockfile, build orchestration
`nova`	REPL — can be deferred past v0.1
`starfmt`	Canonical code formatter — can be deferred past v0.2
Bootstrap language: Rust. Rationale: mature LLVM bindings (`inkwell`), memory-safe implementation of the compiler itself (dogfooding the value proposition), fast to prototype in, large ecosystem for parser tooling if needed (though a hand-written recursive-descent parser is preferred for error-message quality — see §6.2).
Repository layout `[DECIDE DURING IMPLEMENTATION — suggested below]`:
```
cstar/
  compiler/           # starc, written in Rust
    src/
      lexer/
      parser/
      ast/
      typeck/         # type checking + ownership inference
      codegen/        # LLVM IR generation
      diagnostics/     # structured error reporting
      main.rs
    Cargo.toml
  orbit/              # package manager, written in Rust
  stdlib/             # C* standard library, written in C* once bootstrapped enough,
                       # or minimal Rust shims initially
  tests/
    compile-pass/
    compile-fail/     # each has expected .json diagnostic output
    run/              # programs with expected stdout
  examples/
  docs/
    spec/             # formal language spec, grows as features land
```
---
3. Lexical grammar
Source files: UTF-8, extension `.cx`.
Identifiers: `[a-zA-Z_][a-zA-Z0-9_]*`.
Comments: `// line` and `/* block */` (nestable).
No semicolons required at end of statement; newline is a statement terminator (like Go/Kotlin) — `[DECIDE DURING IMPLEMENTATION: exact newline-insertion rule set, but default to "newline ends statement unless the line is clearly incomplete (trailing operator, unclosed paren)"]`.
String literals: `"..."` with standard escapes; no implicit textual interpolation in v0.1 (add `f"...{expr}..."` in a later milestone if desired).
Numeric literals: `123`, `123u32`, `1.5f64`, `0xFF`, `0b1010`. Underscore separators allowed: `1_000_000`.
---
4. Core syntax (EBNF-flavored, informal)
```
module_decl   := "module" IDENT NEWLINE
import_decl   := "use" module_path ("as" IDENT)?

item          := fn_decl | struct_decl | enum_decl | impl_block | const_decl

fn_decl       := "pub"? "fn" IDENT generic_params? "(" param_list? ")" ("->" type)? block
param_list    := param ("," param)*
param         := IDENT ":" type
generic_params:= "<" IDENT ("," IDENT)* ">"

struct_decl   := "pub"? "struct" IDENT generic_params? "{" field_list? "}"
field_list    := field ("," field)*
field         := "pub"? IDENT ":" type

enum_decl     := "pub"? "enum" IDENT generic_params? "{" variant_list? "}"
variant_list  := variant ("," variant)*
variant       := IDENT ("(" type_list ")")?

impl_block    := "impl" type "{" fn_decl* "}"

type          := IDENT generic_args?
                | "&" type              // borrowed reference
                | "[" type "]"          // slice
                | "raw" "*" type        // raw pointer, only legal inside raw{} or FFI signatures
                | "Result" "<" type "," type ">"
                | "Option" "<" type ">"

block         := "{" statement* "}"
statement     := let_stmt | expr_stmt | return_stmt | if_expr | match_expr
              | for_stmt | while_stmt | raw_block

let_stmt      := "let" "mut"? IDENT (":" type)? "=" expr
return_stmt   := "return" expr?
raw_block     := "raw" block             // enters unsafe scope

if_expr       := "if" expr block ("else" (if_expr | block))?
match_expr    := "match" expr "{" match_arm+ "}"
match_arm     := pattern "=>" (expr | block) ","?

for_stmt      := "for" IDENT "in" expr block
while_stmt    := "while" expr block

comptime_fn   := "comptime" "fn" IDENT generic_params
                  ("where" bound_list)? "(" param_list? ")" ("->" type)? block
bound_list    := IDENT ":" IDENT ("," IDENT ":" IDENT)*
```
4.1 Worked example — a small real program
```cstar
module geometry

pub struct Point {
    x: f64,
    y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Point {
        Point { x: x, y: y }
    }

    pub fn distance(self, other: &Point) -> f64 {
        let dx = self.x - other.x
        let dy = self.y - other.y
        sqrt(dx * dx + dy * dy)
    }
}

pub fn safe_divide(a: i32, b: i32) -> Result<i32, string> {
    if b == 0 {
        return Err("division by zero")
    }
    Ok(a / b)
}

fn main() {
    let p1 = Point::new(0.0, 0.0)
    let p2 = Point::new(3.0, 4.0)
    print(p1.distance(&p2))   // 5.0, no explicit lifetime annotation needed

    match safe_divide(10, 0) {
        Ok(v) => print(v),
        Err(e) => print(e),
    }
}
```
4.2 Ownership rules (v0.3 milestone — can compile without this first)
Every value has exactly one owner at a time.
Passing a value to a function moves it unless the parameter type is `&T` (borrow) or the type implements `Copy` (primitive numeric types, small structs marked `#[copy]`).
The compiler performs local, function-scoped lifetime inference first; only when inference fails across a boundary the compiler cannot see through (e.g., returning a borrow from a function) does it require an explicit lifetime parameter — this should be rare in idiomatic code.
`raw *T` pointers bypass ownership entirely and are only constructible/dereferenceable inside a `raw { }` block.
4.3 Error handling
`Result<T, E>` and `Option<T>` are core stdlib types, not language keywords, but `?` is a language-level operator: `expr?` short-circuits and returns `Err(e)`/`None` from the enclosing function if `expr` is `Err`/`None`, otherwise unwraps.
4.4 FFI / raw block example
```cstar
module ffi_example

use core.raw_ffi

pub fn read_c_buffer(ptr: raw *u8, len: usize) -> []u8 {
    raw {
        return slice_from_raw_parts(ptr, len)   // unsafe stdlib primitive
    }
}
```
---
5. Semantics — no-UB rules (must be enforced by typeck/codegen)
C/C++ UB case	C* behavior
Signed integer overflow	Defined: wraps in `--profile=fast`, traps (panics with diagnostic) in `--profile=safe` (default)
Reading uninitialized memory	Compile error — all bindings must be initialized before first read; `typeck` must track definite-assignment
Out-of-bounds array/slice access	Bounds-checked at runtime in safe code; check elided by optimizer when provably in-bounds; inside `raw{}`, unchecked and documented as such
Null pointer dereference	Impossible outside `raw{}` — no null references in safe code; use `Option<T>` instead
Data races	Prevented at compile time by ownership rules (no two mutable aliases across threads without explicit sync primitive)
Use-after-free	Impossible outside `raw{}` — ownership model prevents use after move/drop
---
6. Compiler architecture
6.1 Pipeline
```
source (.cx) → Lexer → tokens → Parser → AST → Resolver (name/module resolution)
  → Type Checker (+ ownership inference) → Typed AST (HIR)
  → LLVM IR Codegen → LLVM optimization passes → object file / native binary
```
6.2 Parser
Hand-written recursive-descent parser (not a parser-generator like yacc/ANTLR). Rationale: error message quality is a first-class requirement (see §1.5), and hand-written parsers give far more control over recovery and precise diagnostics than generated ones.
Must support error recovery: on a syntax error, the parser should skip to a recovery point (next statement boundary) and continue, so a single file can report multiple independent syntax errors in one pass — critical for both human and AI-agent iteration speed.
6.3 Diagnostics format (concrete schema)
```json
{
  "version": "1",
  "diagnostics": [
    {
      "code": "E0203",
      "severity": "error",
      "message": "use of moved value `buf`",
      "file": "src/main.cx",
      "span": { "start_line": 12, "start_col": 5, "end_line": 12, "end_col": 8 },
      "notes": [
        { "message": "`buf` was moved here", "span": { "start_line": 9, "start_col": 10, "end_line": 9, "end_col": 13 } }
      ],
      "suggested_fix": {
        "description": "borrow instead of moving",
        "replacement": "&buf"
      }
    }
  ]
}
```
Every error code (`E0xxx`) must be documented in `docs/spec/errors.md` with a minimal reproducing example — this doubles as compiler test fixtures.
6.4 Type checker + ownership inference
Standard Hindley-Milner-style local type inference for expressions.
Ownership/borrow inference implemented as a separate pass after type checking, operating on the typed AST — do not conflate type inference and borrow inference in one pass; this is the architecture Rust wishes it had started with, and keeping them separate will make the borrow-inference pass much easier to iterate on independently.
6.5 Codegen
Direct HIR → LLVM IR lowering.
Use LLVM's built-in overflow-checked arithmetic intrinsics (`llvm.sadd.with.overflow`, etc.) to implement defined-overflow semantics cheaply.
Bounds checks lowered as ordinary branches; rely on LLVM's optimizer plus a dedicated C*-side "provably in bounds" analysis pass to elide redundant checks.
---
7. Minimal standard library scope for v0.1–v0.2
Keep this deliberately small at first:
Primitive numeric types (`i8`..`i64`, `u8`..`u64`, `f32`, `f64`, `bool`, `char`)
`string`, `[]T` (slice), fixed arrays `[N]T`
`Option<T>`, `Result<T, E>`
`print`, basic I/O (stdin/stdout/stderr, file read/write)
`core.raw_ffi` primitives (`slice_from_raw_parts`, C ABI call support)
Basic math (`sqrt`, `abs`, min/max)
Defer: collections beyond slices/arrays (`Vec`, `HashMap`), concurrency primitives, networking — these come after the core language is stable (post v0.4).
---
8. Phased build plan with acceptance criteria
v0.1 — "It compiles a real program"
Build: lexer, recursive-descent parser, AST, minimal type checker (no ownership inference yet — use simple move-or-copy-everything semantics as a placeholder), LLVM codegen for functions/structs/control flow/arithmetic, `raw{}` block parses and type-checks (semantics can be permissive here), JSON diagnostics for at least parse errors and basic type errors.
Acceptance criteria:
`starc build examples/hello.cx` produces a native binary that runs and prints correctly.
The worked example in §4.1 compiles and runs, producing `5` for the distance call (ownership inference not required yet — treat `&Point` as a plain reference with no inference, just explicit borrow syntax accepted).
`starc build --error-format=json` on a file with 3 independent syntax errors emits all 3 in one pass, matching the schema in §6.3.
CI runs `tests/compile-pass` and `tests/run` on every commit.
v0.2 — Modules + package manager
Build: `use` module resolution across files/directories, `orbit init`/`orbit add`/`orbit build` with a lockfile format, `starc` reads `orbit`-resolved dependency graphs. Also build `cstar-bind` (§9E) as part of this milestone so `orbit add` can pull in an existing C library (via a system package or vendored header) alongside native C* dependencies — reusing existing code should be possible from the same milestone that introduces the package manager, not bolted on later.
Acceptance criteria:
A two-module project (`main` importing `geometry` from §4.1) builds via `orbit build`.
`orbit add <local-path-dependency>` + lockfile round-trips correctly (add, remove, rebuild, lockfile unchanged if nothing changed).
`orbit add --c-header <path-to-.h>` (or equivalent) generates `raw` bindings via `cstar-bind` and the resulting project compiles and successfully calls into the C library.
v0.3 — Ownership inference + no-UB enforcement
Build: the borrow-inference pass (§6.4), definite-assignment checking, overflow/bounds semantics per §5, `Result`/`?` operator.
Acceptance criteria:
A program that uses a value after moving it fails to compile with error code + suggested fix per the schema in §6.3.
A program with unchecked signed overflow in `--profile=safe` panics with a clear diagnostic at runtime; same program in `--profile=fast` wraps silently (both documented, both intentional, neither UB).
`tests/compile-fail` suite (at least 20 cases) each has an expected JSON diagnostic fixture that must match exactly.
v0.4 — Generics (`comptime`) + coroutines
Build: compile-time function execution for generics, lightweight coroutine primitive.
Acceptance criteria:
The `max<T>` generic example (§ design doc) compiles and specializes correctly for at least `i32` and `f64`.
A basic coroutine example (producer/consumer) runs correctly under the runtime scheduler.
v1.0 — Stabilize + self-host
Build: freeze core syntax/semantics, write a C* compiler front-end in C* itself, compare output against the Rust bootstrap compiler on the full test suite.
Acceptance criteria:
The self-hosted compiler compiles itself (fixed point).
The self-hosted and bootstrap compilers produce identical (or intentionally-documented-different) output across the full test suite.
---
9. Testing strategy
`tests/compile-pass/*.cx` — must compile successfully; paired with expected stdout for `tests/run/`.
`tests/compile-fail/*.cx` — must fail with a specific expected JSON diagnostic (exact match on `code`, fuzzy/substring match on `message` to avoid brittle tests).
Golden-file testing for diagnostics is critical — this is the interface AI agents and IDEs will depend on, so schema stability matters more than for a typical internal API.
Add a fuzzer (e.g., `cargo-fuzz` on the parser) once the parser is stable enough not to constantly break on real input — target: no panics/crashes on arbitrary byte input, even if it's a "reject with diagnostic" outcome.
---
9A. Incremental compilation & caching architecture
Gap being closed: "fast compilation" was previously a stated value with no mechanism.
Adopt a query-based compiler architecture, the same model `rustc` moved to and that frameworks like `Salsa` (Rust) formalize: every compiler phase (parse a file, resolve a name, type-check a function, generate IR for a function) is a memoized query keyed on its inputs. Re-running the compiler after a small edit only re-executes queries whose inputs actually changed.
Concretely: `starc` should depend on (or build an equivalent of) an incremental-computation library from the start of v0.1's internal architecture, even though the user-facing benefit (fast rebuilds) won't be obviously visible until projects are multi-file (v0.2+).
Cache granularity target: per-function for type-checking and codegen, per-file for parsing — this matches where real-world edits happen and avoids whole-module invalidation on a one-line change.
Add this acceptance criterion to v0.2: editing a single function's body in a 10-file project and rebuilding must not re-typecheck or re-codegen unrelated functions in other files. Measure via a query-count assertion in CI, not just wall-clock time (wall-clock is noisy; query counts are deterministic and testable).
9B. Tooling & LSP strategy
Gap being closed: "the language could be great and still fail because the tooling around it doesn't exist."
Architect the compiler as a library first, a CLI second. `starc` the binary should be a thin wrapper over a `starc-core` crate that exposes parse/typecheck/diagnose as callable functions returning structured data (the same JSON-shaped diagnostics from §6.3). This is what makes an LSP server possible without a second, drifting implementation of the front end.
LSP server (`star-lsp`) becomes a first-class deliverable at v0.2, not an afterthought bolted on later — reuse `starc-core` directly for: go-to-definition (from the resolver pass), inline diagnostics (already structured), and basic completion (from the type checker's in-scope symbol table). This is a modest scope increase to v0.2 but it's the single highest-leverage tooling investment, since every editor integration (VS Code, Neovim, etc.) can build on one LSP implementation instead of N bespoke integrations.
Defer: full refactoring support, semantic highlighting beyond basics, debugger integration (DAP) — target these for v0.4/v1.0, and note that debugger support specifically will need debug-info emission in the LLVM codegen step (`§6.5`), which should at least reserve the DWARF/line-table hooks in v0.1 codegen even if unused yet, since retrofitting debug info emission later is disruptive.
9C. Concurrency model beyond coroutines
Gap being closed: coroutines alone don't answer real shared-state concurrency needs.
Shared mutable state across threads requires an explicit synchronization type, checked at compile time via the ownership system — e.g. a `Shared<T>` wrapper (mutex-backed) that is the only way a value can be aliased across thread boundaries. Attempting to share a plain `T` across a spawned thread without wrapping it in `Shared<T>` is a compile error, not a runtime race.
Channels as the preferred pattern, not shared memory — provide `Channel<T>` in the stdlib (bounded and unbounded) as the idiomatic way coroutines/threads communicate, with `Shared<T>` available for the genuine cases that need it.
This is a v0.4+ design target, not v0.1 — but the ownership model decisions made in v0.3 (§4.2, §6.4) must be built with this in mind from the start: the borrow-inference pass needs to already understand "this value crosses a thread/task boundary" as a category, even if `Shared<T>`/`Channel<T>` aren't implemented until later. Retrofitting thread-safety awareness into an ownership system that wasn't designed for it is a well-documented source of pain in other languages' histories — design the hook now, implement the feature later.
---
9D. Learnability for existing C / C++ / Rust programmers
Explicit design constraint, added in response to onboarding concerns: the syntax should never be the barrier — only the ownership concept itself should require real learning, because that concept is the entire value proposition of the language.
Surface syntax stays C-family throughout: braces for blocks, same `if`/`else`/`while`/`for` keywords and structure, same operators and precedence, same comment syntax (`//`, `/* */`). Do not introduce novel punctuation or control-flow keywords where a C/C++/Rust programmer would already recognize the equivalent. `match` is the one deliberate exception (no direct C/C++ equivalent) — document it explicitly as "this is `switch`, but exhaustive and without fallthrough."
`impl` blocks and methods should read as close to C++ member functions as the ownership model allows — same call syntax (`obj.method(args)`), same conceptual grouping of behavior with data.
Ship a migration guide as a first-class doc, not an afterthought: `docs/migration/from-c.md`, `docs/migration/from-cpp.md`, `docs/migration/from-rust.md`, each a side-by-side syntax table (e.g. "C `malloc`/`free` → C* owned values + `raw{}` for the rare manual case"; "Rust explicit lifetimes → C* usually infers this, here's when you still need to annotate").
Compiler errors should teach the ownership model, not just reject code. When a C/C++ programmer's first instinct (share a raw pointer everywhere) fails to compile, the diagnostic (per the §6.3 schema) should explain why in ownership terms and suggest the idiomatic fix — this turns the compiler into the primary teaching tool instead of requiring a textbook detour, which is a known friction point in how Rust is typically learned today.
Provide a "raw-heavy" on-ramp mode for porting existing C code: a project can legally be written almost entirely inside `raw{}` blocks initially (mechanically close to a direct C port), compiling and running correctly, and then be incrementally tightened into safe idioms function-by-function as the team learns the ownership model. This mirrors how TypeScript let JavaScript developers adopt gradually rather than requiring a rewrite before anything runs — add this as an explicit, supported, documented workflow rather than an implicit possibility nobody's told about.
What this does not do: it does not make the ownership concept itself optional or trivial. That concept is the mechanism behind the safety claims in §1 — simplifying it away to ease onboarding would remove the reason the language exists. The commitment here is to make everything around that one real learning curve as familiar and low-friction as possible, not to remove the curve itself.
---
9E. Cross-language interoperability
Design principle: the C ABI is the universal interchange layer. Every interop story below routes through it in one form or another — there is no case where C* needs a bespoke binding mechanism per target language.
C — zero additional work
Already covered by `raw{}` + `core.raw_ffi` (§4.4). C has a stable, simple, mostly-uniform ABI across compilers/platforms, so this is the baseline case.
C automatic binding generator (`cstar-bind`) — v0.3 milestone
Rather than requiring every C function signature to be hand-transcribed into C*, build `cstar-bind`: parses a C header (`.h`) and emits `raw` extern declarations plus, where the signature is simple enough to infer safe ownership semantics (e.g. `const char*` + explicit length → treat as a borrowed slice), a thin safe wrapper. Model this directly on Rust's `bindgen`, which solves the identical problem.
Rust — via the same C-ABI door, not a separate mechanism
Rust itself has no stable ABI, but Rust libraries commonly expose one explicitly via `#[no_mangle] extern "C"` functions (typically generated with `cbindgen` on the Rust side). C* consumes a Rust library exactly the way it consumes a C library — `cstar-bind` works unmodified here since the exposed surface is plain C ABI. No dedicated Rust-interop feature is needed — document this clearly so users don't expect (or wait for) separate tooling that isn't actually necessary.
C++ — requires a dedicated bridge tool, scoped honestly
C++ has no single stable ABI (name mangling differs by compiler/version), and templates, exceptions, and RAII destructors have no C-callable equivalent. This is not a C*-specific gap — Rust faces the identical problem and solves it with the `cxx` crate: require the C++ side to expose an explicit, restricted, bridgeable subset (plain functions/methods, no templates, no exceptions crossing the boundary, explicit ownership transfer), and generate a shim from that declaration.
Plan: `cstar-cxx-bridge`, a dedicated tool, targeting post-v1.0 — this is real, non-trivial work and should not block the core language.
Interim path (available immediately, v0.1+): any C++ library that already exposes a C API shim (a very common existing practice — OpenCV, SQLite's C++ internals via a C wrapper, etc.) can be consumed today via the plain C interop path with zero extra tooling.
Other languages (Python, Java, JS, etc.) — deferred, case-by-case
Each of these already exposes its own C-ABI-compatible extension mechanism (Python's C extension API, JNI, Node's N-API). C* interop with any of them is the same pattern again: bind through the existing C-ABI surface that language already provides. Do not build generic tooling for this speculatively — add a specific binding path only when real demand for a specific target language shows up, post-v1.0.
Honest limitation to state clearly in user-facing docs
"C* can use any library, in any language" is not literally true and should never be claimed that way — what's true is "C* can use any library that exposes (or can be given) a C-ABI-compatible surface," which in practice covers the overwhelming majority of systems libraries, but is a real boundary, not an unlimited one. Rust and Zig make the identical claim with the identical caveat — this isn't a weaker position than either of them, just an honestly stated one.
---
10. Explicit instructions for the agent starting this build
Start with §8's v0.1 milestone only. Do not attempt ownership inference, generics, or the package manager until v0.1's acceptance criteria pass.
Set up the repo layout from §2 first, with a working `cargo build` no-op before writing any language logic.
Write the lexer and a golden-file test suite for tokenization before the parser.
Write the parser with error recovery from the start — retrofitting recovery later is significantly harder than building it in.
Implement the JSON diagnostics schema (§6.3) before human-readable text rendering — render text from the JSON structure, not the other way around, so the two never drift.
After each milestone, update `docs/spec/` with whatever was actually built (the spec in this document is a starting intent, not a frozen contract — the agent should treat divergences as normal and document them, not silently ignore this doc).
Where this document says `[DECIDE DURING IMPLEMENTATION]`, make the decision, write one paragraph in `docs/spec/decisions.md` explaining the choice and why, and proceed — do not block on these.
---
11. Known unknowns — not fixable by more design work
These are listed explicitly so nobody mistakes this document for a finished, risk-free plan. Each of these can only be answered by building the language and having people use it — no amount of additional spec-writing resolves them in advance:
Whether inferred ownership actually stays low-ceremony at scale. The design in §4.2/§6.4 is sound in principle, but Rust's explicit-annotation model also looked reasonable on paper before real-world code surfaced the `Arc`/`.clone()` workaround pattern. C*'s inference approach could hit its own equivalent friction point that won't be visible until v0.3 is built and real programs are written against it.
Whether the syntax is actually pleasant to write. Nobody has written real code in this language yet. This is only testable empirically, starting at v0.1.
Ecosystem and library adoption. Even a technically excellent language fails to get used without libraries. C interop (§4.4) is a bridge, not a solution — this is a multi-year, largely non-technical adoption problem.
Whether the concurrency model in §9C is sufficient for real workloads once it's actually implemented and stress-tested, versus needing further primitives nobody has anticipated yet.
Performance versus C/Rust in practice. The mechanisms chosen (LLVM backend, elidable bounds checks, defined-not-checked overflow in fast mode) give a plausible path to competitive performance, but actual benchmarks don't exist until there's a working compiler and real programs to measure.
Treat this section as a living list — update it as milestones land and some of these get resolved (move them to §9's changelog-style notes) while new ones inevitably surface.build it 
</USER_REQUEST>
<ADDITIONAL_METADATA>
The current local time is: 2026-08-02T14:53:56+05:30.
</ADDITIONAL_METADATA>
<USER_SETTINGS_CHANGE>
The user changed setting `Model Selection` from None to Gemini 3.5 Flash (Medium). No need to comment on this change if the user doesn't ask about it. If reporting what model you are, please use a human readable name instead of the exact string.
</USER_SETTINGS_CHANGE>
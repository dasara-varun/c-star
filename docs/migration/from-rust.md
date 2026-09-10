# Migrating from Rust to C*

C* share's Rust's core value proposition of memory safety via ownership without runtime garbage collection, but simplifies developer experience by replacing Rust's explicit lifetime annotations with local compiler-inferred lifetimes.

## Syntax Mapping

| Rust | C* |
|---|-----|
| `cargo build` / `cargo run` | `orbit build` |
| `Cargo.toml` | `Orbit.toml` |
| `fn foo<'a>(x: &'a i32)` | `fn foo(x: &i32)` (lifetimes are inferred) |
| `Box<T>` | Owned values (automatic scope drop) |
| `unsafe { ... }` | `raw { ... }` (pointer arithmetic & dereferences) |
| `Arc<Mutex<T>>` | `Shared<T>` |
| `println!("{}", x)` | `print(x)` |
| `Result<T, E>` / `Option<T>` | `Result<T, E>` / `Option<T>` (same) |
| `?` operator | `?` operator (same) |

## Implicit Lifetimes

The single biggest difference between Rust and C* is that C* does not require explicit lifetime annotations (`'a`) in common code. The compiler infers lifetimes based on reference lifetimes within scope:

```rust
// Rust
struct Borrowed<'a> {
    val: &'a i32,
}

fn get_val<'a>(b: &'a Borrowed<'a>) -> &'a i32 {
    b.val
}
```

```cstar
// C*
struct Borrowed {
    val: &i32,
}

fn get_val(b: &Borrowed) -> &i32 {
    return b.val
}
```

## Raw Blocks vs Unsafe

C* uses `raw { ... }` blocks rather than `unsafe { ... }`. Within a raw block, raw pointers (`raw *T`) can be created and dereferenced:

```rust
// Rust
fn raw_access(ptr: *const u8) {
    unsafe {
        let val = *ptr;
    }
}
```

```cstar
// C*
fn raw_access(ptr: raw *u8) {
    raw {
        let val = *ptr
    }
}
```

## Cargo vs Orbit

`orbit` acts as the package manager and dependency resolution orchestrator for C*, mirroring `cargo`:

- **Initialize**: `orbit init` creates a project structure with `Orbit.toml` and a template source.
- **Dependencies**: Adding a dependency via `orbit add <name>` automatically resolves paths and updates the dependency tree in `Orbit.lock`.
- **Linking C**: `orbit add --c-header <path>` generates C* headers from C binaries automatically using `cstar-bind`.

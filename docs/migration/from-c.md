# Migrating from C to C*

C* retains C-family syntax while adding ownership-based memory safety.

## Syntax Mapping

| C | C* |
|---|-----|
| `#include "foo.h"` | `use foo` |
| `int main()` | `fn main() -> i32` |
| `malloc` / `free` | Owned values (automatic drop) |
| `char*` | `string` or `raw *u8` in `raw{}` |
| `struct Foo { ... }` | `struct Foo { ... }` (same) |
| `void foo(int x)` | `fn foo(x: i32)` |
| `return 0;` | `return 0` (no semicolon required) |
| `if (x) { ... }` | `if x { ... }` |
| `for (i=0; i<n; i++)` | `for i in range { ... }` |
| `switch (x) { ... }` | `match x { ... => ... }` |
| `NULL` | `Option<T>` / `None` |
| `errno` / error codes | `Result<T, E>` |

## Memory Management

```c
// C
char* buf = malloc(100);
strcpy(buf, "hello");
free(buf);
```

```cstar
// C* — owned, no manual free
let buf = "hello"  // string is managed
// buf is dropped automatically at end of scope
```

For C interop, use `raw{}` blocks:

```cstar
use core.raw_ffi

fn read_c_string(ptr: raw *u8, len: usize) -> []u8 {
    raw {
        return slice_from_raw_parts(ptr, len)
    }
}
```

## Raw-Heavy On-Ramp

When porting existing C code, you can start with almost everything inside `raw{}` blocks:

```cstar
fn legacy_port() {
    raw {
        // Direct C-style code here
        let ptr: raw *i32 = 0
        // ...
    }
}
```

Then incrementally move functions out of `raw{}` as you adopt safe idioms.

## Generating Bindings

```powershell
cstar-bind mylib.h -o mylib.cx
```

This generates C* FFI declarations for C functions.

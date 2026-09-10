# C* Compiler Error Codes

Every diagnostic emitted by `starc` uses a structured error code. This document lists each code with a minimal reproducing example.

## Parse Errors (E000x)

### E0001 — Unexpected token
```cstar
fn main() {
    let x = 10
    let y = 20  // missing something before this on same line if intended
}
```

### E0002 — Invalid pub on impl block
```cstar
pub impl Point {
    fn new() -> Point { Point { x: 0.0, y: 0.0 } }
}
```

### E0003 — Expected item at module level
```cstar
module mymod
let x = 10
```

### E0004 — Expected expression
```cstar
fn main() {
    let x = ;
}
```

### E0005 — Unclosed delimiter
```cstar
fn main() {
    if true {
        print("hello")
    // missing closing brace
```

## Type Errors (E010x)

### E0101 — Variable redefinition
```cstar
fn main() {
    let x: i32 = 10
    let x: i32 = 20
}
```

### E0102 — Type mismatch
```cstar
fn main() {
    let x: i32 = "hello"
}
```

### E0103 — Unknown variable
```cstar
fn main() {
    print(unknown_var)
}
```

### E0104 — Return type mismatch
```cstar
fn get_num() -> i32 {
    return "not a number"
}
```

### E0105 — Missing return value
```cstar
fn get_num() -> i32 {
    print("no return")
}
```

### E0106 — Return in void function
```cstar
fn greet() {
    return 42
}
```

### E0107 — While condition must be bool
```cstar
fn main() {
    while 42 {
        print("loop")
    }
}
```

### E0108 — Use of uninitialized variable
```cstar
fn main() {
    let x: i32
    print(x)
}
```

### E0109 — Unknown identifier
```cstar
fn main() {
    unknown_function()
}
```

### E0110 — Unknown struct
```cstar
fn main() {
    let p = UnknownStruct { x: 0 }
}
```

### E0111 — Raw pointer dereference outside raw block
```cstar
fn main() {
    let ptr: raw *i32 = 0
    let val: i32 = *ptr
}
```

### E0112 — Cannot take address of rvalue
```cstar
fn main() {
    let p = &(1 + 2)
}
```

### E0113 — Receiver is not a struct
```cstar
fn main() {
    let x: i32 = 10
    x.field
}
```

### E0114 — Unknown field
```cstar
struct Point { x: f64, y: f64 }
fn main() {
    let p = Point { x: 0.0, y: 0.0 }
    p.unknown
}
```

### E0115 — Method receiver is not a struct
```cstar
fn main() {
    let x: i32 = 10
    x.method()
}
```

### E0116 — Unknown method
```cstar
struct Point { x: f64, y: f64 }
fn main() {
    let p = Point { x: 0.0, y: 0.0 }
    p.unknown()
}
```

### E0117 — Wrong number of arguments
```cstar
fn add(a: i32, b: i32) -> i32 { a + b }
fn main() {
    add(1)
}
```

### E0118 — Argument type mismatch
```cstar
fn greet(name: string) { print(name) }
fn main() {
    greet(42)
}
```

### E0119 — If condition must be bool
```cstar
fn main() {
    if 42 {
        print("yes")
    }
}
```

### E0120 — Callee is not a function
```cstar
fn main() {
    let x: i32 = 10
    x()
}
```

### E0121 — Generic argument count mismatch
```cstar
pub fn id<T>(x: T) -> T { x }
fn main() {
    id(1, 2)
}
```

### E0122 — Cannot infer generic type
```cstar
pub fn id<T>(x: T) -> T { x }
fn main() {
    id()
}
```

### E0123 — Unknown function
```cstar
fn main() {
    nonexistent()
}
```

### E0124 — Match arm type mismatch
```cstar
fn main() {
    match Ok(1) {
        Ok(v) => print(v),
        Err(e) => 42,
    }
}
```

### E0125 — Non-exhaustive match
```cstar
fn main() {
    match Ok(1) {
        Ok(v) => print(v),
    }
}
```

### E0126 — Invalid match pattern
```cstar
fn main() {
    match 42 {
        1 => print("one"),
    }
}
```

### E0127 — Invalid struct pattern
```cstar
struct Point { x: f64, y: f64 }
fn main() {
    let p = Point { x: 0.0, y: 0.0 }
    match p {
        Point { x, z } => print(x),
    }
}
```

### E0128 — If condition must be bool (expression form)
```cstar
fn main() {
    let x = if 42 { 1 } else { 2 }
}
```

### E0129 — Invalid use of ? operator
```cstar
fn main() {
    let x = 42?
}
```

## Ownership Errors (E020x)

### E0203 — Use of moved value
```cstar
struct Point { x: f64, y: f64 }
fn consume(p: Point) {}
fn main() {
    let p1 = Point { x: 0.0, y: 0.0 }
    consume(p1)
    let p2 = p1  // error: p1 was moved
}
```

**Suggested fix:** Borrow instead of moving: `consume(&p1)` or clone if Copy is implemented.

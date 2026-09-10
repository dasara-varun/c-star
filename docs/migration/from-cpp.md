# Migrating from C++ to C*

C* maintains C-family surface syntax and grouping of behavior (methods) with data, but replaces C++ constructor/destructor and template complexity with compile-time monomorphized generics and lifetime/ownership safety.

## Syntax Mapping

| C++ | C* |
|---|-----|
| `#include <iostream>` | `use core` |
| `class Point { public: f64 x; ... };` | `struct Point { pub x: f64, ... }` |
| `Point::Point(f64 x) : x(x) {}` | `impl Point { pub fn new(x: f64) -> Point { Point { x: x } } }` |
| `Point::distance(const Point& other)` | `impl Point { pub fn distance(self, other: &Point) -> f64` |
| `std::unique_ptr<T>` | Owned values (automatic scope-based drop) |
| `std::shared_ptr<T>` | `Shared<T>` (mutex-backed synchronization wrapper) |
| `template<typename T> T max(T a, T b)` | `fn max<T>(a: T, b: T) -> T` |
| `try { ... } catch (const std::exception& e)` | `match func() { Ok(v) => ... , Err(e) => ... }` |
| `obj.method()?` | `obj.method()?` (unwrap or bubble up) |
| `nullptr` | `Option<T>` / `None` |

## Structs and Impls

Unlike C++ where `class` and `struct` are almost identical except for default visibility, C* separates data declaration from method implementation:

```cpp
// C++
class Point {
private:
    double x, y;
public:
    Point(double x, double y) : x(x), y(y) {}
    double get_x() const { return x; }
};
```

```cstar
// C*
pub struct Point {
    x: f64,
    y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Point {
        Point { x: x, y: y }
    }

    pub fn get_x(self) -> f64 {
        return self.x
    }
}
```

## Error Handling

C++ uses exceptions, which can introduce hidden control flow paths and run-time overhead. C* uses explicit `Result<T, E>` types alongside the `?` short-circuiting operator:

```cpp
// C++
double divide(double a, double b) {
    if (b == 0) throw std::invalid_argument("Division by zero");
    return a / b;
}
```

```cstar
// C*
pub fn divide(a: f64, b: f64) -> Result<f64, string> {
    if b == 0.0 {
        return Err("Division by zero")
    }
    return Ok(a / b)
}
```

## Templates vs Generics

C++ templates instantiate code during compilation but allow arbitrary duck-typing, which can lead to long error messages. C* generics are compile-time monomorphized, resolving types cleanly before code generation:

```cpp
// C++
template<typename T>
T find_max(T a, T b) {
    return (a > b) ? a : b;
}
```

```cstar
// C*
pub fn find_max<T>(a: T, b: T) -> T {
    if a > b {
        return a
    }
    return b
}
```

## C++ Interoperability

C++ name-mangling and object layout are highly compiler-dependent. 

- **Direct approach**: Expose C++ logic using a flat C API bridge (`extern "C"` functions) and generate bindings via `cstar-bind`.
- **Dedicated bridge**: For direct C++ interop, the post-v1.0 roadmap includes `cstar-cxx-bridge` to map objects and method tables across the compiler boundary.

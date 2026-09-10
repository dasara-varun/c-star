# Design Decisions - C* Compiler

## Newline-Insertion Rule
A newline character terminates a statement (acting as an implicit semicolon) if the preceding token belongs to one of the following categories:
- **Identifiers** (e.g. variable and function names)
- **Literals** (numeric, string, char, boolean)
- **Closing Delimiters** (`}`, `)`, `]`)
- **Control Flow Keywords** (`return`, `break`, `continue`)

If the line is clearly incomplete, meaning the last token is:
- A trailing binary operator (e.g., `+`, `-`, `*`, `/`, `=`, `==`, `<`, `&&`, `||`, etc.)
- A unary operator (e.g., `&`)
- A comma `,`
- An opening delimiter (`{`, `(`, `[`)
- A return type arrow `->`

Then no virtual statement terminator is inserted, and the statement is allowed to continue onto the next line.

## Workspace Layout
We chose to implement the compiler as a multi-package Cargo workspace located at the root of `e:\c star`:
- `compiler/` containing `starc` (the compiler CLI and driver)
- `orbit/` containing `orbit` (the package manager and build orchestrator)
- `stdlib/` containing the standard library sources
- `tests/` containing the test suite

This allows modular development of both compiler features and tooling.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

fn find_starc() -> Option<PathBuf> {
    // 1. Check current executable directory
    if let Ok(current_exe) = env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let starc_path = dir.join(if cfg!(windows) { "starc.exe" } else { "starc" });
            if starc_path.exists() {
                return Some(starc_path);
            }
        }
    }
    // 2. Check workspace target/debug directory
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let debug_starc = workspace_root.join("target").join("debug").join(if cfg!(windows) { "starc.exe" } else { "starc" });
    if debug_starc.exists() {
        return Some(debug_starc);
    }
    // 3. Fallback to path lookup
    if let Ok(path_var) = env::var("PATH") {
        for path in env::split_paths(&path_var) {
            let p = path.join(if cfg!(windows) { "starc.exe" } else { "starc" });
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn compile_and_run(starc_path: &Path, code: &str, temp_dir: &Path) -> Result<String, String> {
    let source_path = temp_dir.join("repl_temp.cx");
    let exe_path = temp_dir.join(if cfg!(windows) { "repl_temp.exe" } else { "repl_temp" });

    if exe_path.exists() {
        let _ = fs::remove_file(&exe_path);
    }

    fs::write(&source_path, code).map_err(|e| format!("Failed to write temp source: {}", e))?;

    let mut build_cmd = Command::new(starc_path);
    build_cmd.arg("build")
             .arg(&source_path)
             .arg("-o")
             .arg(&exe_path);

    // Forward LLVM and system paths
    let env_path = format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default());
    build_cmd.env("PATH", &env_path);

    let build_output = build_cmd.output().map_err(|e| format!("Failed to run compiler: {}", e))?;

    if !build_output.status.success() {
        let stderr = String::from_utf8_lossy(&build_output.stderr);
        let stdout = String::from_utf8_lossy(&build_output.stdout);
        let mut err_msg = String::new();
        if !stdout.is_empty() {
            err_msg.push_str(&stdout);
        }
        if !stderr.is_empty() {
            err_msg.push_str(&stderr);
        }
        return Err(err_msg.trim().to_string());
    }

    // Run the compiled executable
    let mut run_cmd = Command::new(&exe_path);
    run_cmd.env("PATH", &env_path);
    let run_output = run_cmd.output().map_err(|e| format!("Failed to run REPL output: {}", e))?;

    let run_stdout = String::from_utf8_lossy(&run_output.stdout);
    let run_stderr = String::from_utf8_lossy(&run_output.stderr);

    if !run_output.status.success() {
        return Err(format!("Execution panicked/failed:\nstdout: {}\nstderr: {}", run_stdout.trim(), run_stderr.trim()));
    }

    Ok(run_stdout.to_string())
}

fn build_cstar_code(decls: &[String], stmts: &[String], current_expr: Option<&str>) -> String {
    let mut code = String::new();
    code.push_str("module main\n\n");

    for decl in decls {
        code.push_str(decl);
        code.push_str("\n\n");
    }

    code.push_str("fn main() -> i32 {\n");
    for stmt in stmts {
        code.push_str("    ");
        code.push_str(stmt);
        code.push_str("\n");
    }

    if let Some(expr) = current_expr {
        code.push_str("    ");
        code.push_str(expr);
        code.push_str("\n");
    }

    code.push_str("    return 0\n}\n");
    code
}

fn is_declaration(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("fn ") || 
    trimmed.starts_with("pub fn ") ||
    trimmed.starts_with("struct ") ||
    trimmed.starts_with("pub struct ") ||
    trimmed.starts_with("enum ") ||
    trimmed.starts_with("pub enum ") ||
    trimmed.starts_with("impl ") ||
    trimmed.starts_with("use ")
}

fn is_statement(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("let ") || 
    trimmed.starts_with("return ") ||
    (trimmed.contains("=") && !trimmed.contains("==") && !trimmed.contains("<=") && !trimmed.contains(">=") && !trimmed.contains("!="))
}

fn main() {
    println!("C* Interactive REPL (nova v0.1.0)");
    println!("Type 'exit' or 'quit' to exit.");
    println!("--------------------------------");

    let starc_path = match find_starc() {
        Some(p) => p,
        None => {
            eprintln!("Error: Could not find starc compiler binary. Make sure to build it first.");
            process::exit(1);
        }
    };

    let temp_dir = env::temp_dir().join("cstar_repl");
    let _ = fs::create_dir_all(&temp_dir);

    let mut decls = Vec::new();
    let mut stmts = Vec::new();

    loop {
        print!("cx> ");
        let _ = io::stdout().flush();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }

        let trimmed = input.trim();
        if trimmed == "exit" || trimmed == "quit" {
            break;
        }
        if trimmed.is_empty() {
            continue;
        }

        if is_declaration(trimmed) {
            // Test if it compiles as a global declaration
            let mut temp_decls = decls.clone();
            temp_decls.push(trimmed.to_string());
            let code = build_cstar_code(&temp_decls, &stmts, None);

            match compile_and_run(&starc_path, &code, &temp_dir) {
                Ok(_) => {
                    decls.push(trimmed.to_string());
                    println!("Added declaration.");
                }
                Err(e) => {
                    println!("Compile Error:\n{}", e);
                }
            }
        } else if is_statement(trimmed) {
            // Test if it compiles as a statement inside main
            let mut temp_stmts = stmts.clone();
            temp_stmts.push(trimmed.to_string());
            let code = build_cstar_code(&decls, &temp_stmts, None);

            match compile_and_run(&starc_path, &code, &temp_dir) {
                Ok(_) => {
                    stmts.push(trimmed.to_string());
                }
                Err(e) => {
                    println!("Compile Error:\n{}", e);
                }
            }
        } else {
            // It is an expression. Try printing it.
            let print_expr = format!("print({})", trimmed);
            let code_with_print = build_cstar_code(&decls, &stmts, Some(&print_expr));

            match compile_and_run(&starc_path, &code_with_print, &temp_dir) {
                Ok(output) => {
                    print!("{}", output);
                }
                Err(_) => {
                    // Try compiling it as a plain statement without wrapping in print (e.g. expression returning void or custom statement)
                    let code_plain = build_cstar_code(&decls, &stmts, Some(trimmed));
                    match compile_and_run(&starc_path, &code_plain, &temp_dir) {
                        Ok(output) => {
                            if !output.is_empty() {
                                print!("{}", output);
                            }
                        }
                        Err(e) => {
                            println!("Compile Error:\n{}", e);
                        }
                    }
                }
            }
        }
    }

    // Clean up temp dir
    let _ = fs::remove_dir_all(&temp_dir);
}

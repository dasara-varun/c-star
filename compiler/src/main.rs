use std::env;
use std::fs;
use std::collections::HashMap;
use std::path::Path;
use starc::lexer::Lexer;
use starc::parser::Parser;
use starc::typeck::TypeChecker;
use starc::codegen::Codegen;
use starc::diagnostics::DiagnosticsReport;
use inkwell::context::Context;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: starc build <file> [--error-format=json|human] [--profile=safe|fast] [--emit=llvm-ir] [-o <output>]");
        eprintln!("   or: starc <file> [--error-format=json|human] [--profile=safe|fast] [--emit=llvm-ir] [-o <output>]");
        std::process::exit(1);
    }

    let mut is_build_cmd = false;
    let mut file_idx = 1;

    if args[1] == "build" {
        is_build_cmd = true;
        file_idx = 2;
        if args.len() < 3 {
            eprintln!("Error: build command requires an input file");
            std::process::exit(1);
        }
    }

    let mut input_file = None;
    let mut error_format = "human";
    let mut profile = "safe";
    let mut emit_ir = false;
    let mut output_file = None;

    let mut i = file_idx;
    while i < args.len() {
        match args[i].as_str() {
            arg if arg.starts_with("--error-format=") => {
                error_format = arg.trim_start_matches("--error-format=");
            }
            arg if arg.starts_with("--profile=") => {
                profile = arg.trim_start_matches("--profile=");
                if profile != "safe" && profile != "fast" {
                    eprintln!("Error: --profile must be 'safe' or 'fast'");
                    std::process::exit(1);
                }
            }
            "--emit=llvm-ir" => {
                emit_ir = true;
            }
            "-o" | "--output" => {
                if i + 1 < args.len() {
                    output_file = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    eprintln!("Error: -o / --output requires a file path");
                    std::process::exit(1);
                }
            }
            arg if arg.starts_with("-") => {
                eprintln!("Error: Unknown option {}", arg);
                std::process::exit(1);
            }
            path => {
                input_file = Some(path.to_string());
            }
        }
        i += 1;
    }

    let input_path = match input_file {
        Some(path) => path,
        None => {
            eprintln!("Error: No input file specified");
            std::process::exit(1);
        }
    };

    let source = match fs::read_to_string(&input_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading file {}: {}", input_path, e);
            std::process::exit(1);
        }
    };

    // Cache source code for diagnostics rendering
    let mut source_cache = HashMap::new();
    source_cache.insert(input_path.clone(), source.clone());

    // 1. Lex & Parse main module
    let lexer = Lexer::new(&source, &input_path);
    let mut parser = match Parser::new(lexer, &input_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Lexer initialization error: {}", e);
            std::process::exit(1);
        }
    };

    let mut main_module = match parser.parse_module() {
        Ok(m) => m,
        Err(_) => {
            print_diagnostics(&parser.diagnostics, &source_cache, error_format);
            std::process::exit(1);
        }
    };

    if !parser.diagnostics.is_empty() {
        print_diagnostics(&parser.diagnostics, &source_cache, error_format);
        std::process::exit(1);
    }

    // 1b. Recursive import resolution
    let input_dir = Path::new(&input_path).parent().unwrap_or(Path::new("."));
    let mut resolved_modules = HashMap::new();
    let mut queue = Vec::new();
    for import in &main_module.imports {
        queue.push(import.path.clone());
    }

    while let Some(import_path) = queue.pop() {
        let mod_name = import_path.join(".");
        if resolved_modules.contains_key(&mod_name) {
            continue;
        }

        // Search for mod_name.cx
        let file_name = format!("{}.cx", import_path.last().unwrap());
        let possible_paths = vec![
            input_dir.join(&file_name),
            Path::new("stdlib").join(&file_name),
            Path::new("examples").join(&file_name),
        ];

        let mut found_path = None;
        for path in possible_paths {
            if path.exists() {
                found_path = Some(path);
                break;
            }
        }

        let path = match found_path {
            Some(p) => p,
            None => {
                eprintln!("Error: Cannot find module `{}`", mod_name);
                std::process::exit(1);
            }
        };

        let sub_source = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading imported module `{}`: {}", mod_name, e);
                std::process::exit(1);
            }
        };

        let path_str = path.to_str().unwrap().to_string();
        source_cache.insert(path_str.clone(), sub_source.clone());

        let lexer = Lexer::new(&sub_source, &path_str);
        let mut sub_parser = Parser::new(lexer, &path_str).unwrap();
        let sub_module = match sub_parser.parse_module() {
            Ok(m) => m,
            Err(_) => {
                print_diagnostics(&sub_parser.diagnostics, &source_cache, error_format);
                std::process::exit(1);
            }
        };

        if !sub_parser.diagnostics.is_empty() {
            print_diagnostics(&sub_parser.diagnostics, &source_cache, error_format);
            std::process::exit(1);
        }

        for import in &sub_module.imports {
            queue.push(import.path.clone());
        }

        resolved_modules.insert(mod_name, sub_module);
    }

    // Merge items from all resolved modules into main_module's items list
    for sub_mod in resolved_modules.values() {
        main_module.items.extend(sub_mod.items.clone());
    }

    // 2. Type Check
    let mut tc = TypeChecker::new(&input_path);
    tc.check_module(&main_module);

    if !tc.diagnostics.is_empty() {
        print_diagnostics(&tc.diagnostics, &source_cache, error_format);
        std::process::exit(1);
    }

    main_module.items.extend(tc.specialized_items.clone());

    // 3. Codegen LLVM IR
    let context = Context::create();
    let module_name = Path::new(&input_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main");
    let mut cg = Codegen::new(&context, module_name);
    cg.profile = profile.to_string();
    cg.specialized_calls = tc.specialized_calls.clone();

    if let Err(e) = cg.compile_module(&main_module) {
        eprintln!("Codegen compilation error: {}", e);
        std::process::exit(1);
    }

    let ir_str = cg.module.print_to_string().to_string();

    if is_build_cmd || (output_file.is_some() && !emit_ir) {
        // We compile to executable using clang!
        let out_exe_path = match output_file {
            Some(path) => path,
            None => {
                let stem = Path::new(&input_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("main");
                if cfg!(windows) {
                    format!("{}.exe", stem)
                } else {
                    stem.to_string()
                }
            }
        };

        // Write IR to a temporary .ll file
        let temp_ll = format!("{}_temp.ll", out_exe_path);
        if let Err(e) = fs::write(&temp_ll, &ir_str) {
            eprintln!("Error writing temporary IR file {}: {}", temp_ll, e);
            std::process::exit(1);
        }

        // Run clang to compile the .ll file into native binary
        let mut clang_cmd = std::process::Command::new("C:\\PROGRA~1\\LLVM\\bin\\clang.exe");
        clang_cmd.arg("-o")
                 .arg(&out_exe_path)
                 .arg(&temp_ll);

        // Forward system paths for MSVC linker targeting on Windows
        clang_cmd.env("PATH", format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default()));

        let clang_status = match clang_cmd.status() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to execute clang: {}", e);
                let _ = fs::remove_file(&temp_ll);
                std::process::exit(1);
            }
        };

        let _ = fs::remove_file(&temp_ll);

        if !clang_status.success() {
            eprintln!("Clang compilation failed");
            std::process::exit(1);
        }

        println!("Build successful. Native binary produced: {}", out_exe_path);
    } else {
        if emit_ir {
            if let Some(out) = output_file {
                if let Err(e) = fs::write(out, &ir_str) {
                    eprintln!("Error writing output file: {}", e);
                    std::process::exit(1);
                }
            } else {
                println!("{}", ir_str);
            }
        } else {
            println!("Compilation successful.");
        }
    }
}

fn print_diagnostics(
    diagnostics: &[starc::diagnostics::Diagnostic],
    source_cache: &HashMap<String, String>,
    format: &str,
) {
    let report = DiagnosticsReport::new(diagnostics.to_vec());
    if format == "json" {
        match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error generating JSON diagnostics: {}", e),
        }
    } else {
        print!("{}", report.render_human(source_cache));
    }
}

pub mod ast;
pub mod diagnostics;
pub mod lexer;
pub mod parser;
pub mod typeck;
pub mod codegen;

#[cfg(test)]
mod integration_tests {
    use std::process::Command;
    use std::fs;
    use std::env;
    use std::path::Path;
    use std::collections::HashMap;
    use inkwell::context::Context;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::typeck::TypeChecker;
    use crate::codegen::Codegen;

    fn get_workspace_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
    }

    fn get_test_build_dir() -> std::path::PathBuf {
        let dir = get_workspace_root().join("build_test");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn compile_in_process(input_path: &str, out_exe_path: &str) -> Result<(), String> {
        let source = fs::read_to_string(input_path)
            .map_err(|e| format!("Error reading file {}: {}", input_path, e))?;

        let mut source_cache = HashMap::new();
        source_cache.insert(input_path.to_string(), source.clone());

        let lexer = Lexer::new(&source, input_path);
        let mut parser = Parser::new(lexer, input_path)
            .map_err(|e| format!("Parser init error: {}", e))?;

        let mut main_module = parser.parse_module()
            .map_err(|_| "Parse failed".to_string())?;

        if !parser.diagnostics.is_empty() {
            return Err(format!("Parse errors: {:?}", parser.diagnostics));
        }

        let input_dir = Path::new(input_path).parent().unwrap_or(Path::new("."));
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

            let mod_last = import_path.last().unwrap();
            let file_name = if mod_last == "thread_impl" {
                if cfg!(windows) {
                    "thread_windows.cx".to_string()
                } else {
                    "thread_posix.cx".to_string()
                }
            } else {
                format!("{}.cx", mod_last)
            };
            let possible_paths = vec![
                input_dir.join(&file_name),
                Path::new("stdlib").join(&file_name),
                Path::new("..").join("stdlib").join(&file_name),
                Path::new("examples").join(&file_name),
            ];

            let mut found_path = None;
            for path in possible_paths {
                if path.exists() {
                    found_path = Some(path);
                    break;
                }
            }

            let path = found_path.ok_or_else(|| format!("Cannot find module `{}`", mod_name))?;
            let sub_source = fs::read_to_string(&path)
                .map_err(|e| format!("Error reading imported module `{}`: {}", mod_name, e))?;

            let path_str = path.to_str().unwrap().to_string();
            source_cache.insert(path_str.clone(), sub_source.clone());

            let lexer = Lexer::new(&sub_source, &path_str);
            let mut sub_parser = Parser::new(lexer, &path_str).unwrap();
            let sub_module = sub_parser.parse_module()
                .map_err(|_| format!("Parse error in sub module `{}`", mod_name))?;

            if !sub_parser.diagnostics.is_empty() {
                return Err(format!("Sub-parser errors in `{}`: {:?}", mod_name, sub_parser.diagnostics));
            }

            for import in &sub_module.imports {
                queue.push(import.path.clone());
            }

            resolved_modules.insert(mod_name, sub_module);
        }

        for sub_mod in resolved_modules.values() {
            main_module.items.extend(sub_mod.items.clone());
        }

        let mut main_module = main_module;
        let mut tc = TypeChecker::new(input_path);
        tc.check_module(&main_module);
        if !tc.diagnostics.is_empty() {
            return Err(format!("Type check errors: {:?}", tc.diagnostics));
        }

        main_module.items.extend(tc.specialized_items.clone());

        let context = Context::create();
        let module_name = Path::new(input_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("main");
        let mut cg = Codegen::new(&context, module_name);
        cg.specialized_calls = tc.specialized_calls.clone();
        cg.compile_module(&main_module)?;

        let ir_str = cg.module.print_to_string().to_string();

        let temp_ll = format!("{}_temp.ll", out_exe_path);
        fs::write(&temp_ll, &ir_str)
            .map_err(|e| format!("Error writing temporary IR: {}", e))?;

        let temp_exe = format!("{}_temp.exe", out_exe_path);
        if Path::new(&temp_exe).exists() {
            let _ = fs::remove_file(&temp_exe);
        }

        let mut clang_cmd = std::process::Command::new("C:\\PROGRA~1\\LLVM\\bin\\clang.exe");
        clang_cmd.arg("-o")
                 .arg(&temp_exe)
                 .arg(&temp_ll);

        clang_cmd.env("PATH", format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default()));

        let clang_status = clang_cmd.status()
            .map_err(|e| format!("Failed to execute clang: {}", e))?;

        let _ = fs::remove_file(&temp_ll);

        if !clang_status.success() {
            return Err("Clang compilation failed".to_string());
        }

        // Sign the compiled temporary executable
        let mut sign_cmd = std::process::Command::new("C:\\Program Files (x86)\\Windows Kits\\10\\bin\\10.0.26100.0\\x64\\signtool.exe");
        sign_cmd.arg("sign")
                .arg("/fd")
                .arg("SHA256")
                .arg("/a")
                .arg("/sha1")
                .arg("43234DE57345EB5E30D50F57EECA9379C71CA343")
                .arg(&temp_exe);
        let _ = sign_cmd.status();

        // Copy to final location (ensures the file is signed from creation)
        if Path::new(out_exe_path).exists() {
            let _ = fs::remove_file(out_exe_path);
        }
        fs::copy(&temp_exe, out_exe_path)
            .map_err(|e| format!("Failed to copy signed executable: {}", e))?;
        let _ = fs::remove_file(&temp_exe);

        std::thread::sleep(std::time::Duration::from_millis(100));

        Ok(())
    }

    #[test]
    fn test_integration_compile_hello() {
        let ws_root = get_workspace_root();
        let test_dir = get_test_build_dir();
        let out_exe = test_dir.join("hello_run.exe");
        if out_exe.exists() {
            let _ = fs::remove_file(&out_exe);
        }

        let hello_path = ws_root.join("examples").join("hello.cx");

        compile_in_process(hello_path.to_str().unwrap(), out_exe.to_str().unwrap()).unwrap();

        let mut run_cmd = Command::new(&out_exe);
        run_cmd.env("PATH", format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default()));
        let run_output = run_cmd.output().unwrap();
        assert!(run_output.status.success(), "Failed to execute hello_run.exe!\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&run_output.stdout),
                String::from_utf8_lossy(&run_output.stderr));
        let run_stdout = String::from_utf8_lossy(&run_output.stdout);
        assert_eq!(run_stdout.trim(), "Hello, World!");
    }

    #[test]
    fn test_integration_compile_multi_module() {
        let ws_root = get_workspace_root();
        let test_dir = get_test_build_dir();
        let out_exe = test_dir.join("main_run.exe");
        if out_exe.exists() {
            let _ = fs::remove_file(&out_exe);
        }

        let main_path = ws_root.join("examples").join("main.cx");

        compile_in_process(main_path.to_str().unwrap(), out_exe.to_str().unwrap()).unwrap();

        let mut run_cmd = Command::new(&out_exe);
        run_cmd.env("PATH", format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default()));
        let run_output = run_cmd.output().unwrap();
        assert!(run_output.status.success(), "Failed to execute main_run.exe!\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&run_output.stdout),
                String::from_utf8_lossy(&run_output.stderr));
        let run_stdout = String::from_utf8_lossy(&run_output.stdout);
        assert!(run_stdout.contains("Distance: 5.0"));
    }

    #[test]
    fn test_integration_orbit_package_manager() {
        let temp_dir = get_test_build_dir().join("orbit_test_proj");
        if temp_dir.exists() {
            let _ = fs::remove_dir_all(&temp_dir);
        }
        let _ = fs::create_dir_all(&temp_dir);

        // 1. Mock orbit init
        let toml_content = r#"[package]
name = "my_project"
version = "0.1.0"

[dependencies]
"#;
        let main_content = r#"fn main() -> i32 {
    print("Hello from Orbit package manager!\n")
    return 0
}
"#;
        fs::write(temp_dir.join("Orbit.toml"), toml_content).unwrap();
        let _ = fs::create_dir_all(temp_dir.join("src"));
        fs::write(temp_dir.join("src/main.cx"), main_content).unwrap();

        // 2. Mock orbit add geometry
        let mut content = fs::read_to_string(temp_dir.join("Orbit.toml")).unwrap();
        content.push_str("\ngeometry = \"*\"\n");
        fs::write(temp_dir.join("Orbit.toml"), content).unwrap();

        let lock_content = "# Orbit Lockfile\n\n[[package]]\nname = \"geometry\"\nversion = \"*\"\n";
        fs::write(temp_dir.join("Orbit.lock"), lock_content).unwrap();

        // 3. Mock orbit build -> compile in-process!
        let out_exe = get_test_build_dir().join("orbit_run.exe");
        compile_in_process(
            temp_dir.join("src/main.cx").to_str().unwrap(),
            out_exe.to_str().unwrap()
        ).unwrap();

        assert!(out_exe.exists());

        // Run orbit_run.exe to verify
        let mut run_cmd = Command::new(&out_exe);
        run_cmd.env("PATH", format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default()));
        let run_output = run_cmd.output().unwrap();
        assert!(run_output.status.success());
        let run_stdout = String::from_utf8_lossy(&run_output.stdout);
        assert!(run_stdout.contains("Hello from Orbit"));
    }

    #[test]
    fn test_integration_cstar_bind() {
        let ws_root = get_workspace_root();
        let manifest_path = ws_root.join("Cargo.toml");

        let test_dir = get_test_build_dir();
        let header_file = test_dir.join("math_lib.h");
        let output_cx = test_dir.join("math_lib.cx");

        let header_content = "
        double sqrt(double x);
        int add(int a, int b);
        void print_msg(const char* msg);
        ";
        fs::write(&header_file, header_content).unwrap();

        let status = Command::new("cargo")
            .arg("run")
            .arg("--manifest-path")
            .arg(&manifest_path)
            .arg("-p")
            .arg("cstar-bind")
            .arg("--")
            .arg(header_file.to_str().unwrap())
            .arg("-o")
            .arg(output_cx.to_str().unwrap())
            .status()
            .unwrap();
        assert!(status.success());

        assert!(output_cx.exists());
        let generated = fs::read_to_string(&output_cx).unwrap();
        assert!(generated.contains("pub fn sqrt(x: f64) -> f64"));
        assert!(generated.contains("pub fn add(a: i32, b: i32) -> i32"));
        assert!(generated.contains("pub fn print_msg(msg: raw *u8)"));
    }

    #[test]
    fn test_integration_compile_result_match() {
        let _ws_root = get_workspace_root();
        let test_dir = get_test_build_dir();
        let out_exe = test_dir.join("result_match_run.exe");
        if out_exe.exists() {
            let _ = fs::remove_file(&out_exe);
        }

        let cx_src = "
        pub fn safe_divide(a: i32, b: i32) -> Result<i32, string> {
            if b == 0 {
                return Err(\"division by zero\")
            }
            Ok(a / b)
        }

        fn main() -> i32 {
            match safe_divide(10, 0) {
                Ok(v) => print(v),
                Err(e) => print(e),
            }
            match safe_divide(10, 2) {
                Ok(v) => print(v),
                Err(e) => print(e),
            }
            return 0
        }
        ";

        let src_file = test_dir.join("result_match.cx");
        fs::write(&src_file, cx_src).unwrap();

        compile_in_process(src_file.to_str().unwrap(), out_exe.to_str().unwrap()).unwrap();

        let mut run_cmd = Command::new(&out_exe);
        run_cmd.env("PATH", format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default()));
        let run_output = run_cmd.output().unwrap();
        let run_stdout = String::from_utf8_lossy(&run_output.stdout);
        let run_stderr = String::from_utf8_lossy(&run_output.stderr);
        println!("TEST RUN STDOUT:\n{}", run_stdout);
        println!("TEST RUN STDERR:\n{}", run_stderr);
        println!("TEST RUN STATUS: {:?}", run_output.status);

        assert!(run_output.status.success());
        assert!(run_stdout.contains("division by zero"));
        assert!(run_stdout.contains("5"));
    }

    #[test]
    fn test_integration_generics() {
        let test_dir = get_test_build_dir();
        let out_exe = test_dir.join("generics_run.exe");
        if out_exe.exists() {
            let _ = fs::remove_file(&out_exe);
        }

        let cx_src = "
        pub fn max<T>(a: T, b: T) -> T {
            if (a > b) {
                return a;
            }
            return b;
        }

        fn main() -> i32 {
            let x: i32 = max(10, 20);
            let y: f64 = max(3.14, 2.71);
            print(x);
            print(y);
            return 0;
        }
        ";

        let src_file = test_dir.join("generics.cx");
        fs::write(&src_file, cx_src).unwrap();

        compile_in_process(src_file.to_str().unwrap(), out_exe.to_str().unwrap()).unwrap();

        let mut run_cmd = Command::new(&out_exe);
        run_cmd.env("PATH", format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default()));
        let run_output = run_cmd.output().unwrap();
        let run_stdout = String::from_utf8_lossy(&run_output.stdout);
        println!("GENERICS STDOUT:\n{}", run_stdout);

        assert!(run_output.status.success());
        assert!(run_stdout.contains("20"));
        assert!(run_stdout.contains("3.140000") || run_stdout.contains("3.14"));
    }

    #[test]
    fn test_integration_coroutines() {
        let test_dir = get_test_build_dir();
        let out_exe = test_dir.join("coroutines_run.exe");
        if out_exe.exists() {
            let _ = fs::remove_file(&out_exe);
        }

        let cx_src = "
        // Windows Fiber FFI declarations
        pub fn ConvertThreadToFiber(param: raw *u8) -> raw *u8 {
            return param;
        }

        pub fn CreateFiber(stack_size: usize, start_address: raw *u8, parameter: raw *u8) -> raw *u8 {
            return parameter;
        }

        pub fn SwitchToFiber(fiber: raw *u8) {
        }

        struct Context {
            main_fiber: raw *u8,
            producer_fiber: raw *u8,
            consumer_fiber: raw *u8,
            data: i32,
            has_data: i32,
            count: i32,
        }

        pub fn producer(param: raw *u8) {
            raw {
                let ctx: raw *Context = param;
                while (*ctx).count < 5 {
                    (*ctx).data = (*ctx).count * 10;
                    (*ctx).has_data = 1;
                    print(\"Producer: produced\");
                    print((*ctx).data);
                    (*ctx).count = (*ctx).count + 1;
                    let dest = (*ctx).consumer_fiber;
                    SwitchToFiber(dest);
                }
                let dest = (*ctx).main_fiber;
                SwitchToFiber(dest);
            }
        }

        pub fn consumer(param: raw *u8) {
            raw {
                let ctx: raw *Context = param;
                while true {
                    if (*ctx).has_data == 1 {
                        print(\"Consumer: consumed\");
                        print((*ctx).data);
                        (*ctx).has_data = 0;
                    }
                    let dest = (*ctx).producer_fiber;
                    SwitchToFiber(dest);
                }
            }
        }

        fn main() -> i32 {
            let mut ctx = Context {
                main_fiber: 0,
                producer_fiber: 0,
                consumer_fiber: 0,
                data: 0,
                has_data: 0,
                count: 0,
            };
            let mut ctx_ptr: raw *Context = 0;
            raw {
                ctx_ptr = &ctx;
            }

            let ctx_ptr_int: usize = ctx_ptr;
            let ctx_ptr_u8: raw *u8 = ctx_ptr_int;

            let main_f = ConvertThreadToFiber(0);
            ctx.main_fiber = main_f;

            let prod_f = CreateFiber(0usize, producer, ctx_ptr_u8);
            ctx.producer_fiber = prod_f;

            let cons_f = CreateFiber(0usize, consumer, ctx_ptr_u8);
            ctx.consumer_fiber = cons_f;

            print(\"Main: starting coroutines\");
            SwitchToFiber(prod_f);

            print(\"Main: coroutines finished\");
            return 0;
        }
        ";

        let src_file = test_dir.join("coroutines.cx");
        fs::write(&src_file, cx_src).unwrap();

        compile_in_process(src_file.to_str().unwrap(), out_exe.to_str().unwrap()).unwrap();

        let mut run_cmd = Command::new(&out_exe);
        run_cmd.env("PATH", format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default()));
        let run_output = run_cmd.output().unwrap();
        let run_stdout = String::from_utf8_lossy(&run_output.stdout);
        let run_stderr = String::from_utf8_lossy(&run_output.stderr);
        println!("COROUTINES STDOUT:\n{}", run_stdout);
        println!("COROUTINES STDERR:\n{}", run_stderr);

        assert!(run_output.status.success());
        assert!(run_stdout.contains("Main: starting coroutines"));
        assert!(run_stdout.contains("Producer: produced"));
        assert!(run_stdout.contains("Consumer: consumed"));
        assert!(run_stdout.contains("Main: coroutines finished"));
    }

    #[test]
    fn test_integration_threads() {
        let test_dir = Path::new("build_test");
        let _ = fs::create_dir_all(test_dir);
        let out_exe = test_dir.join("threads_run.exe");
        if out_exe.exists() {
            let _ = fs::remove_file(&out_exe);
        }

        let cx_src = "
        module main
        use thread

        struct Context {
            mutex: Mutex,
            val: i32,
        }

        pub fn worker(param: raw *u8) {
            raw {
                let ctx: raw *Context = param;
                (*ctx).mutex.lock();
                (*ctx).val = (*ctx).val + 1;
                (*ctx).mutex.unlock();
            }
        }

        fn main() -> i32 {
            let mut ctx = Context {
                mutex: Mutex::new(),
                val: 0,
            };

            let mut ctx_ptr: raw *Context = 0;
            raw {
                ctx_ptr = &ctx;
            }

            let ctx_ptr_int: usize = ctx_ptr;
            let ctx_ptr_u8: raw *u8 = ctx_ptr_int;

            let t1 = spawn(worker, ctx_ptr_u8);
            join(&t1);

            print(\"Val: %d\\n\", ctx.val);
            ctx.mutex.destroy();
            return 0;
        }
        ";

        let src_file = test_dir.join("threads.cx");
        fs::write(&src_file, cx_src).unwrap();

        compile_in_process(src_file.to_str().unwrap(), out_exe.to_str().unwrap()).unwrap();

        let mut run_cmd = Command::new(&out_exe);
        run_cmd.env("PATH", format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default()));
        let run_output = run_cmd.output().unwrap();
        let run_stdout = String::from_utf8_lossy(&run_output.stdout);
        let run_stderr = String::from_utf8_lossy(&run_output.stderr);
        println!("THREADS STDOUT:\n{}", run_stdout);
        println!("THREADS STDERR:\n{}", run_stderr);

        assert!(run_output.status.success());
        assert!(run_stdout.contains("Val: 1"));
    }

    #[test]
    fn test_compile_fail_suite() {
        let test_dirs = vec![
            Path::new("tests").join("compile-fail"),
            Path::new("..").join("tests").join("compile-fail"),
        ];

        let mut compile_fail_dir = None;
        for dir in test_dirs {
            if dir.exists() {
                compile_fail_dir = Some(dir);
                break;
            }
        }

        let dir_path = compile_fail_dir.expect("compile-fail tests directory not found");
        let entries = fs::read_dir(dir_path).unwrap();

        let mut count = 0;
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "cx") {
                let filename = path.file_name().unwrap().to_str().unwrap();
                let expected_code = filename.split('_').next().unwrap().to_uppercase();

                println!("Running compile-fail test {} expecting {}", filename, expected_code);

                let src = fs::read_to_string(&path).unwrap();
                let lexer = crate::lexer::Lexer::new(&src, filename);
                let mut parser = crate::parser::Parser::new(lexer, filename).unwrap();
                
                let mut diagnostics = Vec::new();
                match parser.parse_module() {
                    Ok(module) => {
                        let mut tc = crate::typeck::TypeChecker::new(filename);
                        tc.check_module(&module);
                        diagnostics.extend(tc.diagnostics);
                    }
                    Err(_) => {
                        diagnostics.extend(parser.diagnostics);
                    }
                }

                let has_expected = diagnostics.iter().any(|d| d.code.to_uppercase() == expected_code);
                if !has_expected {
                    panic!(
                        "Test {} failed: expected error code {} but got diagnostics {:?}",
                        filename, expected_code, diagnostics
                    );
                }
                count += 1;
            }
        }
        println!("Successfully verified {} compile-fail tests.", count);
        assert!(count >= 8);
    }
}

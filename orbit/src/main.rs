use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    match args[1].as_str() {
        "init" => init_project(),
        "build" => build_project(),
        "add" => {
            if args.len() < 3 {
                eprintln!("Error: add requires a dependency name or path");
                std::process::exit(1);
            }
            let c_header = args.iter().position(|a| a == "--c-header");
            if let Some(idx) = c_header {
                if idx + 1 < args.len() {
                    add_c_header_dependency(&args[idx + 1]);
                } else {
                    eprintln!("Error: --c-header requires a path to a .h file");
                    std::process::exit(1);
                }
            } else {
                add_dependency(&args[2]);
            }
        }
        _ => print_usage(),
    }
}

fn print_usage() {
    println!("Orbit C* Package Manager");
    println!("Usage:");
    println!("  orbit init                          Initialize a new C* package");
    println!("  orbit add <dep>                     Add a dependency by name");
    println!("  orbit add --c-header <path.h>       Add C library via header binding");
    println!("  orbit build                         Build the current package");
}

fn init_project() {
    let toml_content = r#"[package]
name = "my_project"
version = "0.1.0"

[dependencies]
"#;

    let main_content = r#"fn main() -> i32 {
    print("Hello, World!\n")
    return 0
}
"#;

    if !Path::new("Orbit.toml").exists() {
        fs::write("Orbit.toml", toml_content).expect("failed to write Orbit.toml");
    }
    fs::create_dir_all("src").expect("failed to create src dir");
    if !Path::new("src/main.cx").exists() {
        fs::write("src/main.cx", main_content).expect("failed to write src/main.cx");
    }
    println!("Initialized C* package my_project");
}

fn build_project() {
    if !Path::new("Orbit.toml").exists() {
        eprintln!("Error: Orbit.toml not found in the current directory");
        std::process::exit(1);
    }

    resolve_dependencies();

    let main_cx = find_main_source();
    let out_exe = if cfg!(windows) { "main.exe" } else { "main" };

    // Incremental build check: Compare modification times
    let mut needs_rebuild = true;
    if let Ok(exe_meta) = fs::metadata(out_exe) {
        if let Ok(exe_mtime) = exe_meta.modified() {
            let mut max_src_mtime = None;
            
            if let Ok(meta) = fs::metadata("Orbit.toml") {
                if let Ok(mtime) = meta.modified() {
                    max_src_mtime = Some(mtime);
                }
            }

            if let Ok(meta) = fs::metadata("Orbit.lock") {
                if let Ok(mtime) = meta.modified() {
                    if max_src_mtime.map_or(true, |max| mtime > max) {
                        max_src_mtime = Some(mtime);
                    }
                }
            }

            if let Ok(entries) = fs::read_dir("src") {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_file() {
                            if let Ok(mtime) = meta.modified() {
                                if max_src_mtime.map_or(true, |max| mtime > max) {
                                    max_src_mtime = Some(mtime);
                                }
                            }
                        }
                    }
                }
            }

            if let Some(src_mtime) = max_src_mtime {
                if exe_mtime >= src_mtime {
                    needs_rebuild = false;
                }
            }
        }
    }

    if !needs_rebuild {
        println!("Orbit: project is up-to-date (no changes detected).");
        return;
    }

    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("-p")
        .arg("starc")
        .arg("--")
        .arg("build")
        .arg(&main_cx)
        .arg("-o")
        .arg(out_exe)
        .env(
            "PATH",
            format!(
                "C:\\PROGRA~1\\LLVM\\bin;{}",
                env::var("PATH").unwrap_or_default()
            ),
        );

    let status = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to execute cargo run: {}", e);
            std::process::exit(1);
        }
    };

    if status.success() {
        println!("Orbit build finished successfully.");
    } else {
        eprintln!("Orbit build failed.");
        std::process::exit(1);
    }
}

fn find_main_source() -> String {
    if Path::new("src/main.cx").exists() {
        return "src/main.cx".to_string();
    }
    if Path::new("main.cx").exists() {
        return "main.cx".to_string();
    }
    eprintln!("Error: No main.cx found in src/ or project root");
    std::process::exit(1);
}

fn resolve_dependencies() {
    let toml = match fs::read_to_string("Orbit.toml") {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some((name, _)) = trimmed.split_once('=') {
            let dep_name = name.trim();
            if dep_name == "name" || dep_name == "version" {
                continue;
            }

            let search_paths = vec![
                PathBuf::from("deps").join(format!("{}.cx", dep_name)),
                PathBuf::from("..").join(format!("{}.cx", dep_name)),
                PathBuf::from("examples").join(format!("{}.cx", dep_name)),
                PathBuf::from("stdlib").join(format!("{}.cx", dep_name)),
            ];

            for path in search_paths {
                if path.exists() {
                    let dest_dir = PathBuf::from("src");
                    let dest = dest_dir.join(format!("{}.cx", dep_name));
                    if !dest.exists() {
                        let _ = fs::create_dir_all(&dest_dir);
                        if let Err(e) = fs::copy(&path, &dest) {
                            eprintln!("Warning: failed to copy dependency {}: {}", dep_name, e);
                        } else {
                            println!("Resolved dependency `{}` from {}", dep_name, path.display());
                        }
                    }
                    break;
                }
            }
        }
    }
}

fn add_dependency(dep: &str) {
    if !Path::new("Orbit.toml").exists() {
        eprintln!("Error: Orbit.toml not found");
        std::process::exit(1);
    }
    let mut content = fs::read_to_string("Orbit.toml").unwrap();
    if !content.contains(dep) {
        content.push_str(&format!("\n{} = \"*\"\n", dep));
        fs::write("Orbit.toml", content).unwrap();
        println!("Added dependency `{}` to Orbit.toml", dep);

        update_lockfile(dep, "*");
    } else {
        println!("Dependency `{}` already exists in Orbit.toml", dep);
    }
}

fn add_c_header_dependency(header_path: &str) {
    if !Path::new(header_path).exists() {
        eprintln!("Error: header file not found: {}", header_path);
        std::process::exit(1);
    }

    let stem = Path::new(header_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("bindings");

    let output_cx = format!("src/{}.cx", stem);
    fs::create_dir_all("src").expect("failed to create src dir");

    let status = Command::new("cargo")
        .arg("run")
        .arg("-p")
        .arg("cstar-bind")
        .arg("--")
        .arg(header_path)
        .arg("-o")
        .arg(&output_cx)
        .status()
        .expect("failed to run cstar-bind");

    if !status.success() {
        eprintln!("Error: cstar-bind failed");
        std::process::exit(1);
    }

    if Path::new("Orbit.toml").exists() {
        let mut content = fs::read_to_string("Orbit.toml").unwrap();
        if !content.contains(stem) {
            content.push_str(&format!("\n{} = {{ path = \"{}\" }}\n", stem, output_cx));
            fs::write("Orbit.toml", content).unwrap();
        }
    }

    update_lockfile(stem, "c-header");
    println!(
        "Added C header dependency `{}` with bindings at {}",
        header_path, output_cx
    );
}

fn update_lockfile(name: &str, version: &str) {
    let lock_path = "Orbit.lock";
    let mut lock = if Path::new(lock_path).exists() {
        fs::read_to_string(lock_path).unwrap_or_default()
    } else {
        "# Orbit Lockfile\n".to_string()
    };

    if !lock.contains(&format!("name = \"{}\"", name)) {
        lock.push_str(&format!(
            "\n[[package]]\nname = \"{}\"\nversion = \"{}\"\n",
            name, version
        ));
        fs::write(lock_path, lock).unwrap();
    }
}

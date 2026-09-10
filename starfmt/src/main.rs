use std::env;
use std::fs;
use std::path::Path;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: starfmt [-w | --write] [--check] <file1> <file2> ...");
        process::exit(1);
    }

    let mut write_mode = false;
    let mut check_mode = false;
    let mut files = Vec::new();

    for arg in &args[1..] {
        match arg.as_str() {
            "-w" | "--write" => write_mode = true,
            "--check" => check_mode = true,
            _ => {
                if arg.starts_with("-") {
                    eprintln!("Unknown option: {}", arg);
                    process::exit(1);
                } else {
                    files.push(arg.clone());
                }
            }
        }
    }

    if files.is_empty() {
        eprintln!("Error: No files specified");
        process::exit(1);
    }

    let mut check_failed = false;

    for file in &files {
        let path = Path::new(file);
        if !path.exists() {
            eprintln!("Error: File not found: {}", file);
            process::exit(1);
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error reading file {}: {}", file, e);
                process::exit(1);
            }
        };

        let formatted = format_code(&content);

        if check_mode {
            if content != formatted {
                println!("Diff found in {}", file);
                check_failed = true;
            }
        } else if write_mode {
            if content != formatted {
                if let Err(e) = fs::write(path, &formatted) {
                    eprintln!("Error writing file {}: {}", file, e);
                    process::exit(1);
                }
                println!("Formatted {}", file);
            }
        } else {
            print!("{}", formatted);
        }
    }

    if check_failed {
        process::exit(1);
    }
}

fn format_code(content: &str) -> String {
    let mut formatted = String::new();
    let mut indent_level = 0;
    let mut in_block_comment = false;
    let mut consecutive_empty_lines = 0;

    let lines: Vec<&str> = content.lines().collect();

    for raw_line in lines {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            consecutive_empty_lines += 1;
            if consecutive_empty_lines <= 1 {
                formatted.push_str("\n");
            }
            continue;
        }
        consecutive_empty_lines = 0;

        let (processed_line, brace_diff, starts_with_closing) = analyze_and_format_line(trimmed, &mut in_block_comment);

        if starts_with_closing && indent_level > 0 {
            indent_level -= 1;
        }

        let indent = " ".repeat(indent_level * 4);
        formatted.push_str(&indent);
        formatted.push_str(&processed_line);
        formatted.push_str("\n");

        if !starts_with_closing {
            if brace_diff > 0 {
                indent_level += brace_diff as usize;
            } else if brace_diff < 0 {
                let diff_abs = brace_diff.abs() as usize;
                if indent_level >= diff_abs {
                    indent_level -= diff_abs;
                } else {
                    indent_level = 0;
                }
            }
        } else {
            // Already subtracted one level for starts_with_closing.
            // Apply rest of the diff.
            let rest_diff = brace_diff + 1; // since brace_diff was negative, rest_diff will be brace_diff + 1
            if rest_diff > 0 {
                indent_level += rest_diff as usize;
            } else if rest_diff < 0 {
                let diff_abs = rest_diff.abs() as usize;
                if indent_level >= diff_abs {
                    indent_level -= diff_abs;
                } else {
                    indent_level = 0;
                }
            }
        }
    }

    formatted
}

fn analyze_and_format_line(trimmed: &str, in_block_comment: &mut bool) -> (String, i32, bool) {
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut brace_diff = 0;
    let mut first_char = None;
    let mut char_indices = trimmed.char_indices().peekable();

    while let Some(&(i, c)) = char_indices.peek() {
        if *in_block_comment {
            if c == '*' && trimmed[i..].starts_with("*/") {
                *in_block_comment = false;
                char_indices.next(); // '*'
                char_indices.next(); // '/'
            } else {
                char_indices.next();
            }
            continue;
        }

        if in_line_comment {
            char_indices.next();
            continue;
        }

        if in_string {
            if c == '"' {
                in_string = false;
            }
            char_indices.next();
            continue;
        }

        // Detect comments
        if c == '/' && trimmed[i..].starts_with("//") {
            in_line_comment = true;
            char_indices.next();
            char_indices.next();
            continue;
        }

        if c == '/' && trimmed[i..].starts_with("/*") {
            *in_block_comment = true;
            char_indices.next();
            char_indices.next();
            continue;
        }

        if c == '"' {
            in_string = true;
            char_indices.next();
            continue;
        }

        if !c.is_whitespace() && first_char.is_none() {
            first_char = Some(c);
        }

        if c == '{' {
            brace_diff += 1;
        } else if c == '}' {
            brace_diff -= 1;
        }

        char_indices.next();
    }

    let starts_with_closing = first_char == Some('}');

    // Basic syntax beautification for standard spacing around operators outside strings/comments.
    let beautified = beautify_spacing(trimmed);

    (beautified, brace_diff, starts_with_closing)
}

fn beautify_spacing(line: &str) -> String {
    let mut result = String::new();
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Handle string literals
        if in_string {
            result.push(c);
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        // Handle block comments
        if in_block_comment {
            result.push(c);
            if c == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                result.push('/');
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        // Handle line comments
        if in_line_comment {
            result.push(c);
            i += 1;
            continue;
        }

        // Detect comment start
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            in_line_comment = true;
            result.push('/');
            result.push('/');
            i += 2;
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            in_block_comment = true;
            result.push('/');
            result.push('*');
            i += 2;
            continue;
        }

        // Detect string start
        if c == '"' {
            in_string = true;
            result.push('"');
            i += 1;
            continue;
        }

        // Standardize spacing around operators and delimiters
        // Fat arrow "=>"
        if c == '=' && i + 1 < chars.len() && chars[i + 1] == '>' {
            ensure_trailing_space(&mut result);
            result.push_str("=> ");
            i += 2;
            skip_whitespace(&chars, &mut i);
            continue;
        }
        // Equality "=="
        if c == '=' && i + 1 < chars.len() && chars[i + 1] == '=' {
            ensure_trailing_space(&mut result);
            result.push_str("== ");
            i += 2;
            skip_whitespace(&chars, &mut i);
            continue;
        }
        // Return arrow "->"
        if c == '-' && i + 1 < chars.len() && chars[i + 1] == '>' {
            ensure_trailing_space(&mut result);
            result.push_str("-> ");
            i += 2;
            skip_whitespace(&chars, &mut i);
            continue;
        }
        // Inequality "!="
        if c == '!' && i + 1 < chars.len() && chars[i + 1] == '=' {
            ensure_trailing_space(&mut result);
            result.push_str("!= ");
            i += 2;
            skip_whitespace(&chars, &mut i);
            continue;
        }
        // Less than or equal "<="
        if c == '<' && i + 1 < chars.len() && chars[i + 1] == '=' {
            ensure_trailing_space(&mut result);
            result.push_str("<= ");
            i += 2;
            skip_whitespace(&chars, &mut i);
            continue;
        }
        // Greater than or equal ">="
        if c == '>' && i + 1 < chars.len() && chars[i + 1] == '=' {
            ensure_trailing_space(&mut result);
            result.push_str(">= ");
            i += 2;
            skip_whitespace(&chars, &mut i);
            continue;
        }
        // Double colon "::"
        if c == ':' && i + 1 < chars.len() && chars[i + 1] == ':' {
            trim_trailing_whitespace(&mut result);
            result.push_str("::");
            i += 2;
            skip_whitespace(&chars, &mut i);
            continue;
        }
        // Single operators with spaces
        if (c == '=' || c == '+' || c == '<' || c == '>') 
            && (i == 0 || chars[i - 1] != '*') // raw pointer type raw *T
            && (i == 0 || chars[i - 1] != '&') // reference &T
        {
            ensure_trailing_space(&mut result);
            result.push(c);
            result.push(' ');
            i += 1;
            skip_whitespace(&chars, &mut i);
            continue;
        }

        // Minus sign (could be negative number literal or subtract)
        if c == '-' {
            // If it's a binary subtract, put spaces.
            // A simple heuristic: if preceded by alphanumeric or closing paren/brace, it's subtract.
            let is_subtract = if i > 0 {
                let prev = chars[i - 1];
                prev.is_alphanumeric() || prev == ')' || prev == '}' || prev == ']'
            } else {
                false
            };

            if is_subtract {
                ensure_trailing_space(&mut result);
                result.push_str("- ");
                i += 1;
                skip_whitespace(&chars, &mut i);
                continue;
            }
        }

        // Colon (type annotation, space after, none before)
        if c == ':' {
            trim_trailing_whitespace(&mut result);
            result.push_str(": ");
            i += 1;
            skip_whitespace(&chars, &mut i);
            continue;
        }

        // Comma (space after, none before)
        if c == ',' {
            trim_trailing_whitespace(&mut result);
            result.push_str(", ");
            i += 1;
            skip_whitespace(&chars, &mut i);
            continue;
        }

        // Braces spacing
        if c == '{' {
            ensure_trailing_space(&mut result);
            result.push('{');
            i += 1;
            continue;
        }

        result.push(c);
        i += 1;
    }

    result.trim_end().to_string()
}

fn ensure_trailing_space(s: &mut String) {
    if !s.is_empty() && !s.ends_with(' ') {
        s.push(' ');
    }
}

fn trim_trailing_whitespace(s: &mut String) {
    while s.ends_with(' ') {
        s.pop();
    }
}

fn skip_whitespace(chars: &[char], i: &mut usize) {
    while *i < chars.len() && chars[*i].is_whitespace() {
        *i += 1;
    }
}

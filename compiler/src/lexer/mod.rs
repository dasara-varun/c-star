use crate::diagnostics::Span;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum TokenKind {
    // Keywords
    Module,
    Use,
    As,
    Pub,
    Fn,
    Struct,
    Enum,
    Impl,
    Let,
    Mut,
    Return,
    Raw,
    If,
    Else,
    Match,
    For,
    In,
    While,
    Comptime,
    Where,

    // Literals
    Ident(String),
    Int(i64, Option<String>),   // value, suffix (e.g. "u32")
    Float(f64, Option<String>), // value, suffix (e.g. "f64")
    Str(String),
    Bool(bool),

    // Operators and Punctuation
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    EqEq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Amp,
    AmpAmp,
    PipePipe,
    Bang,
    Arrow,    // ->
    FatArrow, // =>
    TryOp,    // ?
    Comma,
    Colon,
    ColonColon,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Dot,

    // Virtual statement terminator
    Semicolon,
    Newline,

    // Special
    EOF,
}

impl TokenKind {
    pub fn can_terminate_statement(&self) -> bool {
        match self {
            TokenKind::Ident(_)
            | TokenKind::Int(_, _)
            | TokenKind::Float(_, _)
            | TokenKind::Str(_)
            | TokenKind::Bool(_)
            | TokenKind::RParen
            | TokenKind::RBrace
            | TokenKind::RBracket
            | TokenKind::Return => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[allow(dead_code)]
pub struct Lexer<'a> {
    source: &'a str,
    chars: Vec<(usize, char)>, // byte index and char
    cursor: usize,
    line: usize,
    col: usize,
    filename: String,
    
    // Newline-insertion state
    last_kind: Option<TokenKind>,
    paren_nesting: usize,
    bracket_nesting: usize,
    
    // Buffered tokens (e.g., virtual semicolons)
    token_buffer: Vec<Token>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str, filename: &str) -> Self {
        let chars = source.char_indices().collect();
        Self {
            source,
            chars,
            cursor: 0,
            line: 1,
            col: 1,
            filename: filename.to_string(),
            last_kind: None,
            paren_nesting: 0,
            bracket_nesting: 0,
            token_buffer: Vec::new(),
        }
    }

    pub fn line(&self) -> usize {
        self.line
    }

    pub fn col(&self) -> usize {
        self.col
    }

    fn peek(&self) -> Option<char> {
        if self.cursor < self.chars.len() {
            Some(self.chars[self.cursor].1)
        } else {
            None
        }
    }

    fn peek_next(&self) -> Option<char> {
        if self.cursor + 1 < self.chars.len() {
            Some(self.chars[self.cursor + 1].1)
        } else {
            None
        }
    }

    fn advance(&mut self) -> Option<char> {
        if self.cursor < self.chars.len() {
            let (_, c) = self.chars[self.cursor];
            self.cursor += 1;
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            Some(c)
        } else {
            None
        }
    }

    fn current_span(&self, start_line: usize, start_col: usize) -> Span {
        Span::new(start_line, start_col, self.line, self.col)
    }

    /// Read next raw token (skipping whitespace/comments, but yielding newlines)
    fn next_raw_token(&mut self) -> Result<Option<Token>, String> {
        loop {
            let start_line = self.line;
            let start_col = self.col;

            let c = match self.peek() {
                Some(c) => c,
                None => return Ok(None),
            };

            // Handle Newlines explicitly
            if c == '\n' {
                self.advance();
                let span = self.current_span(start_line, start_col);
                return Ok(Some(Token {
                    kind: TokenKind::Newline, // Represent raw newline internally
                    span,
                }));
            }

            // Handle carriage returns or simple whitespace
            if c.is_whitespace() {
                self.advance();
                continue;
            }

            // Handle comments
            if c == '/' {
                if self.peek_next() == Some('/') {
                    // Line comment
                    self.advance(); // /
                    self.advance(); // /
                    while let Some(nc) = self.peek() {
                        if nc == '\n' {
                            break;
                        }
                        self.advance();
                    }
                    continue;
                } else if self.peek_next() == Some('*') {
                    // Block comment (nestable)
                    self.advance(); // /
                    self.advance(); // *
                    let mut depth = 1;
                    while depth > 0 {
                        let nc = match self.advance() {
                            Some(nc) => nc,
                            None => return Err("Unterminated block comment".to_string()),
                        };
                        if nc == '/' && self.peek() == Some('*') {
                            self.advance();
                            depth += 1;
                        } else if nc == '*' && self.peek() == Some('/') {
                            self.advance();
                            depth -= 1;
                        }
                    }
                    continue;
                }
            }

            // Identifiers or Keywords
            if c.is_alphabetic() || c == '_' {
                let mut ident = String::new();
                while let Some(nc) = self.peek() {
                    if nc.is_alphanumeric() || nc == '_' {
                        ident.push(self.advance().unwrap());
                    } else {
                        break;
                    }
                }

                let kind = match ident.as_str() {
                    "module" => TokenKind::Module,
                    "use" => TokenKind::Use,
                    "as" => TokenKind::As,
                    "pub" => TokenKind::Pub,
                    "fn" => TokenKind::Fn,
                    "struct" => TokenKind::Struct,
                    "enum" => TokenKind::Enum,
                    "impl" => TokenKind::Impl,
                    "let" => TokenKind::Let,
                    "mut" => TokenKind::Mut,
                    "return" => TokenKind::Return,
                    "raw" => TokenKind::Raw,
                    "if" => TokenKind::If,
                    "else" => TokenKind::Else,
                    "match" => TokenKind::Match,
                    "for" => TokenKind::For,
                    "in" => TokenKind::In,
                    "while" => TokenKind::While,
                    "comptime" => TokenKind::Comptime,
                    "where" => TokenKind::Where,
                    "true" => TokenKind::Bool(true),
                    "false" => TokenKind::Bool(false),
                    _ => TokenKind::Ident(ident),
                };

                let span = self.current_span(start_line, start_col);
                return Ok(Some(Token { kind, span }));
            }

            // Numeric Literals
            if c.is_ascii_digit() {
                let mut num_str = String::new();
                let mut is_hex = false;
                let mut is_bin = false;

                // Check for prefixes like 0x or 0b
                if c == '0' {
                    if let Some(next) = self.peek_next() {
                        if next == 'x' || next == 'X' {
                            is_hex = true;
                            num_str.push(self.advance().unwrap()); // '0'
                            num_str.push(self.advance().unwrap()); // 'x'
                        } else if next == 'b' || next == 'B' {
                            is_bin = true;
                            num_str.push(self.advance().unwrap()); // '0'
                            num_str.push(self.advance().unwrap()); // 'b'
                        }
                    }
                }

                let mut is_float = false;
                while let Some(nc) = self.peek() {
                    if nc.is_ascii_hexdigit() || nc == '_' || nc == '.' {
                        if nc == '.' {
                            // Only a float if it is followed by a digit and is not hex/bin
                            if is_hex || is_bin {
                                break;
                            }
                            if let Some(next) = self.peek_next() {
                                if next.is_ascii_digit() {
                                    is_float = true;
                                    num_str.push(self.advance().unwrap()); // '.'
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        } else {
                            num_str.push(self.advance().unwrap());
                        }
                    } else {
                        break;
                    }
                }

                // Lex type suffix (e.g. u32, i64, f64)
                let mut suffix = None;
                if let Some(nc) = self.peek() {
                    if nc == 'u' || nc == 'i' || nc == 'f' {
                        let mut s = String::new();
                        while let Some(snc) = self.peek() {
                            if snc.is_alphanumeric() {
                                s.push(self.advance().unwrap());
                            } else {
                                break;
                            }
                        }
                        suffix = Some(s);
                    }
                }

                let span = self.current_span(start_line, start_col);
                let cleaned_str = num_str.replace('_', "");
                if is_float {
                    let val: f64 = cleaned_str.parse::<f64>().map_err(|e| e.to_string())?;
                    return Ok(Some(Token {
                        kind: TokenKind::Float(val, suffix),
                        span,
                    }));
                } else {
                    let val = if is_hex {
                        i64::from_str_radix(cleaned_str.trim_start_matches("0x").trim_start_matches("0X"), 16)
                            .map_err(|e| e.to_string())?
                    } else if is_bin {
                        i64::from_str_radix(cleaned_str.trim_start_matches("0b").trim_start_matches("0B"), 2)
                            .map_err(|e| e.to_string())?
                    } else {
                        cleaned_str.parse::<i64>().map_err(|e| e.to_string())?
                    };
                    return Ok(Some(Token {
                        kind: TokenKind::Int(val, suffix),
                        span,
                    }));
                }
            }

            // String Literals
            if c == '"' {
                self.advance(); // '"'
                let mut val = String::new();
                while let Some(nc) = self.peek() {
                    if nc == '"' {
                        self.advance();
                        let span = self.current_span(start_line, start_col);
                        return Ok(Some(Token {
                            kind: TokenKind::Str(val),
                            span,
                        }));
                    }
                    if nc == '\\' {
                        self.advance();
                        let escape = match self.advance() {
                            Some('n') => '\n',
                            Some('r') => '\r',
                            Some('t') => '\t',
                            Some('\\') => '\\',
                            Some('"') => '"',
                            Some('\'') => '\'',
                            Some(escaped) => return Err(format!("Invalid string escape: \\{}", escaped)),
                            None => return Err("Unterminated string literal".to_string()),
                        };
                        val.push(escape);
                    } else {
                        val.push(self.advance().unwrap());
                    }
                }
                return Err("Unterminated string literal".to_string());
            }

            // Operators & Punctuation
            let kind = match c {
                '+' => { self.advance(); TokenKind::Plus }
                '-' => {
                    self.advance();
                    if self.peek() == Some('>') {
                        self.advance();
                        TokenKind::Arrow
                    } else {
                        TokenKind::Minus
                    }
                }
                '*' => { self.advance(); TokenKind::Star }
                '/' => { self.advance(); TokenKind::Slash }
                '=' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        TokenKind::EqEq
                    } else if self.peek() == Some('>') {
                        self.advance();
                        TokenKind::FatArrow
                    } else {
                        TokenKind::Eq
                    }
                }
                '!' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        TokenKind::Ne
                    } else {
                        TokenKind::Bang
                    }
                }
                '<' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        TokenKind::Le
                    } else {
                        TokenKind::Lt
                    }
                }
                '>' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        TokenKind::Ge
                    } else {
                        TokenKind::Gt
                    }
                }
                '&' => {
                    self.advance();
                    if self.peek() == Some('&') {
                        self.advance();
                        TokenKind::AmpAmp
                    } else {
                        TokenKind::Amp
                    }
                }
                '|' => {
                    self.advance();
                    if self.peek() == Some('|') {
                        self.advance();
                        TokenKind::PipePipe
                    } else {
                        return Err("Single pipe '|' operator is not defined".to_string());
                    }
                }
                '?' => { self.advance(); TokenKind::TryOp }
                ',' => { self.advance(); TokenKind::Comma }
                ':' => {
                    self.advance();
                    if self.peek() == Some(':') {
                        self.advance();
                        TokenKind::ColonColon
                    } else {
                        TokenKind::Colon
                    }
                }
                '(' => { self.advance(); TokenKind::LParen }
                ')' => { self.advance(); TokenKind::RParen }
                '{' => { self.advance(); TokenKind::LBrace }
                '}' => { self.advance(); TokenKind::RBrace }
                '[' => { self.advance(); TokenKind::LBracket }
                ']' => { self.advance(); TokenKind::RBracket }
                '.' => { self.advance(); TokenKind::Dot }
                ';' => { self.advance(); TokenKind::Semicolon }
                _ => return Err(format!("Unexpected character: '{}'", c)),
            };

            let span = self.current_span(start_line, start_col);
            return Ok(Some(Token { kind, span }));
        }
    }

    /// Retrieve the next token, performing newline-to-semicolon insertion
    pub fn next_token(&mut self) -> Result<Token, String> {
        if !self.token_buffer.is_empty() {
            return Ok(self.token_buffer.remove(0));
        }

        loop {
            let raw = self.next_raw_token()?;
            match raw {
                None => {
                    // EOF reached.
                    // Insert a final semicolon if the last token was terminatable
                    let start_line = self.line;
                    let start_col = self.col;
                    let eof_span = Span::new(start_line, start_col, start_line, start_col);

                    if let Some(ref last) = self.last_kind {
                        if last.can_terminate_statement() && self.paren_nesting == 0 && self.bracket_nesting == 0 {
                            self.last_kind = None;
                            return Ok(Token {
                                kind: TokenKind::Semicolon,
                                span: eof_span,
                            });
                        }
                    }
                    return Ok(Token {
                        kind: TokenKind::EOF,
                        span: eof_span,
                    });
                }
                Some(mut tok) => {
                    if tok.kind == TokenKind::Newline {
                        // This was a raw newline!
                        // Check if we should insert a virtual semicolon
                        if let Some(ref last) = self.last_kind {
                            if last.can_terminate_statement() && self.paren_nesting == 0 && self.bracket_nesting == 0 {
                                self.last_kind = None;
                                tok.kind = TokenKind::Semicolon;
                                return Ok(tok); // Yield the virtual semicolon!
                            }
                        }
                        // Skip newline otherwise
                        continue;
                    }

                    if tok.kind == TokenKind::Semicolon {
                        self.last_kind = None;
                        return Ok(tok);
                    }

                    // Update nesting state
                    match tok.kind {
                        TokenKind::LParen => self.paren_nesting += 1,
                        TokenKind::RParen => self.paren_nesting = self.paren_nesting.saturating_sub(1),
                        TokenKind::LBracket => self.bracket_nesting += 1,
                        TokenKind::RBracket => self.bracket_nesting = self.bracket_nesting.saturating_sub(1),
                        _ => {}
                    }

                    self.last_kind = Some(tok.kind.clone());
                    return Ok(tok);
                }
            }
        }
    }

    /// Helper to lex all tokens up to EOF
    pub fn lex_all(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = tok.kind == TokenKind::EOF;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex_source(src: &str) -> Vec<TokenKind> {
        let mut lexer = Lexer::new(src, "test.cx");
        lexer.lex_all().unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn test_basic_tokens() {
        let src = "let mut x = 42 // comment\nlet y = 3.14u32";
        let kinds = lex_source(src);
        assert_eq!(
            kinds,
            vec![
                TokenKind::Let,
                TokenKind::Mut,
                TokenKind::Ident("x".to_string()),
                TokenKind::Eq,
                TokenKind::Int(42, None),
                TokenKind::Semicolon, // Virtual semicolon from newline
                TokenKind::Let,
                TokenKind::Ident("y".to_string()),
                TokenKind::Eq,
                TokenKind::Float(3.14, Some("u32".to_string())),
                TokenKind::Semicolon, // Virtual semicolon from EOF
                TokenKind::EOF,
            ]
        );
    }

    #[test]
    fn test_newline_insertion() {
        // Newline inserted when ending on terminatable tokens
        let src = "x\ny";
        let kinds = lex_source(src);
        assert_eq!(
            kinds,
            vec![
                TokenKind::Ident("x".to_string()),
                TokenKind::Semicolon,
                TokenKind::Ident("y".to_string()),
                TokenKind::Semicolon,
                TokenKind::EOF
            ]
        );

        // No newline inserted after operators
        let src = "x +\ny";
        let kinds = lex_source(src);
        assert_eq!(
            kinds,
            vec![
                TokenKind::Ident("x".to_string()),
                TokenKind::Plus,
                TokenKind::Ident("y".to_string()),
                TokenKind::Semicolon,
                TokenKind::EOF
            ]
        );

        // No newline inserted inside parentheses
        let src = "foo(\n  5\n)";
        let kinds = lex_source(src);
        assert_eq!(
            kinds,
            vec![
                TokenKind::Ident("foo".to_string()),
                TokenKind::LParen,
                TokenKind::Int(5, None),
                TokenKind::RParen,
                TokenKind::Semicolon,
                TokenKind::EOF
            ]
        );
    }
}


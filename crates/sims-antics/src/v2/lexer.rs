//! Tokenizer for the high-level `.antics` v2 authoring language.

use crate::error::{Error, Result, Span};

/// One token produced by the `.antics` v2 lexer.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Token kinds in the `.antics` v2 language.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    /// `#RRGGBB` 24-bit RGB color literal.
    Color([u8; 3]),
    /// Verbatim `.air` body scanned from `raw { ... }` using `.air` comment/string rules.
    RawBlock(String),
    // Delimiters
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    // Punctuation
    Colon,
    ColonColon,
    Semicolon,
    Comma,
    Dot,
    At,
    Arrow,    // ->
    FatArrow, // =>
    // Logical & comparison operators
    AndAnd, // &&
    OrOr,   // ||
    EqEq,   // ==
    BangEq, // !=
    LtEq,   // <=
    GtEq,   // >=
    Lt,     // <
    Gt,     // >
    Bang,   // !
    // Assignment & arithmetic operators
    Assign,    // = or :=
    PlusEq,    // +=
    MinusEq,   // -=
    StarEq,    // *=
    SlashEq,   // /=
    PercentEq, // %=
    AmpEq,     // &=
    PipeEq,    // |=
    CaretEq,   // ^=
    Plus,      // +
    Minus,     // -
    Star,      // *
    Slash,     // /
    Percent,   // %
    Amp,       // &
    Pipe,      // |
    Caret,     // ^
    Eof,
}

/// Tokenizes a `.antics` v2 source string into a `Vec<Token>` terminated by [`TokenKind::Eof`].
pub fn tokenize(source: &str) -> Result<Vec<Token>> {
    let mut lexer = Lexer {
        src: source,
        bytes: source.as_bytes(),
        pos: 0,
        line: 1,
        col: 1,
    };
    let mut tokens = Vec::new();
    loop {
        lexer.skip_ws_and_comments()?;
        let span = lexer.span();
        let Some(b) = lexer.peek() else {
            tokens.push(Token {
                kind: TokenKind::Eof,
                span,
            });
            return Ok(tokens);
        };
        let kind = match b {
            b'{' => {
                lexer.bump();
                TokenKind::LBrace
            }
            b'}' => {
                lexer.bump();
                TokenKind::RBrace
            }
            b'(' => {
                lexer.bump();
                TokenKind::LParen
            }
            b')' => {
                lexer.bump();
                TokenKind::RParen
            }
            b'[' => {
                lexer.bump();
                TokenKind::LBracket
            }
            b']' => {
                lexer.bump();
                TokenKind::RBracket
            }
            b';' => {
                lexer.bump();
                TokenKind::Semicolon
            }
            b',' => {
                lexer.bump();
                TokenKind::Comma
            }
            b'.' => {
                lexer.bump();
                TokenKind::Dot
            }
            b'@' => {
                lexer.bump();
                TokenKind::At
            }
            b':' => {
                lexer.bump();
                if lexer.peek() == Some(b':') {
                    lexer.bump();
                    TokenKind::ColonColon
                } else if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::Assign
                } else {
                    TokenKind::Colon
                }
            }
            b'=' => {
                lexer.bump();
                if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::EqEq
                } else if lexer.peek() == Some(b'>') {
                    lexer.bump();
                    TokenKind::FatArrow
                } else {
                    TokenKind::Assign
                }
            }
            b'!' => {
                lexer.bump();
                if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::BangEq
                } else {
                    TokenKind::Bang
                }
            }
            b'<' => {
                lexer.bump();
                if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            b'>' => {
                lexer.bump();
                if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            b'+' => {
                lexer.bump();
                if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::PlusEq
                } else {
                    TokenKind::Plus
                }
            }
            b'-' => {
                lexer.bump();
                if lexer.peek() == Some(b'>') {
                    lexer.bump();
                    TokenKind::Arrow
                } else if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::MinusEq
                } else {
                    TokenKind::Minus
                }
            }
            b'*' => {
                lexer.bump();
                if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::StarEq
                } else {
                    TokenKind::Star
                }
            }
            b'/' => {
                lexer.bump();
                if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::SlashEq
                } else {
                    TokenKind::Slash
                }
            }
            b'%' => {
                lexer.bump();
                if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::PercentEq
                } else {
                    TokenKind::Percent
                }
            }
            b'&' => {
                lexer.bump();
                if lexer.peek() == Some(b'&') {
                    lexer.bump();
                    TokenKind::AndAnd
                } else if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::AmpEq
                } else {
                    TokenKind::Amp
                }
            }
            b'|' => {
                lexer.bump();
                if lexer.peek() == Some(b'|') {
                    lexer.bump();
                    TokenKind::OrOr
                } else if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::PipeEq
                } else {
                    TokenKind::Pipe
                }
            }
            b'^' => {
                lexer.bump();
                if lexer.peek() == Some(b'=') {
                    lexer.bump();
                    TokenKind::CaretEq
                } else {
                    TokenKind::Caret
                }
            }
            b'"' => TokenKind::Str(lexer.lex_string(span)?),
            b'#' => TokenKind::Color(lexer.lex_color(span)?),
            b'0'..=b'9' => lexer.lex_number(span)?,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let ident = lexer.lex_ident();
                if ident == "raw" {
                    let saved_pos = lexer.pos;
                    let saved_line = lexer.line;
                    let saved_col = lexer.col;
                    lexer.skip_ws_and_comments()?;
                    if lexer.peek() == Some(b'{') {
                        let raw_body = lexer.lex_raw_air_block(span)?;
                        TokenKind::RawBlock(raw_body)
                    } else {
                        lexer.pos = saved_pos;
                        lexer.line = saved_line;
                        lexer.col = saved_col;
                        TokenKind::Ident(ident)
                    }
                } else {
                    TokenKind::Ident(ident)
                }
            }
            other => {
                return Err(Error::at(
                    span,
                    format!("unexpected character {:?}", char::from(other)),
                ));
            }
        };
        tokens.push(Token { kind, span });
    }
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    fn span(&self) -> Span {
        Span::new(self.line, self.col)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(b)
    }

    fn skip_ws_and_comments(&mut self) -> Result<()> {
        loop {
            while let Some(b) = self.peek() {
                if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                    self.bump();
                } else {
                    break;
                }
            }
            if self.peek() == Some(b'/') && self.peek_at(1) == Some(b'/') {
                while let Some(b) = self.bump() {
                    if b == b'\n' {
                        break;
                    }
                }
                continue;
            }
            if self.peek() == Some(b'/') && self.peek_at(1) == Some(b'*') {
                let start = self.span();
                self.bump();
                self.bump();
                let mut depth = 1usize;
                while depth > 0 {
                    match (self.peek(), self.peek_at(1)) {
                        (Some(b'/'), Some(b'*')) => {
                            self.bump();
                            self.bump();
                            depth += 1;
                        }
                        (Some(b'*'), Some(b'/')) => {
                            self.bump();
                            self.bump();
                            depth -= 1;
                        }
                        (Some(_), _) => {
                            self.bump();
                        }
                        (None, _) => {
                            return Err(Error::at(start, "unterminated block comment"));
                        }
                    }
                }
                continue;
            }
            return Ok(());
        }
    }

    fn lex_ident(&mut self) -> String {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.bump();
            } else {
                break;
            }
        }
        self.src[start..self.pos].to_owned()
    }

    fn lex_color(&mut self, span: Span) -> Result<[u8; 3]> {
        self.bump(); // '#'
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.bump();
            } else {
                break;
            }
        }
        let hex = &self.src[start..self.pos];
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::at(
                span,
                format!("invalid hex color '#{hex}' (expected 6 hex digits '#RRGGBB')"),
            ));
        }
        let r = u8::from_str_radix(&hex[0..2], 16)
            .map_err(|_| Error::at(span, format!("invalid red channel in '#{hex}'")))?;
        let g = u8::from_str_radix(&hex[2..4], 16)
            .map_err(|_| Error::at(span, format!("invalid green channel in '#{hex}'")))?;
        let b = u8::from_str_radix(&hex[4..6], 16)
            .map_err(|_| Error::at(span, format!("invalid blue channel in '#{hex}'")))?;
        Ok([r, g, b])
    }

    fn lex_number(&mut self, span: Span) -> Result<TokenKind> {
        let start = self.pos;
        if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x' | b'X')) {
            self.bump();
            self.bump();
            let hex_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_hexdigit() || b == b'_' {
                    self.bump();
                } else {
                    break;
                }
            }
            let raw: String = self.src[hex_start..self.pos]
                .chars()
                .filter(|&c| c != '_')
                .collect();
            if raw.is_empty() {
                return Err(Error::at(span, "expected hex digits after '0x'"));
            }
            let val = i64::from_str_radix(&raw, 16)
                .map_err(|_| Error::at(span, format!("invalid hex integer '0x{raw}'")))?;
            return Ok(TokenKind::Int(val));
        }

        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || b == b'_' {
                self.bump();
            } else {
                break;
            }
        }
        if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) {
            self.bump(); // '.'
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() || b == b'_' {
                    self.bump();
                } else {
                    break;
                }
            }
            let raw: String = self.src[start..self.pos]
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let val = raw
                .parse::<f64>()
                .map_err(|_| Error::at(span, format!("invalid float literal '{raw}'")))?;
            if !val.is_finite() {
                return Err(Error::at(span, "non-finite float literal"));
            }
            return Ok(TokenKind::Float(val));
        }

        let raw: String = self.src[start..self.pos]
            .chars()
            .filter(|&c| c != '_')
            .collect();
        let val = raw
            .parse::<i64>()
            .map_err(|_| Error::at(span, format!("invalid integer literal '{raw}'")))?;
        Ok(TokenKind::Int(val))
    }

    fn lex_string(&mut self, span: Span) -> Result<String> {
        self.bump(); // opening '"'
        let mut out = String::new();
        while let Some(b) = self.peek() {
            match b {
                b'"' => {
                    self.bump();
                    return Ok(out);
                }
                b'\\' => {
                    self.bump();
                    let Some(esc) = self.bump() else {
                        return Err(Error::at(span, "unterminated string escape"));
                    };
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'0' => out.push('\0'),
                        other => {
                            return Err(Error::at(
                                span,
                                format!("unsupported escape '\\{}'", char::from(other)),
                            ));
                        }
                    }
                }
                b'\n' => {
                    return Err(Error::at(span, "unterminated string literal"));
                }
                other if other < 0x80 => {
                    self.bump();
                    out.push(char::from(other));
                }
                _ => {
                    let start = self.pos;
                    let ch = self.src[start..]
                        .chars()
                        .next()
                        .ok_or_else(|| Error::at(span, "invalid UTF-8 in string literal"))?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                    self.col += 1;
                }
            }
        }
        Err(Error::at(span, "unterminated string literal"))
    }

    /// Scans the opaque `.air` body inside `raw { ... }` honoring `.air` comments
    /// (`//`, `#`, `/* ... */`), strings with escapes, and nested braces.
    fn lex_raw_air_block(&mut self, start_span: Span) -> Result<String> {
        self.bump(); // '{'
        let content_start = self.pos;
        let mut depth = 1usize;
        while let Some(b) = self.peek() {
            match b {
                b'"' => {
                    self.bump();
                    let mut escaped = false;
                    while let Some(sb) = self.bump() {
                        if escaped {
                            escaped = false;
                        } else if sb == b'\\' {
                            escaped = true;
                        } else if sb == b'"' {
                            break;
                        }
                    }
                }
                b'#' => {
                    while let Some(cb) = self.bump() {
                        if cb == b'\n' {
                            break;
                        }
                    }
                }
                b'/' if self.peek_at(1) == Some(b'/') => {
                    while let Some(cb) = self.bump() {
                        if cb == b'\n' {
                            break;
                        }
                    }
                }
                b'/' if self.peek_at(1) == Some(b'*') => {
                    let cspan = self.span();
                    self.bump();
                    self.bump();
                    let mut cdepth = 1usize;
                    while cdepth > 0 {
                        match (self.peek(), self.peek_at(1)) {
                            (Some(b'/'), Some(b'*')) => {
                                self.bump();
                                self.bump();
                                cdepth += 1;
                            }
                            (Some(b'*'), Some(b'/')) => {
                                self.bump();
                                self.bump();
                                cdepth -= 1;
                            }
                            (Some(_), _) => {
                                self.bump();
                            }
                            (None, _) => {
                                return Err(Error::at(
                                    cspan,
                                    "unterminated block comment inside 'raw { ... }'",
                                ));
                            }
                        }
                    }
                }
                b'{' => {
                    self.bump();
                    depth += 1;
                }
                b'}' => {
                    let content_end = self.pos;
                    self.bump();
                    depth -= 1;
                    if depth == 0 {
                        return Ok(self.src[content_start..content_end].to_owned());
                    }
                }
                _ => {
                    self.bump();
                }
            }
        }
        Err(Error::at(start_span, "unterminated 'raw { ... }' block"))
    }
}

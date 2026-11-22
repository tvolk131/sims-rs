//! Deterministic lexer for the `.antics` language with 1-based line/column spans.

use crate::error::{Error, Result, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Label(String),
    Int(i64),
    Str(String),
    // Punctuation & delimiters
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Semicolon,
    Dot,
    Arrow, // ->
    // Operators
    Assign,    // := or =
    PlusEq,    // +=
    MinusEq,   // -=
    StarEq,    // *=
    SlashEq,   // /=
    PercentEq, // %=
    AmpEq,     // &=
    PipeEq,    // |=
    CaretEq,   // ^=
    EqEq,      // ==
    BangEq,    // !=
    Lt,        // <
    LtEq,      // <=
    Gt,        // >
    GtEq,      // >=
    Bang,      // !
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub fn tokenize(source: &str) -> Result<Vec<Token>> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        let is_eof = token.kind == TokenKind::Eof;
        tokens.push(token);
        if is_eof {
            break;
        }
    }
    Ok(tokens)
}

struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    fn span(&self) -> Span {
        Span::new(self.line, self.column)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<u8> {
        self.bytes.get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let ch = self.peek()?;
        self.pos += 1;
        if ch == b'\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<()> {
        loop {
            while let Some(b) = self.peek() {
                if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                    self.bump();
                } else {
                    break;
                }
            }
            if self.peek() == Some(b'/') && self.peek_next() == Some(b'/') {
                while let Some(b) = self.bump() {
                    if b == b'\n' {
                        break;
                    }
                }
                continue;
            }
            if self.peek() == Some(b'#') {
                while let Some(b) = self.bump() {
                    if b == b'\n' {
                        break;
                    }
                }
                continue;
            }
            if self.peek() == Some(b'/') && self.peek_next() == Some(b'*') {
                let start = self.span();
                self.bump();
                self.bump();
                let mut closed = false;
                while let Some(b) = self.bump() {
                    if b == b'*' && self.peek() == Some(b'/') {
                        self.bump();
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return Err(Error::at(start, "unterminated block comment"));
                }
                continue;
            }
            break;
        }
        Ok(())
    }

    fn next_token(&mut self) -> Result<Token> {
        self.skip_whitespace_and_comments()?;
        let span = self.span();
        let Some(b) = self.peek() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                span,
            });
        };

        let kind = match b {
            b'{' => {
                self.bump();
                TokenKind::LBrace
            }
            b'}' => {
                self.bump();
                TokenKind::RBrace
            }
            b'(' => {
                self.bump();
                TokenKind::LParen
            }
            b')' => {
                self.bump();
                TokenKind::RParen
            }
            b'[' => {
                self.bump();
                TokenKind::LBracket
            }
            b']' => {
                self.bump();
                TokenKind::RBracket
            }
            b',' => {
                self.bump();
                TokenKind::Comma
            }
            b';' => {
                self.bump();
                TokenKind::Semicolon
            }
            b'.' => {
                self.bump();
                TokenKind::Dot
            }
            b':' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::Assign
                } else {
                    TokenKind::Colon
                }
            }
            b'-' => {
                if self.peek_next() == Some(b'>') {
                    self.bump();
                    self.bump();
                    TokenKind::Arrow
                } else if self.peek_next() == Some(b'=') {
                    self.bump();
                    self.bump();
                    TokenKind::MinusEq
                } else if self.peek_next().is_some_and(|n| n.is_ascii_digit()) {
                    self.lex_number(span)?
                } else {
                    return Err(Error::at(span, "unexpected '-'"));
                }
            }
            b'+' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::PlusEq
                } else {
                    return Err(Error::at(span, "expected '+='"));
                }
            }
            b'*' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::StarEq
                } else {
                    return Err(Error::at(span, "expected '*='"));
                }
            }
            b'/' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::SlashEq
                } else {
                    return Err(Error::at(span, "expected '/='"));
                }
            }
            b'%' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::PercentEq
                } else {
                    return Err(Error::at(span, "expected '%='"));
                }
            }
            b'&' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::AmpEq
                } else {
                    return Err(Error::at(span, "expected '&='"));
                }
            }
            b'|' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::PipeEq
                } else {
                    return Err(Error::at(span, "expected '|='"));
                }
            }
            b'^' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::CaretEq
                } else {
                    return Err(Error::at(span, "expected '^='"));
                }
            }
            b'=' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::EqEq
                } else {
                    TokenKind::Assign
                }
            }
            b'!' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::BangEq
                } else {
                    TokenKind::Bang
                }
            }
            b'<' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            b'>' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            b'"' => self.lex_string(span)?,
            b'@' => {
                self.bump();
                let name = self.lex_ident_str(span, "expected label name after '@'")?;
                TokenKind::Label(name)
            }
            b'0'..=b'9' => self.lex_number(span)?,
            b if b.is_ascii_alphabetic() || b == b'_' => {
                let name = self.lex_ident_str(span, "expected identifier")?;
                TokenKind::Ident(name)
            }
            other => {
                return Err(Error::at(
                    span,
                    format!("unexpected character {:?}", char::from(other)),
                ));
            }
        };

        Ok(Token { kind, span })
    }

    fn lex_ident_str(&mut self, span: Span, empty_msg: &str) -> Result<String> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.bump();
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(Error::at(span, empty_msg));
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| Error::at(span, "invalid UTF-8 in identifier"))?;
        Ok(text.to_owned())
    }

    fn lex_number(&mut self, span: Span) -> Result<TokenKind> {
        let start = self.pos;
        let negative = if self.peek() == Some(b'-') {
            self.bump();
            true
        } else {
            false
        };
        if self.peek() == Some(b'0') && matches!(self.peek_next(), Some(b'x' | b'X')) {
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
            let raw: String = self.bytes[hex_start..self.pos]
                .iter()
                .filter(|&&b| b != b'_')
                .map(|&b| char::from(b))
                .collect();
            if raw.is_empty() {
                return Err(Error::at(span, "empty hexadecimal integer literal"));
            }
            let val = i64::from_str_radix(&raw, 16)
                .map_err(|_| Error::at(span, format!("invalid hexadecimal literal '0x{raw}'")))?;
            return Ok(TokenKind::Int(if negative { -val } else { val }));
        }
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || b == b'_' {
                self.bump();
            } else {
                break;
            }
        }
        let raw: String = self.bytes[start..self.pos]
            .iter()
            .filter(|&&b| b != b'_')
            .map(|&b| char::from(b))
            .collect();
        let val = raw
            .parse::<i64>()
            .map_err(|_| Error::at(span, format!("invalid integer literal '{raw}'")))?;
        Ok(TokenKind::Int(val))
    }

    fn lex_string(&mut self, span: Span) -> Result<TokenKind> {
        self.bump(); // opening quote
        let mut out = Vec::new();
        while let Some(b) = self.bump() {
            match b {
                b'"' => {
                    let s = String::from_utf8(out)
                        .map_err(|_| Error::at(span, "string literal is not valid UTF-8"))?;
                    return Ok(TokenKind::Str(s));
                }
                b'\\' => {
                    let esc = self
                        .bump()
                        .ok_or_else(|| Error::at(span, "unterminated string escape"))?;
                    match esc {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'0' => out.push(0),
                        b'x' => {
                            let hi = self
                                .bump()
                                .ok_or_else(|| Error::at(span, "truncated \\x escape"))?;
                            let lo = self
                                .bump()
                                .ok_or_else(|| Error::at(span, "truncated \\x escape"))?;
                            let hex = [hi, lo];
                            let s = std::str::from_utf8(&hex)
                                .map_err(|_| Error::at(span, "invalid \\x escape"))?;
                            let val = u8::from_str_radix(s, 16)
                                .map_err(|_| Error::at(span, "invalid \\x hex escape"))?;
                            out.push(val);
                        }
                        other => {
                            return Err(Error::at(
                                span,
                                format!("unknown string escape '\\{}'", char::from(other)),
                            ));
                        }
                    }
                }
                b'\n' => {
                    return Err(Error::at(span, "unterminated string literal"));
                }
                other => out.push(other),
            }
        }
        Err(Error::at(span, "unterminated string literal"))
    }
}

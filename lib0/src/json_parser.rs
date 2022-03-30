/**
 * the MIT License
 *
 * Copyright (c) 2016-2022 rhysd
 * Copyright (c) 2022 Bartosz Sypytkowski
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
 * of the Software, and to permit persons to whom the Software is furnished to do so,
 * subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
 * INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
 * PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
 * LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
 * TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR
 * THE USE OR OTHER DEALINGS IN THE SOFTWARE.
 */
use std::char;
use std::collections::HashMap;
use std::fmt;
use std::iter::Peekable;

use crate::any::Any;

#[derive(Debug)]
pub struct JsonParseError {
    msg: String,
    line: usize,
    col: usize,
}

impl JsonParseError {
    fn new(msg: String, line: usize, col: usize) -> JsonParseError {
        JsonParseError { msg, line, col }
    }
}

impl fmt::Display for JsonParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Parse error at line:{}, col:{}: {}",
            self.line, self.col, &self.msg,
        )
    }
}

impl std::error::Error for JsonParseError {}

pub type JsonParseResult = Result<Any, JsonParseError>;

// Note: char::is_ascii_whitespace is not available because some characters are not defined as
// whitespace character in JSON spec. For example, U+000C FORM FEED is whitespace in Rust but
// it isn't in JSON.
fn is_whitespace(c: char) -> bool {
    match c {
        '\u{0020}' | '\u{000a}' | '\u{000d}' | '\u{0009}' => true,
        _ => false,
    }
}

pub struct JsonParser<I>
where
    I: Iterator<Item = char>,
{
    chars: Peekable<I>,
    line: usize,
    col: usize,
}

impl<I: Iterator<Item = char>> JsonParser<I> {
    pub fn new(it: I) -> Self {
        JsonParser {
            chars: it.peekable(),
            line: 1,
            col: 0,
        }
    }

    fn err<T>(&self, msg: String) -> Result<T, JsonParseError> {
        Err(JsonParseError::new(msg, self.line, self.col))
    }

    fn unexpected_eof(&self) -> Result<char, JsonParseError> {
        Err(JsonParseError::new(
            String::from("Unexpected EOF"),
            self.line,
            self.col,
        ))
    }

    fn next_pos(&mut self, c: char) {
        if c == '\n' {
            self.col = 0;
            self.line += 1;
        } else {
            self.col += 1;
        }
    }

    fn peek(&mut self) -> Result<char, JsonParseError> {
        while let Some(c) = self.chars.peek().copied() {
            if !is_whitespace(c) {
                return Ok(c);
            }
            self.next_pos(c);
            self.chars.next().unwrap();
        }
        self.unexpected_eof()
    }

    fn next(&mut self) -> Option<char> {
        while let Some(c) = self.chars.next() {
            self.next_pos(c);
            if !is_whitespace(c) {
                return Some(c);
            }
        }
        None
    }

    fn consume(&mut self) -> Result<char, JsonParseError> {
        if let Some(c) = self.next() {
            Ok(c)
        } else {
            self.unexpected_eof()
        }
    }

    fn consume_no_skip(&mut self) -> Result<char, JsonParseError> {
        if let Some(c) = self.chars.next() {
            self.next_pos(c);
            Ok(c)
        } else {
            self.unexpected_eof()
        }
    }

    fn parse_object(&mut self) -> JsonParseResult {
        if self.consume()? != '{' {
            return self.err(String::from("Object must starts with '{'"));
        }

        if self.peek()? == '}' {
            self.consume().unwrap();
            return Ok(Any::Map(Box::new(HashMap::new())));
        }

        let mut m = HashMap::new();
        loop {
            let key = match self.parse_any()? {
                Any::String(s) => s.into_string(),
                v => return self.err(format!("Key of object must be string but found {:?}", v)),
            };

            let c = self.consume()?;
            if c != ':' {
                return self.err(format!(
                    "':' is expected after key of object but actually found '{}'",
                    c
                ));
            }

            m.insert(key, self.parse_any()?);

            match self.consume()? {
                ',' => {}
                '}' => return Ok(Any::Map(Box::new(m))),
                c => {
                    return self.err(format!(
                        "',' or '}}' is expected for object but actually found '{}'",
                        c.escape_debug(),
                    ))
                }
            }
        }
    }

    fn parse_array(&mut self) -> JsonParseResult {
        if self.consume()? != '[' {
            return self.err(String::from("Array must starts with '['"));
        }

        if self.peek()? == ']' {
            self.consume().unwrap();
            return Ok(Any::Array(Box::new([])));
        }

        let mut v = vec![self.parse_any()?];
        loop {
            match self.consume()? {
                ',' => {}
                ']' => return Ok(Any::Array(v.into_boxed_slice())),
                c => {
                    return self.err(format!(
                        "',' or ']' is expected for array but actually found '{}'",
                        c
                    ))
                }
            }

            v.push(self.parse_any()?); // Next element
        }
    }

    fn push_utf16(&self, s: &mut String, utf16: &mut Vec<u16>) -> Result<(), JsonParseError> {
        if utf16.is_empty() {
            return Ok(());
        }

        match String::from_utf16(utf16) {
            Ok(utf8) => s.push_str(&utf8),
            Err(err) => return self.err(format!("Invalid UTF-16 sequence {:?}: {}", &utf16, err)),
        }
        utf16.clear();
        Ok(())
    }

    fn parse_string(&mut self) -> JsonParseResult {
        if self.consume()? != '"' {
            return self.err(String::from("String must starts with double quote"));
        }

        let mut utf16 = Vec::new(); // Buffer for parsing \uXXXX UTF-16 characters
        let mut s = String::new();
        loop {
            let c = match self.consume_no_skip()? {
                '\\' => match self.consume_no_skip()? {
                    '\\' => '\\',
                    '/' => '/',
                    '"' => '"',
                    'b' => '\u{0008}',
                    'f' => '\u{000c}',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'u' => {
                        let mut u = 0u16;
                        for _ in 0..4 {
                            let c = self.consume()?;
                            if let Some(h) = c.to_digit(16) {
                                u = u * 0x10 + h as u16;
                            } else {
                                return self.err(format!("Unicode character must be \\uXXXX (X is hex character) format but found character '{}'", c));
                            }
                        }
                        utf16.push(u);
                        // Additional \uXXXX character may follow. UTF-16 characters must be converted
                        // into UTF-8 string as sequence because surrogate pairs must be considered
                        // like "\uDBFF\uDFFF".
                        continue;
                    }
                    c => return self.err(format!("'\\{}' is invalid escaped character", c)),
                },
                '"' => {
                    self.push_utf16(&mut s, &mut utf16)?;
                    return Ok(Any::String(s.into_boxed_str()));
                }
                // Note: c.is_control() is not available here because JSON accepts 0x7f (DEL) in
                // string literals but 0x7f is control character.
                // Rough spec of JSON says string literal cannot contain control characters. But it
                // can actually contain 0x7f.
                c if (c as u32) < 0x20 => {
                    return self.err(format!(
                        "String cannot contain control character {}",
                        c.escape_debug(),
                    ));
                }
                c => c,
            };

            self.push_utf16(&mut s, &mut utf16)?;

            s.push(c);
        }
    }

    fn parse_constant(&mut self, s: &'static str) -> Option<JsonParseError> {
        for c in s.chars() {
            match self.consume_no_skip() {
                Ok(x) if x != c => {
                    return Some(JsonParseError::new(
                        format!("Unexpected character '{}' while parsing '{}'", c, s),
                        self.line,
                        self.col,
                    ));
                }
                Ok(_) => {}
                Err(e) => return Some(e),
            }
        }
        None
    }

    fn parse_null(&mut self) -> JsonParseResult {
        match self.parse_constant("null") {
            Some(err) => Err(err),
            None => Ok(Any::Null),
        }
    }

    fn parse_true(&mut self) -> JsonParseResult {
        match self.parse_constant("true") {
            Some(err) => Err(err),
            None => Ok(Any::Bool(true)),
        }
    }

    fn parse_false(&mut self) -> JsonParseResult {
        match self.parse_constant("false") {
            Some(err) => Err(err),
            None => Ok(Any::Bool(false)),
        }
    }

    fn parse_number(&mut self) -> JsonParseResult {
        let neg = if self.peek()? == '-' {
            self.consume_no_skip().unwrap();
            true
        } else {
            false
        };

        let mut s = String::new();
        let mut saw_dot = false;
        let mut saw_exp = false;

        while let Some(d) = self.chars.peek() {
            match d {
                '0'..='9' => s.push(*d),
                '.' => {
                    saw_dot = true;
                    break;
                }
                'e' | 'E' => {
                    saw_exp = true;
                    break;
                }
                _ => break,
            }
            self.consume_no_skip().unwrap();
        }

        if s.is_empty() {
            return self.err("Integer part must not be empty in number literal".to_string());
        }

        if s.starts_with('0') && s.len() > 1 {
            return self
                .err("Integer part of number must not start with 0 except for '0'".to_string());
        }

        if saw_dot {
            s.push(self.consume_no_skip().unwrap()); // eat '.'
            while let Some(d) = self.chars.peek() {
                match d {
                    '0'..='9' => s.push(*d),
                    'e' | 'E' => {
                        saw_exp = true;
                        break;
                    }
                    _ => break,
                }
                self.consume_no_skip().unwrap();
            }
            if s.ends_with('.') {
                return self.err("Fraction part of number must not be empty".to_string());
            }
        }

        if saw_exp {
            s.push(self.consume_no_skip().unwrap()); // eat 'e' or 'E'
            if let Some('+') | Some('-') = self.chars.peek() {
                s.push(self.consume_no_skip().unwrap());
            }

            let mut saw_digit = false;
            while let Some(d) = self.chars.peek() {
                match d {
                    '0'..='9' => s.push(*d),
                    _ => break,
                }
                saw_digit = true;
                self.consume_no_skip().unwrap();
            }

            if !saw_digit {
                return self.err("Exponent part must not be empty in number literal".to_string());
            }
        }

        match s.parse::<f64>() {
            Ok(n) => Ok(Any::Number(if neg { -n } else { n })),
            Err(err) => self.err(format!("Invalid number literal '{}': {}", s, err)),
        }
    }

    fn parse_any(&mut self) -> JsonParseResult {
        match self.peek()? {
            '0'..='9' | '-' => self.parse_number(),
            '"' => self.parse_string(),
            '[' => self.parse_array(),
            '{' => self.parse_object(),
            't' => self.parse_true(),
            'f' => self.parse_false(),
            'n' => self.parse_null(),
            c => self.err(format!("Invalid character: {}", c.escape_debug())),
        }
    }

    pub fn parse(&mut self) -> JsonParseResult {
        let v = self.parse_any()?;

        if let Some(c) = self.next() {
            return self.err(format!(
                "Expected EOF but got character '{}'",
                c.escape_debug(),
            ));
        }

        Ok(v)
    }
}

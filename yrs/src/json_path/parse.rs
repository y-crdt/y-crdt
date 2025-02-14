use super::{JsonPath, JsonPathToken, ParseError};
use std::fmt::Display;
use std::str::FromStr;

impl<'a> JsonPath<'a> {
    pub fn parse(path: &'a str) -> Result<Self, ParseError> {
        let mut tokens = Vec::new();
        let mut i = 0;
        let mut iter = path.chars().peekable();
        while let Some(c) = iter.next() {
            i += c.len_utf8();
            match c {
                '$' => tokens.push(JsonPathToken::Root),
                '@' => tokens.push(JsonPathToken::Current),
                '.' => {
                    let c = iter.peek();
                    match c {
                        Some('.') => {
                            tokens.push(JsonPathToken::Descend);
                            iter.next();
                            i += 1;
                        }
                        Some('*') => {
                            tokens.push(JsonPathToken::Wildcard);
                            iter.next();
                            i += 1;
                        }
                        Some(a) if a.is_alphabetic() => {
                            let start = i;
                            while let Some(a) = iter.peek() {
                                if a.is_alphanumeric() || a == &'_' {
                                    i += a.len_utf8();
                                    iter.next();
                                } else {
                                    break;
                                }
                            }
                            let end = i;
                            let member = &path[start..end];
                            tokens.push(JsonPathToken::Member(member));
                        }
                        Some(a) => {
                            return Err(invalid_char(*a, path));
                        }
                        None => {
                            return Err(ParseError::InvalidJsonPath(format!(
                                "Path cannot end with '.': '{}'",
                                path
                            )))
                        }
                    }
                }
                '[' => {
                    let start = i;
                    let mut slice = &path[i..];
                    let mut quote_start = false;
                    for c in iter.by_ref() {
                        i += c.len_utf8();
                        if c == ']' && !quote_start {
                            slice = &slice[..(i - start - 1)];
                            break;
                        }
                        if c == '\'' {
                            quote_start = !quote_start;
                        }
                    }
                    if slice == "*" {
                        tokens.push(JsonPathToken::Wildcard);
                    } else if let Ok(index) = slice.parse::<i32>() {
                        tokens.push(JsonPathToken::Index(index));
                    } else if slice.contains(':') {
                        let mut split = slice.split(':');
                        let start = split
                            .next()
                            .and_then(|s| u32::from_str(s).ok())
                            .unwrap_or(0);
                        let end = split
                            .next()
                            .and_then(|s| u32::from_str(s).ok())
                            .unwrap_or(u32::MAX);
                        let by = split
                            .next()
                            .and_then(|s| u32::from_str(s).ok())
                            .unwrap_or(1);
                        tokens.push(JsonPathToken::Slice(start, end, by));
                    } else if slice.starts_with('\'') && slice.ends_with('\'') {
                        let member = &slice[1..slice.len() - 1];
                        tokens.push(JsonPathToken::Member(member));
                    } else {
                        return Err(ParseError::InvalidJsonPath(format!(
                            "substring '{}' is not supported: '{}'",
                            slice, path
                        )));
                    }
                }
                '*' => tokens.push(JsonPathToken::Wildcard),
                c if c.is_alphabetic() => {
                    // handle cases like `..name`, `$name` or `@name`
                    let start = i - c.len_utf8();
                    while let Some(a) = iter.peek() {
                        if a.is_alphanumeric() || a == &'_' {
                            i += a.len_utf8();
                            iter.next();
                        } else {
                            break;
                        }
                    }
                    let end = i;
                    let member = &path[start..end];
                    tokens.push(JsonPathToken::Member(member));
                }
                _ => {
                    return Err(invalid_char(c, path));
                }
            }
        }
        Ok(JsonPath { tokens })
    }
}

fn invalid_char(c: char, path: &str) -> ParseError {
    ParseError::InvalidJsonPath(format!("Invalid character '{}' in path: '{}'", c, path))
}

#[cfg(test)]
mod test {
    use crate::json_path::parse::{JsonPath, JsonPathToken};
    use fastrand::u32;

    #[test]
    fn parse_root() {
        let path = JsonPath::parse("$").unwrap();
        assert_eq!(path.tokens, vec![JsonPathToken::Root]);
    }

    #[test]
    fn parse_current() {
        let path = JsonPath::parse("@").unwrap();
        assert_eq!(path.tokens, vec![JsonPathToken::Current]);
    }

    #[test]
    fn parse_wildcard() {
        let path = JsonPath::parse("$.*").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Wildcard]
        );
    }

    #[test]
    fn parse_wildcard_quoted() {
        let path = JsonPath::parse("$[*]").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Wildcard]
        );
    }

    #[test]
    fn parse_descend() {
        let path = JsonPath::parse("$..").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Descend]
        );
    }

    #[test]
    fn parse_descend_gradual() {
        let path = JsonPath::parse("$..name").unwrap();
        assert_eq!(
            path.tokens,
            vec![
                JsonPathToken::Root,
                JsonPathToken::Descend,
                JsonPathToken::Member("name")
            ]
        );
    }

    #[test]
    fn parse_dot_member() {
        let path = JsonPath::parse("$.key").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Member("key")]
        );
    }

    #[test]
    fn parse_quote_member() {
        let path = JsonPath::parse("$['key']").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Member("key")]
        );
    }

    #[test]
    fn parse_index() {
        let path = JsonPath::parse("$[0]").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Index(0)]
        );
    }

    #[test]
    fn parse_negative_index() {
        let path = JsonPath::parse("$[-3]").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Index(-3)]
        );
    }

    #[test]
    fn parse_slice_bounded() {
        let path = JsonPath::parse("$[1:3]").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Slice(1, 3, 1)]
        );
    }

    #[test]
    fn parse_slice_left_unbounded() {
        let path = JsonPath::parse("$[:3]").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Slice(0, 3, 1)]
        );
    }

    #[test]
    fn parse_slice_right_unbounded() {
        let path = JsonPath::parse("$[3:]").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Slice(3, u32::MAX, 1)]
        );
    }

    #[test]
    fn parse_slice_with_step() {
        let path = JsonPath::parse("$[3::2]").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Slice(3, u32::MAX, 2)]
        );
        let path = JsonPath::parse("$[3:5:2]").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Slice(3, 5, 2)]
        );
        let path = JsonPath::parse("$[:3:2]").unwrap();
        assert_eq!(
            path.tokens,
            vec![JsonPathToken::Root, JsonPathToken::Slice(0, 3, 2)]
        );
    }

    #[test]
    fn parse_complex() {
        let path = JsonPath::parse("$.key_1[0].key2[*]").unwrap();
        assert_eq!(
            path.tokens,
            vec![
                JsonPathToken::Root,
                JsonPathToken::Member("key_1"),
                JsonPathToken::Index(0),
                JsonPathToken::Member("key2"),
                JsonPathToken::Wildcard
            ]
        );
    }
}

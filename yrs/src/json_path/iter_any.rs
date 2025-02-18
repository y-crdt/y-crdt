use super::{JsonPath, JsonPathToken};
use crate::Any;
use proptest::num::usize;

impl Any {
    pub fn json_path<'a>(&'a self, path: &'a JsonPath<'a>) -> JsonPathIter<'a> {
        let mut acc = Vec::new();
        let root = self;
        let current = root;
        Self::json_path_internal(root, current, path.as_ref(), &mut acc, false);
        Box::new(acc.into_iter())
    }

    fn json_path_internal<'a>(
        root: &'a Self,
        mut current: &'a Self,
        pattern: &'a [JsonPathToken<'a>],
        acc: &mut Vec<&'a Any>,
        is_descending: bool,
    ) {
        let mut early_return = false;
        for i in 0..pattern.len() {
            let segment = &pattern[i];
            match segment {
                JsonPathToken::Root => current = root,
                JsonPathToken::Current => { /* do nothing */ }
                JsonPathToken::Member(key) => {
                    if let Any::Map(map) = current {
                        current = match map.get(*key) {
                            Some(value) => value,
                            None => {
                                early_return = true;
                                break;
                            }
                        }
                    } else {
                        early_return = true;
                    }
                }
                JsonPathToken::Index(idx) => {
                    if let Any::Array(array) = current {
                        let idx = if *idx < 0 {
                            array.len() as i32 + idx
                        } else {
                            *idx
                        } as usize;
                        current = match array.get(idx) {
                            Some(value) => value,
                            None => {
                                early_return = true;
                                break;
                            }
                        };
                    } else {
                        early_return = true;
                    }
                }
                JsonPathToken::Wildcard => {
                    if let Some(iter) = any_iter(current) {
                        let pattern = &pattern[i + 1..];
                        for any in iter {
                            Self::json_path_internal(root, any, pattern, acc, false);
                        }
                    } else {
                        early_return = true;
                    }
                }
                JsonPathToken::RecursiveDescend => {
                    if let Some(iter) = any_iter(current) {
                        let pattern = &pattern[i + 1..];
                        for any in iter {
                            Self::json_path_internal(root, any, pattern, acc, true);
                        }
                    } else {
                        early_return = true;
                    }
                }
                JsonPathToken::Slice(from, to, by) => {
                    if let Some(iter) =
                        slice_iter(current, *from as usize, *to as usize, *by as usize)
                    {
                        let pattern = &pattern[i + 1..];
                        for any in iter {
                            Self::json_path_internal(root, any, pattern, acc, false);
                        }
                    } else {
                        early_return = true;
                    }
                }
            }

            if early_return {
                break;
            }
        }

        if !early_return {
            acc.push(current);
        } else if is_descending {
            match current {
                Any::Array(array) => {
                    for any in array.iter() {
                        Self::json_path_internal(root, any, pattern, acc, true);
                    }
                }
                Any::Map(map) => {
                    for any in map.values() {
                        Self::json_path_internal(root, any, pattern, acc, true);
                    }
                }
                _ => { /* do nothing */ }
            }
        }
    }
}

fn slice_iter<'a>(
    any: &'a Any,
    from: usize,
    to: usize,
    by: usize,
) -> Option<Box<dyn Iterator<Item = &'a Any> + 'a>> {
    match any {
        Any::Array(array) => {
            let to = array.len().min(to);
            Some(Box::new(
                array.iter().skip(from).take(to - from).step_by(by),
            ))
        }
        _ => None,
    }
}

fn any_iter<'a>(any: &'a Any) -> Option<Box<dyn Iterator<Item = &'a Any> + 'a>> {
    match any {
        Any::Array(array) => Some(Box::new(array.iter())),
        Any::Map(map) => Some(Box::new(map.values())),
        _ => None,
    }
}

pub type JsonPathIter<'a> = Box<dyn Iterator<Item = &'a Any> + 'a>;

/// Scope used for recursive iteration, i.e. wildcard, descent or slice.
struct IterScope<'a> {
    /// Scopes can be nested in each other i.e. `$.people.*.friends[*]name`. In such case they
    /// are organized in a linked list, with the first elements being the innermost scopes.
    next: Option<Box<IterScope<'a>>>,
    /// Offset to tokens array, where the current scope starts.
    index: usize,
    /// Current object this scope is iterating over.
    current: &'a Any,
    /// Iterator used by this scope.
    state: ScopeIterator<'a>,
}

enum ScopeIterator<'a> {
    /// Iterator used by wildcard or slice.
    Iter(Box<dyn Iterator<Item = &'a Any> + 'a>),
}

#[cfg(test)]
mod test {
    use crate::json_path::JsonPath;
    use crate::{any, Any};
    use std::path::Display;

    fn mixed_sample() -> Any {
        any!({
            "users": [
                {
                    "name": "Alice",
                    "surname": "Smith",
                    "age": 25,
                    "friends": [
                        { "name": "Bob", "nick": "boreas" },
                        { "nick": "crocodile91" }
                    ]
                },
                {
                    "name": "Bob",
                    "nick": "boreas",
                    "age": 30
                },
                {
                    "nick": "crocodile91",
                    "age": 35
                },
                {
                    "name": "Damian",
                    "surname": "Smith",
                    "age": 30
                },
                {
                    "name": "Elise",
                    "age": 35
                }
            ]
        })
    }

    #[test]
    fn eval_member_partial() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.users").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        let expected = any!([
            {
                "name": "Alice",
                "surname": "Smith",
                "age": 25,
                "friends": [
                    { "name": "Bob", "nick": "boreas" },
                    { "nick": "crocodile91" }
                ]
            },
            {
                "name": "Bob",
                "nick": "boreas",
                "age": 30
            },
            {
                "nick": "crocodile91",
                "age": 35
            },
            {
                "name": "Damian",
                "surname": "Smith",
                "age": 30
            },
            {
                "name": "Elise",
                "age": 35
            }
        ]);
        assert_eq!(values, vec![&expected]);
    }

    #[test]
    fn eval_member_full() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.users[0].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Alice")]);
    }

    #[test]
    fn eval_member_negative_index() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.users[-1].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Elise")]);
    }

    #[test]
    fn eval_member_wildcard_array() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.users[*].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(
            values,
            vec![
                &any!("Alice"),
                &any!("Bob"),
                &any!("Damian"),
                &any!("Elise")
            ]
        );
    }

    #[test]
    fn eval_member_slice() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.users[1:3].nick").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("boreas"), &any!("crocodile91")]);
    }

    #[test]
    fn eval_descent_flat() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.users..name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(
            values,
            vec![
                &any!("Alice"),
                &any!("Bob"),
                &any!("Damian"),
                &any!("Elise")
            ]
        );
    }

    #[test]
    fn eval_descent_multi_level() {
        let any = any!({
            "a": {
                "b1": {
                    "c": {
                        "f": {
                            "name": "Alice"
                        }
                    }
                },
                "b2": {
                    "d": {
                        "c": {
                            "g": {
                                "h": {
                                    "name": "Bob"
                                }
                            }
                        }
                    }
                }
            }
        });
        let path = JsonPath::parse("$..c..name").unwrap();
        let mut values: Vec<_> = any.json_path(&path).map(|any| any.to_string()).collect();
        values.sort();
        assert_eq!(values, vec!["Alice".to_string(), "Bob".into()]);
    }
}

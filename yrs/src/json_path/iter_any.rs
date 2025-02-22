use super::{JsonPath, JsonPathToken};
use crate::Any;
use proptest::num::usize;

impl Any {
    pub fn json_path<'a>(&'a self, path: &'a JsonPath<'a>) -> JsonPathIter<'a> {
        JsonPathIter::new(self, path.as_ref())
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

pub struct JsonPathIter<'a> {
    root: &'a Any,
    pattern: &'a [JsonPathToken<'a>],
    frame: ExecutionFrame<'a>,
}

impl<'a> JsonPathIter<'a> {
    fn new(root: &'a Any, path: &'a [JsonPathToken<'a>]) -> Self {
        Self {
            root,
            pattern: path.as_ref(),
            frame: ExecutionFrame::new(root, 0, None),
        }
    }
}

impl<'a> Iterator for JsonPathIter<'a> {
    type Item = &'a Any;

    fn next(&mut self) -> Option<Self::Item> {
        let frame = &mut self.frame;
        if let Some(iter) = &mut frame.iter {
            match iter.next() {
                None => {
                    frame.iter = None;
                    return if frame.ascend() { self.next() } else { None };
                }
                Some(curr) => {
                    frame.descend(curr);
                }
            }
        }
        // early return only works if the loop after has at least one iteration
        let mut early_return = frame.index == self.pattern.len();
        while frame.index < self.pattern.len() {
            let segment = &self.pattern[frame.index];
            frame.index += 1;
            match segment {
                JsonPathToken::Root => frame.current = self.root,
                JsonPathToken::Current => { /* do nothing */ }
                JsonPathToken::Member(key) => {
                    if let Any::Map(map) = frame.current {
                        frame.current = match map.get(*key) {
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
                    if let Any::Array(array) = frame.current {
                        let idx = if *idx < 0 {
                            array.len() as i32 + idx
                        } else {
                            *idx
                        } as usize;
                        frame.current = match array.get(idx) {
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
                    if let Some(iter) = any_iter(frame.current) {
                        frame.iter = Some(iter);
                        return self.next();
                    }
                    early_return = true;
                }
                JsonPathToken::RecursiveDescend => {
                    if let Some(iter) = any_iter(frame.current) {
                        frame.iter = Some(iter);
                        frame.is_descending = true;
                        return self.next();
                    }
                    early_return = true;
                }
                JsonPathToken::Slice(from, to, by) => {
                    if let Some(iter) =
                        slice_iter(frame.current, *from as usize, *to as usize, *by as usize)
                    {
                        frame.iter = Some(iter);
                        return self.next();
                    }
                    early_return = true;
                }
            }

            if early_return {
                break;
            }
        }

        if !early_return {
            return Some(frame.current);
        } else if frame.is_descending {
            if let Some(iter) = any_iter(frame.current) {
                frame.iter = Some(iter);
                frame.is_descending = true;
                frame.index -= 1; // '..' means we're not consuming the segment in this iteration
                return self.next();
            }
        }

        if frame.iter.is_none() && !frame.ascend() {
            None // we got to the end of the evaluator
        } else {
            self.next()
        }
    }
}

/// Scope used for recursive iteration, i.e. wildcard, descent or slice.
struct ExecutionFrame<'a> {
    /// Offset to tokens array, where the current scope starts.
    index: usize,
    /// Whether we're in recursive descent scope.
    is_descending: bool,
    /// Current object this scope is iterating over.
    current: &'a Any,
    /// Iterator used by this scope.
    iter: Option<ScopeIterator<'a>>,
    /// Scopes can be nested in each other i.e. `$.people.*.friends[*]name`. In such case they
    /// are organized in a linked list, with the first elements being the innermost scopes.
    next: Option<Box<ExecutionFrame<'a>>>,
}

impl<'a> ExecutionFrame<'a> {
    fn new(current: &'a Any, index: usize, iter: Option<ScopeIterator<'a>>) -> Self {
        Self {
            index,
            is_descending: false,
            current,
            iter,
            next: None,
        }
    }

    /// Descent into given iterator context, moving current frame to the stack and replacing it with
    /// a new one executing in a context of that iterator.
    fn descend(&mut self, current: &'a Any) {
        let new_self = ExecutionFrame {
            index: self.index,
            is_descending: self.is_descending,
            current,
            iter: None,
            next: None,
        };
        let old_frame = std::mem::replace(self, new_self);
        self.next = Some(Box::new(old_frame));
    }

    /// Return from current scope, restoring previous iter frame from the stack.
    /// If there are no more iter frames, return false.
    fn ascend(&mut self) -> bool {
        match self.next.take() {
            None => false,
            Some(next) => {
                *self = *next;
                true
            }
        }
    }
}

type ScopeIterator<'a> = Box<dyn Iterator<Item = &'a Any> + 'a>;

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

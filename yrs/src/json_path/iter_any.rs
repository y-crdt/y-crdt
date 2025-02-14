use super::{JsonPath, JsonPathToken};
use crate::Any;
use proptest::num::usize;

impl Any {
    pub fn json_path<'a>(&'a self, path: &'a JsonPath<'a>) -> JsonPathIter<'a> {
        JsonPathIter::new(&path.tokens, self)
    }
}

pub struct JsonPathIter<'a> {
    /// Root object to start from.
    root: &'a Any,
    /// Current object we're at.
    current: &'a Any,
    /// JsonPath tokens to evaluate.
    tokens: &'a [JsonPathToken<'a>],
    /// Offset to tokens array, pointing to currently evaluated token.
    index: usize,
    /// Context used for recursive iterating, ie. recursive descent, wildcard or slices.
    context: Option<Box<IterScope<'a>>>,
}

impl<'a> JsonPathIter<'a> {
    fn new(tokens: &'a [JsonPathToken<'a>], root: &'a Any) -> Self {
        Self {
            root,
            current: root,
            tokens,
            index: 0,
            context: None,
        }
    }

    /// If current iterator works in the context of multi-value retrieval like wildcard or slice,
    /// once a value was retrieved in current branch, we need to iterate to the next value from
    /// the last place where wildcard or slice was located.
    fn advance(&mut self) -> bool {
        if let Some(item) = &mut self.context {
            self.index = item.index;
            let finished = match &mut item.state {
                ScopeIterator::Iter(iter) => {
                    if let Some(next) = iter.next() {
                        self.current = next;
                        false
                    } else {
                        true
                    }
                }
            };
            if finished {
                // end of iterator, rollback to the previous context
                self.current = item.current;
                self.context = item.next.take();
                self.index = usize::MAX; // force rollback in the next iteration
            }
            true
        } else {
            false
        }
    }

    fn foreach(&mut self) -> Option<Box<dyn Iterator<Item = &'a Any> + 'a>> {
        let iter: Box<dyn Iterator<Item = &'a Any> + 'a> = match self.current {
            Any::Array(array) => Box::new(array.iter()),
            Any::Map(map) => Box::new(map.values()),
            _ => return None,
        };
        Some(iter)
    }
}

impl<'a> Iterator for JsonPathIter<'a> {
    type Item = &'a Any;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index > self.tokens.len() {
            if self.advance() {
                return self.next();
            }
            None // reached the end
        } else if self.index == self.tokens.len() {
            self.index += 1;
            Some(self.current)
        } else {
            match &self.tokens[self.index] {
                JsonPathToken::Root => self.current = self.root,
                JsonPathToken::Current => { /* do nothing */ }
                JsonPathToken::Member(key) => {
                    if let Any::Map(map) = self.current {
                        println!("pick key: {} from {}", key, self.current);
                        self.current = match map.get(*key) {
                            Some(child) => child,
                            None => {
                                //TODO: if we didn't match the key and we're descending, just allocate next
                                // scope, self.index-- and continue
                                todo!()
                            }
                        };
                    }
                }
                JsonPathToken::Index(index) => {
                    if let Any::Array(array) = self.current {
                        let mut index = *index;
                        if index < 0 {
                            index = array.len() as i32 + index;
                        }
                        println!("pick index: {} from {}", index, self.current);
                        self.current = array.get(index as usize)?;
                        //TODO: if we didn't match the key and we're descending, just allocate next
                        // scope, self.index-- and continue
                    }
                }
                JsonPathToken::Wildcard => {
                    if let Some(mut iter) = self.foreach() {
                        if let Some(next) = iter.next() {
                            let context = IterScope {
                                next: self.context.take(),
                                index: self.index,
                                current: self.current,
                                state: ScopeIterator::Iter(iter),
                            };
                            self.current = next;
                            self.context = Some(Box::new(context));
                        }
                    }
                }
                JsonPathToken::Slice(from, to, by) => {
                    if let Some(mut iter) = self.foreach() {
                        iter = Box::new(
                            iter.skip(*from as usize)
                                .take((*to - *from) as usize)
                                .step_by(*by as usize),
                        );
                        if let Some(next) = iter.next() {
                            let context = IterScope {
                                next: self.context.take(),
                                index: self.index,
                                current: self.current,
                                state: ScopeIterator::Iter(iter),
                            };
                            self.current = next;
                            self.context = Some(Box::new(context));
                        }
                    }
                }
                JsonPathToken::Descend => {
                    todo!("recursive descent");
                }
            }
            self.index += 1;
            return self.next();
        }
    }
}

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

    fn mixed_sample() -> Any {
        any!({
          "friends": [
            { "name": "Alice" },
            { "nick": "boreas" },
            { "nick": "crocodile91" },
            { "name": "Damian" },
            { "name": "Elise" },
          ]
        })
    }

    #[test]
    fn eval_member_partial() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.friends").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(
            values,
            vec![&any!([
              { "name": "Alice" },
              { "nick": "boreas" },
              { "nick": "crocodile91" },
              { "name": "Damian" },
              { "name": "Elise" },
            ])]
        );
    }

    #[test]
    fn eval_member_full() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.friends[0].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Alice")]);
    }

    #[test]
    fn eval_member_negative_index() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.friends[-1].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Elise")]);
    }

    #[test]
    fn eval_member_wildcard_array() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.friends[*].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(
            values,
            vec![
                &any!("Alice"),
                &Any::Undefined,
                &Any::Undefined,
                &any!("Damian"),
                &any!("Elise")
            ]
        );
    }

    #[test]
    fn eval_member_slice() {
        let any = mixed_sample();
        let path = JsonPath::parse("$.friends[1:3].nick").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("boreas"), &any!("crocodile91")]);
    }
}

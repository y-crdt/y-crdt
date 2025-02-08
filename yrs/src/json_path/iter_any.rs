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
    context: Option<Box<IterItem<'a>>>,
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
                IterState::ArrayIter(array) => {
                    if let Some(next) = array.next() {
                        self.current = next;
                        false
                    } else {
                        true
                    }
                }
                IterState::SliceIter(slice) => {
                    if let Some(next) = slice.next() {
                        self.current = next;
                        false
                    } else {
                        true
                    }
                }
                IterState::MapIter(map) => {
                    if let Some((_, next)) = map.next() {
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

    fn foreach(&mut self, slice: Option<(u32, u32, u32)>) {
        match self.current {
            Any::Array(array) => {
                let mut iter = array.iter();
                if let Some((from, to, by)) = slice {
                    let mut iter = iter
                        .skip(from as usize)
                        .take((to - from) as usize)
                        .step_by(by as usize);
                    if let Some(next) = iter.next() {
                        let context = IterItem {
                            next: self.context.take(),
                            index: self.index,
                            current: self.current,
                            state: IterState::SliceIter(iter),
                        };
                        self.current = next;
                        self.context = Some(Box::new(context));
                    }
                } else {
                    if let Some(next) = iter.next() {
                        let context = IterItem {
                            next: self.context.take(),
                            index: self.index,
                            current: self.current,
                            state: IterState::ArrayIter(iter),
                        };
                        self.current = next;
                        self.context = Some(Box::new(context));
                    }
                }
            }
            Any::Map(map) => {
                let mut iter = map.iter();
                if let Some((_, next)) = iter.next() {
                    let context = IterItem {
                        next: self.context.take(),
                        index: self.index,
                        current: self.current,
                        state: IterState::MapIter(iter),
                    };
                    self.current = next;
                    self.context = Some(Box::new(context));
                }
            }
            _ => { /* do nothing */ }
        }
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
                        self.current = map.get(*key)?;
                    }
                }
                JsonPathToken::Index(index) => {
                    if let Any::Array(array) = self.current {
                        let mut index = *index;
                        if index < 0 {
                            index = array.len() as i32 + index;
                        }
                        self.current = array.get(index as usize)?;
                    }
                }
                JsonPathToken::Wildcard => {
                    self.foreach(None);
                }
                JsonPathToken::Descend => {
                    todo!("recursive descent");
                }
                JsonPathToken::Slice(from, to, by) => {
                    self.foreach(Some((*from, *to, *by)));
                }
            }
            self.index += 1;
            return self.next();
        }
    }
}

struct IterItem<'a> {
    next: Option<Box<IterItem<'a>>>,
    index: usize,
    current: &'a Any,
    state: IterState<'a>,
}

enum IterState<'a> {
    ArrayIter(std::slice::Iter<'a, Any>),
    MapIter(std::collections::hash_map::Iter<'a, String, Any>),
    SliceIter(std::iter::StepBy<std::iter::Take<std::iter::Skip<std::slice::Iter<'a, Any>>>>),
}

#[cfg(test)]
mod test {
    use crate::json_path::JsonPath;
    use crate::{any, Any};

    fn sample() -> Any {
        any!({
          "friends": [
            { "name": "Alice" },
            { "name": "Bob" },
            { "name": "Carl" },
            { "name": "Damian" },
            { "name": "Elise" },
          ]
        })
    }

    #[test]
    fn eval_member_partial() {
        let any = sample();
        let path = JsonPath::parse("$.friends").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(
            values,
            vec![&any!([
              { "name": "Alice" },
              { "name": "Bob" },
              { "name": "Carl" },
              { "name": "Damian" },
              { "name": "Elise" },
            ])]
        );
    }

    #[test]
    fn eval_member_full() {
        let any = sample();
        let path = JsonPath::parse("$.friends[0].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Alice")]);
    }

    #[test]
    fn eval_member_negative_index() {
        let any = sample();
        let path = JsonPath::parse("$.friends[-1].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Elise")]);
    }

    #[test]
    fn eval_member_wildcard_array() {
        let any = sample();
        let path = JsonPath::parse("$.friends[*].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(
            values,
            vec![
                &any!("Alice"),
                &any!("Bob"),
                &any!("Carl"),
                &any!("Damian"),
                &any!("Elise")
            ]
        );
    }

    #[test]
    fn eval_member_slice() {
        let any = sample();
        let path = JsonPath::parse("$.friends[1:3].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Bob"), &any!("Carl")]);
    }
}

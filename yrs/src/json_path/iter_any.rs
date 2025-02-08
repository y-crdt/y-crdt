use super::{JsonPath, JsonPathToken};
use crate::Any;

impl Any {
    pub fn json_path<'a>(&'a self, path: &'a JsonPath<'a>) -> JsonPathIter<'a> {
        JsonPathIter::new(path, self)
    }
}

pub struct JsonPathIter<'a> {
    root: &'a Any,
    current: &'a Any,
    tokens: &'a [JsonPathToken<'a>],
    index: usize,
    stack: Vec<IterItem<'a>>,
}

impl<'a> JsonPathIter<'a> {
    fn new(path: &'a JsonPath<'a>, root: &'a Any) -> Self {
        Self {
            root,
            current: root,
            tokens: &path.tokens,
            index: 0,
            stack: Vec::default(),
        }
    }
}

impl<'a> Iterator for JsonPathIter<'a> {
    type Item = &'a Any;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index > self.tokens.len() {
            None // reached the end
        } else if self.index == self.tokens.len() {
            self.index += 1;
            Some(self.current)
        } else {
            match &self.tokens[self.index] {
                JsonPathToken::Root => {
                    self.current = self.root;
                }
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
                    // do nothing
                }
                JsonPathToken::Descend => {
                    // do nothing
                }
                JsonPathToken::Slice(slice) => {
                    // do nothing
                }
            }
            self.index += 1;
            return self.next();
        }
    }
}

struct IterItem<'a> {
    index: usize,
    current: &'a Any,
    state: IterState,
}

enum IterState {}

#[cfg(test)]
mod test {
    use crate::json_path::JsonPath;
    use crate::{any, Any};

    fn sample() -> Any {
        any!({
          "friends": [
            { "name": "Alice" },
            { "name": "Bob" }
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
              { "name": "Bob" }
            ])]
        );
    }

    #[test]
    fn eval_member_full() {
        let any = sample();
        let path = JsonPath::parse("$.friends[1].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Bob")]);
    }

    #[test]
    fn eval_member_negative_index() {
        let any = sample();
        let path = JsonPath::parse("$.friends[-2].name").unwrap();
        let values: Vec<_> = any.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Alice")]);
    }
}

use crate::json_path::JsonPathToken;
use crate::{
    Any, Array, JsonPath, JsonPathEval, Map, Out, ReadTxn, Store, Transact, Xml, XmlFragment,
};

impl<T> JsonPathEval for T
where
    T: ReadTxn,
{
    type Iter<'a>
        = JsonPathIter<'a, T>
    where
        T: 'a;
    fn json_path<'a>(&'a self, path: &'a JsonPath<'a>) -> Self::Iter<'a> {
        JsonPathIter::new(self, path.as_ref())
    }
}

fn slice_iter<'a>(
    value: &'a Out,
    from: usize,
    to: usize,
    by: usize,
) -> Option<Box<dyn Iterator<Item = Out> + 'a>> {
    match value {
        Any::Array(array) => {
            let to = array.len().min(to);
            Some(Box::new(
                array.iter().skip(from).take(to - from).step_by(by),
            ))
        }
        _ => None,
    }
}

fn any_iter<'a, T: ReadTxn>(
    txn: &'a T,
    out: Option<&'a Out>,
) -> Option<Box<dyn Iterator<Item = Out> + 'a>> {
    match out {
        None => Some(Box::new(txn.root_refs().map(|(_, out)| out))),
        Some(Out::Any())
    }
}

fn member_union_iter<'a>(
    value: &'a Out,
    members: &'a [&'a str],
) -> Option<Box<dyn Iterator<Item = &'a Any> + 'a>> {
    match value {
        Any::Map(map) => {
            let iter = members.into_iter().flat_map(move |key| map.get(*key));
            Some(Box::new(iter))
        }
        _ => None,
    }
}

fn index_union_iter<'a>(
    value: &'a Out,
    indices: &'a [i32],
) -> Option<Box<dyn Iterator<Item = Out> + 'a>> {
    match value {
        Any::Array(array) => {
            let iter = indices.into_iter().flat_map(move |i| {
                let i = if *i < 0 {
                    (array.len() as i32 + *i) as usize
                } else {
                    *i as usize
                };
                array.get(i)
            });
            Some(Box::new(iter))
        }
        _ => None,
    }
}

pub struct JsonPathIter<'a, T> {
    txn: &'a T,
    pattern: &'a [JsonPathToken<'a>],
    frame: ExecutionFrame<'a>,
}

impl<'a, T> JsonPathIter<'a, T>
where
    T: ReadTxn,
{
    fn new(txn: &'a T, path: &'a [JsonPathToken<'a>]) -> Self {
        let store = txn.store();
        Self {
            txn: store,
            pattern: path.as_ref(),
            frame: ExecutionFrame::new(None, 0, None),
        }
    }
}

impl<'a, T> Iterator for JsonPathIter<'a, T>
where
    T: ReadTxn,
{
    type Item = Out;

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
        } else if frame.index == self.pattern.len() {
            return if frame.ascend() { self.next() } else { None };
        }
        // early return only works if the loop after has at least one iteration
        let mut early_return = false;
        while frame.index < self.pattern.len() {
            let segment = &self.pattern[frame.index];
            frame.index += 1;
            match segment {
                JsonPathToken::Root => frame.current = None,
                JsonPathToken::Current => { /* do nothing */ }
                JsonPathToken::Member(key) => {
                    if let Some(value) = get_member(self.txn, frame.current.as_ref(), *key) {
                        frame.current = Some(value);
                    } else {
                        early_return = true;
                    }
                }
                JsonPathToken::Index(idx) => {
                    if let Some(value) = get_index(self.txn, frame.current.as_ref(), *idx) {
                        frame.current = Some(value);
                    } else {
                        early_return = true;
                    }
                }
                JsonPathToken::Wildcard => {
                    if let Some(iter) = any_iter(self.txn, frame.current.as_ref()) {
                        frame.iter = Some(iter);
                        return self.next();
                    }
                    early_return = true;
                }
                JsonPathToken::RecursiveDescend => {
                    if let Some(iter) = any_iter(self.txn, frame.current.as_ref()) {
                        frame.iter = Some(iter);
                        frame.is_descending = true;
                        return self.next();
                    }
                    early_return = true;
                }
                JsonPathToken::Slice(from, to, by) => {
                    if let Some(iter) = slice_iter(
                        self.txn,
                        frame.current.as_ref(),
                        *from as usize,
                        *to as usize,
                        *by as usize,
                    ) {
                        frame.iter = Some(iter);
                        return self.next();
                    }
                    early_return = true;
                }
                JsonPathToken::MemberUnion(members) => {
                    if let Some(iter) =
                        member_union_iter(self.txn, frame.current.as_ref(), &members)
                    {
                        frame.iter = Some(iter);
                        return self.next();
                    }
                    early_return = true;
                }
                JsonPathToken::IndexUnion(indices) => {
                    if let Some(iter) = index_union_iter(self.txn, frame.current.as_ref(), &indices)
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
            return frame.current.clone();
        } else if frame.is_descending {
            if let Some(iter) = any_iter(self.txn, frame.current.as_ref()) {
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

fn get_member<T: ReadTxn>(txn: &T, out: Option<&Out>, key: &str) -> Option<Out> {
    match out {
        None => txn.get(key),
        Some(Out::YMap(map)) => map.get(txn, key),
        Some(Out::Any(Any::Map(map))) => map.get(key).map(|any| Out::Any(any.clone())),
        Some(Out::YXmlElement(elem)) => elem
            .get_attribute(txn, key)
            .map(|attr| Out::Any(Any::String(attr.into()))),
        Some(Out::YXmlText(elem)) => elem
            .get_attribute(txn, key)
            .map(|attr| Out::Any(Any::String(attr.into()))),
        _ => None,
    }
}

fn get_index<T: ReadTxn>(txn: &T, out: Option<&Out>, idx: i32) -> Option<Out> {
    match out {
        Some(Out::YArray(array)) => {
            let idx = if *idx < 0 {
                array.len(txn) as i32 + idx
            } else {
                *idx
            } as u32;
            array.get(txn, idx)
        }
        Some(Out::Any(Any::Array(array))) => {
            let idx = if *idx < 0 {
                array.len() as i32 + idx
            } else {
                *idx
            } as usize;
            array.get(idx).cloned().map(Out::Any)
        }
        Some(Out::YXmlFragment(elem)) => {
            let idx = if *idx < 0 {
                elem.len(txn) as i32 + idx
            } else {
                *idx
            } as u32;
            elem.get(txn, idx).map(Out::from)
        }
        Some(Out::YXmlElement(elem)) => {
            let idx = if *idx < 0 {
                elem.len(txn) as i32 + idx
            } else {
                *idx
            } as u32;
            elem.get(txn, idx).map(Out::from)
        }
        _ => None,
    }
}

/// Scope used for recursive iteration, i.e. wildcard, descent or slice.
struct ExecutionFrame<'a> {
    /// Offset to tokens array, where the current scope starts.
    index: usize,
    /// Whether we're in recursive descent scope.
    is_descending: bool,
    /// Current object this scope is iterating over.
    current: Option<Out>,
    /// Iterator used by this scope.
    iter: Option<ScopeIterator<'a>>,
    /// Scopes can be nested in each other i.e. `$.people.*.friends[*]name`. In such case they
    /// are organized in a linked list, with the first elements being the innermost scopes.
    next: Option<Box<ExecutionFrame<'a>>>,
}

impl<'a> ExecutionFrame<'a> {
    fn new(current: Option<Out>, index: usize, iter: Option<ScopeIterator<'a>>) -> Self {
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
    fn descend(&mut self, current: Out) {
        let new_self = ExecutionFrame {
            index: self.index,
            is_descending: self.is_descending,
            current: Some(current),
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

type ScopeIterator<'a> = Box<dyn Iterator<Item = Out> + 'a>;

#[cfg(test)]
mod test {
    use crate::{
        any, Any, Array, ArrayPrelim, Doc, In, JsonPath, JsonPathEval, Map, MapPrelim, Transact,
        WriteTxn,
    };

    fn mixed_sample() -> Doc {
        let doc = Doc::new();
        let mut tx = doc.transact_mut();
        let users = tx.get_or_insert_array("users");
        users.insert(
            &mut tx,
            0,
            MapPrelim::from([
                ("name".into(), any!("Alice").into()),
                ("surname".into(), any!("Smith").into()),
                ("age".into(), any!(25).into()),
                (
                    "friends".into(),
                    In::from(ArrayPrelim::from([
                        any!({ "name": "Bob", "nick": "boreas" }),
                        any!({ "nick": "crocodile91" }),
                    ])),
                ),
            ]),
        );
        users.insert(
            &mut tx,
            1,
            MapPrelim::from([
                ("name".into(), any!("Bob").into()),
                ("nick".into(), any!("boreas").into()),
                ("age".into(), any!(30).into()),
            ]),
        );
        users.insert(
            &mut tx,
            2,
            MapPrelim::from([
                ("nick".into(), any!("crocodile91").into()),
                ("age".into(), any!(35).into()),
            ]),
        );
        users.insert(
            &mut tx,
            3,
            MapPrelim::from([
                ("name".into(), any!("Damian").into()),
                ("surname".into(), any!("Smith").into()),
                ("age".into(), any!(30).into()),
            ]),
        );
        users.insert(
            &mut tx,
            4,
            MapPrelim::from([
                ("name".into(), any!("Elise").into()),
                ("age".into(), any!(35).into()),
            ]),
        );
        doc
    }

    #[test]
    fn eval_member_partial() {
        let doc = mixed_sample();
        let path = JsonPath::parse("$.users").unwrap();
        let tx = doc.transact();
        let values: Vec<_> = tx.json_path(&path).collect();
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
        let doc = mixed_sample();
        let path = JsonPath::parse("$.users[0].name").unwrap();
        let tx = doc.transact();
        let values: Vec<_> = tx.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Alice")]);
    }

    #[test]
    fn eval_member_negative_index() {
        let doc = mixed_sample();
        let path = JsonPath::parse("$.users[-1].name").unwrap();
        let tx = doc.transact();
        let values: Vec<_> = tx.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Elise")]);
    }

    #[test]
    fn eval_member_wildcard_array() {
        let doc = mixed_sample();
        let path = JsonPath::parse("$.users[*].name").unwrap();
        let tx = doc.transact();
        let values: Vec<_> = tx.json_path(&path).collect();
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
        let doc = mixed_sample();
        let path = JsonPath::parse("$.users[1:3].nick").unwrap();
        let tx = doc.transact();
        let values: Vec<_> = tx.json_path(&path).collect();
        assert_eq!(values, vec![&any!("boreas"), &any!("crocodile91")]);
    }

    #[test]
    fn eval_index_union() {
        let doc = mixed_sample();
        let path = JsonPath::parse("$.users[1,3].name").unwrap();
        let tx = doc.transact();
        let values: Vec<_> = tx.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Bob"), &any!("Damian")]);
    }

    #[test]
    fn eval_member_union() {
        let doc = mixed_sample();
        let path = JsonPath::parse("$.users[0]['name','surname']").unwrap();
        let tx = doc.transact();
        let values: Vec<_> = tx.json_path(&path).collect();
        assert_eq!(values, vec![&any!("Alice"), &any!("Smith")]);
    }

    #[test]
    fn eval_descent_flat() {
        let doc = mixed_sample();
        let path = JsonPath::parse("$.users..name").unwrap();
        let tx = doc.transact();
        let values: Vec<_> = tx.json_path(&path).collect();
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
}

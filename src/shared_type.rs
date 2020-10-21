use crate::*;

#[wasm_bindgen]
impl Type {
    #[wasm_bindgen(js_name = toString)]
    #[allow(clippy::inherent_to_string)]
    pub fn to_string (&self) -> String {
        let ss = &self.doc.borrow().ss;
        let mut start = self.inner.start.get();

        let mut s = String::new();
        while let Some(a) = start.as_ref() {
            let item = ss.get_item(&a);
            s.push(item.content);
            start = item.right
        }
        s
    }
    fn find_list_pos (&self, ss: &BlockStore, pos: u32) -> ItemPosition {
        if pos == 0 {
            ItemPosition {
                parent: self,
                after: None
            }
        } else {
            let mut ptr = &self.inner.start.get();
            let mut curr_pos = 1;
            while curr_pos != pos {
                if let Some(a) = ptr.as_ref() {
                    ptr = &ss.get_item(a).right;
                    curr_pos += 1;
                } else {
                    // todo: throw error here
                    break;
                }
            }
            ItemPosition {
                parent: self,
                after: *ptr
            }
        }
    }
    pub fn insert (&self, _: &Transaction, pos: u32, c: char) {
        let mut doc = self.doc.borrow_mut();
        let pos = self.find_list_pos(&doc.ss, pos);
        doc.create_item(&pos, c);
    }
}

pub struct TypeInner {
    pub start: Cell<Option<BlockPtr>>,
    pub ptr: TypePtr
}

pub enum TypePtr {
    Named(u32),
    // Id(ID)
}

impl <'a> TypePtr {
    pub fn clone (&self) -> TypePtr {
        match self {
            TypePtr::Named(tname) => TypePtr::Named(*tname)
        }
    }
}

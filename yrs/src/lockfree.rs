use arc_swap::{ArcSwapOption, AsRaw};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

pub struct Stack<T> {
    head: ArcSwapOption<Node<T>>,
}

impl<T> Stack<T> {
    pub fn new() -> Self {
        Stack {
            head: ArcSwapOption::new(None),
        }
    }

    #[allow(unused)]
    pub fn push(&self, value: T) {
        let mut node = Arc::new(Node::new(value));
        let cur = self.head.load();
        loop {
            {
                // update new node next pointer to point to current head
                // it's safe to unwrap, since until current node is successfully inserted
                // there will be no more that a single Arc reference to it
                let n = Arc::get_mut(&mut node).unwrap();
                n.next.store(cur.clone());
            }

            let prev = self.head.compare_and_swap(&*cur, Some(node.clone()));
            let swapped = std::ptr::eq(prev.as_raw(), cur.as_raw());
            if swapped {
                // we successfully swapped the head, we can exit the loop
                break;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.head.load().is_none()
    }

    pub fn clear(&self) {
        self.head.store(None);
    }

    pub fn remove_where(&self, predicate: impl Fn(&T) -> bool) -> bool {
        while let Some(head) = self.head.load_full() {
            if predicate(&head.value) {
                // the element to remove is the head of the list
                // we need to swap head pointer of self to the next element
                let next = head.next.load_full();
                let prev = self.head.compare_and_swap(&head, next);
                if !std::ptr::eq(prev.as_raw(), Arc::as_ptr(&head)) {
                    // head changed, retry
                    continue;
                } else {
                    return true;
                }
            } else {
                // the element to remove is somewhere in the middle of the list
                // we need to find it and repoint its predecessor's next pointer
                // to its successor
                let mut prev = head;
                while let Some(next) = prev.next.load_full() {
                    if predicate(&next.value) {
                        prev.next.store(next.next.load_full());
                        return true;
                    }
                    prev = next;
                }
            }
        }
        false
    }

    #[inline]
    pub fn each(&self, mut f: impl FnMut(&T)) {
        let mut current = self.head.load();
        while let Some(node) = &*current {
            f(&node.value);
            current = node.next.load();
        }
    }
}

impl<T: Eq> Stack<T> {
    pub fn push_unique(&self, value: T) {
        let mut node = Arc::new(Node::new(value));
        let cur = self.head.load();
        let curr = loop {
            {
                // update new node next pointer to point to current head
                // it's safe to unwrap, since until current node is successfully inserted
                // there will be no more that a single Arc reference to it
                let n = Arc::get_mut(&mut node).unwrap();
                n.next.store(cur.clone());
            }

            let prev = self.head.compare_and_swap(&*cur, Some(node.clone()));
            let swapped = std::ptr::eq(prev.as_raw(), cur.as_raw());
            if swapped {
                // we successfully swapped the head, we can exit the loop
                break node;
            }
        };

        let mut prev = curr.clone();
        while let Some(next) = prev.next.load_full() {
            if next.value == curr.value {
                prev.next.store(next.next.load_full());
                return;
            }
            prev = next;
        }
    }
}

impl<T> Default for Stack<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct Node<T> {
    value: T,
    next: ArcSwapOption<Node<T>>,
}

impl<T> Node<T> {
    fn new(value: T) -> Self {
        Node {
            value,
            next: Default::default(),
        }
    }
}

impl<T> Deref for Node<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for Node<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

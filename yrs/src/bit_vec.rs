use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::fmt::{Debug, Display, Formatter};

pub struct BitVec<T> {
    capacity: usize,
    /// Number of element currently stored inside of BitVec.
    count: usize,
    //Note: occupation and values could be small vecs
    occupation: *mut u8,
    values: *mut T,
}

impl<T> BitVec<T> {
    pub fn new(capacity: usize) -> Self {
        let occupation = unsafe { alloc_zeroed(Layout::array::<u8>(1 + capacity / 8).unwrap()) };
        let values = unsafe { alloc_zeroed(Layout::array::<T>(capacity).unwrap()) as *mut T };
        BitVec {
            count: 0,
            capacity,
            occupation,
            values,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn pop(&mut self, index: usize) -> Option<T> {
        if self.set_unoccupied(index) {
            self.count -= 1;
            Some(unsafe { self.values.offset(index as isize).read() })
        } else {
            None
        }
    }

    pub fn peek(&self, index: usize) -> Option<&T> {
        if self.is_occupied(index) {
            unsafe { self.values.offset(index as isize).as_ref() }
        } else {
            None
        }
    }

    pub fn peek_mut(&mut self, index: usize) -> Option<&mut T> {
        if self.is_occupied(index) {
            unsafe { self.values.offset(index as isize).as_mut() }
        } else {
            None
        }
    }

    pub fn set(&mut self, index: usize, value: T) -> Option<T> {
        let ptr = unsafe { self.values.offset(index as isize) };
        let prev = if self.set_occupied(index) {
            let prev = unsafe { ptr.read() };
            Some(prev)
        } else {
            self.count += 1;
            None
        };
        unsafe {
            ptr.write(value);
        }
        prev
    }

    pub fn iter(&self) -> Iter<T> {
        Iter { vec: self, curr: 0 }
    }

    pub fn into_iter(self) -> IntoIter<T> {
        IntoIter { vec: self, curr: 0 }
    }

    #[inline]
    fn parse_index(index: usize) -> (usize, u8) {
        let byte_position = index / 8;
        let mask = 1u8 << (index & 0b111);
        (byte_position, mask)
    }

    fn is_occupied(&self, index: usize) -> bool {
        let (i, m) = Self::parse_index(index);
        let byte = unsafe { self.occupation.offset(i as isize).read() };
        byte & m == m
    }

    /// Set field at given index as occupied. Returns true if value was not occupied previously.
    fn set_occupied(&mut self, index: usize) -> bool {
        let (i, m) = Self::parse_index(index);
        unsafe {
            let ptr = self.occupation.offset(i as isize);
            let byte = ptr.read();
            ptr.write(byte | m);
            byte & m == 0
        }
    }

    /// Set field at given index as unoccupied. Returns true if value was occupied previously.
    fn set_unoccupied(&mut self, index: usize) -> bool {
        let (i, m) = Self::parse_index(index);
        unsafe {
            let ptr = self.occupation.offset(i as isize);
            let byte = ptr.read();
            ptr.write(byte & !m);
            byte & m == m
        }
    }
}

impl<T> Drop for BitVec<T> {
    fn drop(&mut self) {
        let mut i = 0;
        // drop all values still residing on bitvec, in case they still hold any resources
        while i < self.capacity {
            if self.is_occupied(i) {
                unsafe { drop(self.values.offset(i as isize).read()) }
            }
            i += 1;
        }
        unsafe {
            dealloc(
                self.occupation,
                Layout::array::<u8>(1 + self.capacity / 8).unwrap(),
            );
            dealloc(
                self.values as *mut u8,
                Layout::array::<T>(self.capacity).unwrap(),
            );
        }
    }
}

impl<T: Debug> Debug for BitVec<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "BitVec {{")?;
        let mut i = self.iter();
        if let Some(v) = i.next() {
            write!(f, "{:?}", v)?;
            while let Some(v) = i.next() {
                write!(f, ", {:?}", v)?;
            }
        }
        write!(f, "}}")
    }
}

impl<T: Display> Display for BitVec<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        let mut i = self.iter();
        if let Some(v) = i.next() {
            write!(f, "{}", v)?;
            while let Some(v) = i.next() {
                write!(f, ", {}", v)?;
            }
        }
        write!(f, "]")
    }
}

pub struct Iter<'a, T> {
    vec: &'a BitVec<T>,
    curr: usize,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        while self.curr < self.vec.capacity {
            let i = self.curr;
            self.curr += 1;
            if self.vec.is_occupied(i) {
                return unsafe { self.vec.values.offset(i as isize).as_ref() };
            }
        }
        None
    }
}

pub struct IntoIter<T> {
    vec: BitVec<T>,
    curr: usize,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        while self.curr < self.vec.capacity {
            let i = self.curr;
            self.curr += 1;
            if self.vec.set_unoccupied(i) {
                return Some(unsafe { self.vec.values.offset(i as isize).read() });
            }
        }
        None
    }
}

#[cfg(test)]
mod test {
    use crate::bit_vec::BitVec;

    #[test]
    fn peek() {
        let capacity = 14;
        let mut v = BitVec::new(capacity);
        for i in 0..capacity {
            v.set(i, i.to_string());
        }
        for i in 0..capacity {
            let str = v.peek(i);
            assert_eq!(str, Some(&i.to_string()));
        }
        // peek doesn't remove original value so it should be possible to take it again
        for i in 0..capacity {
            let str = v.peek(i);
            assert_eq!(str, Some(&i.to_string()));
        }
    }

    #[test]
    fn pop() {
        let capacity = 14;
        let mut v = BitVec::new(capacity);
        for i in 0..capacity {
            v.set(i, i.to_string());
        }
        for i in 0..capacity {
            let str = v.pop(i);
            assert_eq!(str, Some(i.to_string()));
        }
        // pop removes original value
        for i in 0..capacity {
            let str = v.pop(i);
            assert!(str.is_none());
        }
    }

    #[test]
    fn iter() {
        let capacity = 14;
        let mut v = BitVec::new(capacity);
        v.set(1, "1");
        v.set(3, "3");
        v.set(4, "4");
        v.set(8, "8");
        v.set(11, "11");

        let mut i = v.iter();
        assert_eq!(i.next(), Some(&"1"));
        assert_eq!(i.next(), Some(&"3"));
        assert_eq!(i.next(), Some(&"4"));
        assert_eq!(i.next(), Some(&"8"));
        assert_eq!(i.next(), Some(&"11"));
        assert_eq!(i.next(), None);
    }

    #[test]
    fn is_empty() {
        let mut v = BitVec::new(10);
        assert!(v.is_empty());
        v.set(1, "1");
        assert!(!v.is_empty());

        // peek doesn't affect size
        v.peek(1);
        assert!(!v.is_empty());

        v.set(1, "2");
        assert!(!v.is_empty());
        v.pop(1);
        assert!(v.is_empty());
    }
}

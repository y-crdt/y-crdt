pub mod client_hasher;

pub(crate) trait OptionExt<T> {
    fn get_or_init(&mut self) -> &mut T;
}

impl<T: Default> OptionExt<T> for Option<Box<T>> {
    fn get_or_init(&mut self) -> &mut T {
        self.get_or_insert_with(|| Box::new(T::default()))
    }
}

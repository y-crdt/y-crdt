use crate::*;

impl Store {
  pub fn get_local_state (&self) -> u32 {
    self.blocks.get_state(self.client_id)
  }
  pub fn get_type<'a>(&'a self, ptr: &types::TypePtr) -> Option<&'a types::Inner> {
      match ptr {
          types::TypePtr::NamedRef(name_ref) => {
            self.types.get(*name_ref as usize).map(|t| &t.0)
          },
          types::TypePtr::Id(id) => {
            // @todo the item might not exist
            if let block::ItemContent::Type(t) = &self.blocks.get_item(id).content {
              Some(t)
            } else {
              None
            }
          },
          types::TypePtr::Named(name) => {
            self.get_type_ref(name).map(|tref| &self.types[tref as usize].0)
          }
      }
  }

  pub fn init_type_from_ptr<'a>(&'a mut self, ptr: &types::TypePtr) -> Option<&'a types::Inner> {
    match ptr {
        types::TypePtr::Named(name) => {
          let id = self.init_type_ref(name);
          self.types.get(id as usize).map(|t| &t.0)
        }
        _ => {
          if let Some(inner) = self.get_type(ptr) {
            return Some(inner)
          } else {
            None
          }
        }
    }
  }
  pub fn get_type_ref(&self, string: &str) -> Option<u32> {
    self.type_refs.get(string).map(|r| *r)
  }
  pub fn init_type_ref(&mut self, string: &str) -> u32 {
      let types = &mut self.types;
      *self.type_refs.entry(string.to_owned()).or_insert_with(|| {
        let name_ref = types.len() as u32;
        let ptr = types::TypePtr::NamedRef(name_ref);
        let inner = types::Inner::new(ptr, None, types::TypeRefs::YArray);
        types.push((
          inner,
          string.to_owned(),
        ));
        name_ref
      })
  }
}
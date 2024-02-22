use crate::js::Js;
use crate::Result;
use std::cell::{Ref, RefMut};
use std::ops::Deref;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use yrs::TransactionMut;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "YTransaction | null = null")]
    pub type ImplicitTransaction;
}
enum Cell<'a, T> {
    Owned(T),
    Borrowed(&'a T),
}

#[wasm_bindgen]
pub struct YTransaction {
    inner: Cell<'static, TransactionMut<'static>>,
}

impl YTransaction {
    pub fn from_implicit<'a>(txn: &'a ImplicitTransaction) -> Option<Ref<'a, Self>> {
        todo!()
    }

    pub fn from_implicit_mut<'a>(txn: &'a mut ImplicitTransaction) -> Option<RefMut<'a, Self>> {
        todo!()
    }

    pub fn from_ref(txn: &TransactionMut) -> Self {
        let txn: &'static TransactionMut<'static> = unsafe { std::mem::transmute(txn) };
        YTransaction {
            inner: Cell::Borrowed(txn),
        }
    }

    pub fn as_mut(&mut self) -> Result<&mut TransactionMut<'static>> {
        match &mut self.inner {
            Cell::Owned(v) => Ok(v),
            Cell::Borrowed(v) => Err(JsValue::from_str(
                "cannot modify transaction in this context",
            )),
        }
    }
}

#[wasm_bindgen]
impl YTransaction {
    #[wasm_bindgen(getter, js_name = origin)]
    pub fn origin(&self) -> JsValue {
        if let Some(origin) = self.deref().origin() {
            Js::from(origin).into()
        } else {
            JsValue::UNDEFINED
        }
    }
}

impl<'doc> From<TransactionMut<'doc>> for YTransaction {
    fn from(value: TransactionMut<'doc>) -> Self {
        let txn: TransactionMut<'static> = unsafe { std::mem::transmute(value) };
        YTransaction {
            inner: Cell::Owned(txn),
        }
    }
}

impl Deref for YTransaction {
    type Target = TransactionMut<'static>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        match &self.inner {
            Cell::Owned(v) => v,
            Cell::Borrowed(v) => *v,
        }
    }
}

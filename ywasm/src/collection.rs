use crate::js::Js;
use crate::transaction::{ImplicitTransaction, YTransaction};
use crate::Result;
use std::ops::Deref;
use wasm_bindgen::JsValue;
use yrs::{Desc, Doc, Origin, ReadTxn, SharedRef, Transact, Transaction, TransactionMut};

pub enum SharedCollection<P, S> {
    Prelim(P),
    Integrated(Integrated<S>),
}

impl<P, S: SharedRef + 'static> SharedCollection<P, S> {
    #[inline]
    pub fn prelim(prelim: P) -> Self {
        SharedCollection::Prelim(prelim)
    }

    #[inline]
    pub fn integrated(shared_ref: S, doc: Doc) -> Self {
        SharedCollection::Integrated(Integrated::new(shared_ref, doc))
    }

    #[inline]
    pub fn is_prelim(&self) -> bool {
        match self {
            SharedCollection::Prelim(_) => true,
            SharedCollection::Integrated(_) => false,
        }
    }

    pub fn is_alive(&self, txn: &YTransaction) -> bool {
        match self {
            SharedCollection::Prelim(_) => true,
            SharedCollection::Integrated(col) => {
                let desc = &col.desc;
                desc.get(txn.deref()).is_some()
            }
        }
    }
}

pub struct Integrated<S> {
    pub desc: Desc<S>,
    pub doc: Doc,
}

impl<S: SharedRef + 'static> Integrated<S> {
    pub fn new(shared_ref: S, doc: Doc) -> Self {
        let desc = shared_ref.desc();
        Integrated { desc, doc }
    }

    pub fn readonly<F, T>(&self, txn: ImplicitTransaction, f: F) -> Result<T>
    where
        F: FnOnce(&S, &TransactionMut<'_>) -> Result<T>,
    {
        match YTransaction::from_implicit(&txn) {
            Some(txn) => {
                let txn: &TransactionMut = &*txn;
                let shared_ref = self.unpack(txn)?;
                f(&shared_ref, txn)
            }
            None => {
                let txn = self.transact_mut()?;
                let shared_ref = self.unpack(&txn)?;
                f(&shared_ref, &txn)
            }
        }
    }

    pub fn mutably<F, T>(&self, mut txn: ImplicitTransaction, f: F) -> Result<T>
    where
        F: FnOnce(&S, &mut TransactionMut<'_>) -> Result<T>,
    {
        match YTransaction::from_implicit_mut(&mut txn) {
            Some(mut txn) => {
                let txn = txn.as_mut()?;
                let shared_ref = self.unpack(txn)?;
                f(&shared_ref, txn)
            }
            None => {
                let mut txn = self.transact_mut()?;
                let shared_ref = self.unpack(&mut txn)?;
                f(&shared_ref, &mut txn)
            }
        }
    }

    pub fn unpack<T: ReadTxn>(&self, txn: &T) -> Result<S> {
        match self.desc.get(txn) {
            Some(shared_ref) => Ok(shared_ref),
            None => Err(JsValue::from_str("shared collection has been destroyed")),
        }
    }

    pub fn transact(&self) -> Result<Transaction> {
        match self.doc.try_transact() {
            Ok(tx) => Ok(tx),
            Err(_) => Err(JsValue::from_str(
                "another read-write transaction is in progress",
            )),
        }
    }

    pub fn transact_mut(&self) -> Result<TransactionMut> {
        match self.doc.try_transact_mut() {
            Ok(tx) => Ok(tx),
            Err(_) => Err(JsValue::from_str("another transaction is in progress")),
        }
    }

    pub fn transact_mut_with(&self, js: &JsValue) -> Result<TransactionMut> {
        let origin: Origin = Js::from(js.clone()).into();
        match self.doc.try_transact_mut_with(origin) {
            Ok(tx) => Ok(tx),
            Err(_) => Err(JsValue::from_str("another transaction is in progress")),
        }
    }
}

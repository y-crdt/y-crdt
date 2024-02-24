use crate::transaction::{ImplicitTransaction, YTransaction};
use crate::Result;
use std::ops::Deref;
use wasm_bindgen::JsValue;
use yrs::{BranchID, Doc, Hook, ReadTxn, SharedRef, Transact, Transaction, TransactionMut};

pub enum SharedCollection<P, S> {
    Integrated(Integrated<S>),
    Prelim(P),
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
                let desc = &col.hook;
                desc.get(txn.deref()).is_some()
            }
        }
    }

    #[inline]
    pub fn branch_id(&self) -> Option<&BranchID> {
        match self {
            SharedCollection::Prelim(_) => None,
            SharedCollection::Integrated(v) => Some(v.hook.id()),
        }
    }
}

pub struct Integrated<S> {
    pub hook: Hook<S>,
    pub doc: Doc,
}

impl<S: SharedRef + 'static> Integrated<S> {
    pub fn new(shared_ref: S, doc: Doc) -> Self {
        let desc = shared_ref.hook();
        Integrated { hook: desc, doc }
    }

    pub fn readonly<F, T>(&self, txn: ImplicitTransaction, f: F) -> Result<T>
    where
        F: FnOnce(&S, &TransactionMut<'_>) -> Result<T>,
    {
        match YTransaction::from_implicit(&txn)? {
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
        match YTransaction::from_implicit_mut(&mut txn)? {
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
        match self.hook.get(txn) {
            Some(shared_ref) => Ok(shared_ref),
            None => Err(JsValue::from_str(crate::js::errors::REF_DISPOSED)),
        }
    }

    pub fn transact(&self) -> Result<Transaction> {
        match self.doc.try_transact() {
            Ok(tx) => Ok(tx),
            Err(_) => Err(JsValue::from_str(crate::js::errors::ANOTHER_RW_TX)),
        }
    }

    pub fn transact_mut(&self) -> Result<TransactionMut> {
        match self.doc.try_transact_mut() {
            Ok(tx) => Ok(tx),
            Err(_) => Err(JsValue::from_str(crate::js::errors::ANOTHER_TX)),
        }
    }
}

use crate::transaction::Transaction;
use crate::Result;
use gloo_utils::format::JsValueSerdeExt;
use std::ops::Deref;
use wasm_bindgen::JsValue;
use yrs::{BranchID, Doc as YDoc, Hook, SharedRef, TransactionMut as YTransaction};

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
    pub fn integrated(shared_ref: S, doc: crate::Doc) -> Self {
        SharedCollection::Integrated(Integrated::new(shared_ref, doc))
    }

    pub fn id(&self) -> crate::Result<JsValue> {
        match self {
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
            SharedCollection::Integrated(c) => {
                let branch_id = c.hook.id();
                JsValue::from_serde(branch_id).map_err(|e| JsValue::from_str(&e.to_string()))
            }
        }
    }

    pub fn try_integrated(&self) -> Result<(&BranchID, &crate::Doc)> {
        match self {
            SharedCollection::Integrated(i) => {
                let branch_id = i.hook.id();
                let doc = &i.doc;
                Ok((branch_id, doc))
            }
            SharedCollection::Prelim(_) => {
                Err(JsValue::from_str(crate::js::errors::INVALID_PRELIM_OP))
            }
        }
    }

    #[inline]
    pub fn is_prelim(&self) -> bool {
        match self {
            SharedCollection::Prelim(_) => true,
            SharedCollection::Integrated(_) => false,
        }
    }

    pub fn is_alive(&self) -> bool {
        match self {
            SharedCollection::Prelim(_) => true,
            SharedCollection::Integrated(col) => col.transact(|_, _| Ok(())).is_ok(),
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
    pub doc: crate::Doc,
}

impl<S: SharedRef + 'static> Integrated<S> {
    pub fn new(shared_ref: S, doc: crate::Doc) -> Self {
        let desc = shared_ref.hook();
        Integrated { hook: desc, doc }
    }

    pub fn transact<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&S, &mut YTransaction) -> Result<T>,
    {
        self.doc.transact(JsValue::UNDEFINED, |tx| {
            let shared_ref = self
                .hook
                .get(tx)
                .ok_or_else(|| JsValue::from_str(crate::js::errors::REF_DISPOSED))?;
            f(&shared_ref, tx)
        })
    }
}

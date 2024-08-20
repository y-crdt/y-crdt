use crate::{Doc, Origin, Store, Transaction, TransactionMut};
use async_lock::futures::{Read, Write};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use thiserror::Error;

/// Trait implemented by [Doc] and shared types, used for carrying over the responsibilities of
/// creating new transactions, used as a unit of work in Yrs.
pub trait Transact {
    /// Creates and returns a lightweight read-only transaction.
    ///
    /// # Errors
    ///
    /// While it's possible to have multiple read-only transactions active at the same time,
    /// this method will return a [TransactionAcqError::SharedAcqFailed] error whenever called
    /// while a read-write transaction (see: [Self::try_transact_mut]) is active at the same time.
    fn try_transact(&self) -> Result<Transaction, TransactionAcqError>;

    /// Creates and returns a read-write capable transaction. This transaction can be used to
    /// mutate the contents of underlying document store and upon dropping or committing it may
    /// subscription callbacks.
    ///
    /// # Errors
    ///
    /// Only one read-write transaction can be active at the same time. If any other transaction -
    /// be it a read-write or read-only one - is active at the same time, this method will return
    /// a [TransactionAcqError::ExclusiveAcqFailed] error.
    fn try_transact_mut(&self) -> Result<TransactionMut, TransactionAcqError>;

    /// Creates and returns a read-write capable transaction with an `origin` classifier attached.
    /// This transaction can be used to mutate the contents of underlying document store and upon
    /// dropping or committing it may subscription callbacks.
    ///
    /// An `origin` may be used to identify context of operations made (example updates performed
    /// locally vs. incoming from remote replicas) and it's used i.e. by [`UndoManager`][crate::undo::UndoManager].
    ///
    /// # Errors
    ///
    /// Only one read-write transaction can be active at the same time. If any other transaction -
    /// be it a read-write or read-only one - is active at the same time, this method will return
    /// a [TransactionAcqError::ExclusiveAcqFailed] error.
    fn try_transact_mut_with<T>(&self, origin: T) -> Result<TransactionMut, TransactionAcqError>
    where
        T: Into<Origin>;

    /// Creates and returns a read-write capable transaction with an `origin` classifier attached.
    /// This transaction can be used to mutate the contents of underlying document store and upon
    /// dropping or committing it may subscription callbacks.
    ///
    /// An `origin` may be used to identify context of operations made (example updates performed
    /// locally vs. incoming from remote replicas) and it's used i.e. by [`UndoManager`][crate::undo::UndoManager].
    ///
    /// # Errors
    ///
    /// Only one read-write transaction can be active at the same time. If any other transaction -
    /// be it a read-write or read-only one - is active at the same time, this method will panic.
    fn transact_mut_with<T>(&self, origin: T) -> TransactionMut
    where
        T: Into<Origin>,
    {
        self.try_transact_mut_with(origin).unwrap()
    }

    /// Creates and returns a lightweight read-only transaction.
    ///
    /// # Panics
    ///
    /// While it's possible to have multiple read-only transactions active at the same time,
    /// this method will panic whenever called while a read-write transaction
    /// (see: [Self::transact_mut]) is active at the same time.
    fn transact(&self) -> Transaction {
        self.try_transact().unwrap()
    }

    /// Creates and returns a read-write capable transaction. This transaction can be used to
    /// mutate the contents of underlying document store and upon dropping or committing it may
    /// subscription callbacks.
    ///
    /// # Panics
    ///
    /// Only one read-write transaction can be active at the same time. If any other transaction -
    /// be it a read-write or read-only one - is active at the same time, this method will panic.
    fn transact_mut(&self) -> TransactionMut {
        self.try_transact_mut().unwrap()
    }
}

impl Transact for Doc {
    fn try_transact(&self) -> Result<Transaction, TransactionAcqError> {
        match self.store.try_read() {
            Some(store) => Ok(Transaction::new(store)),
            None => Err(TransactionAcqError::SharedAcqFailed),
        }
    }

    fn try_transact_mut(&self) -> Result<TransactionMut, TransactionAcqError> {
        match self.store.try_write() {
            Some(store) => Ok(TransactionMut::new(self.clone(), store, None)),
            None => Err(TransactionAcqError::ExclusiveAcqFailed),
        }
    }

    fn try_transact_mut_with<T>(&self, origin: T) -> Result<TransactionMut, TransactionAcqError>
    where
        T: Into<Origin>,
    {
        match self.store.try_write() {
            Some(store) => Ok(TransactionMut::new(
                self.clone(),
                store,
                Some(origin.into()),
            )),
            None => Err(TransactionAcqError::ExclusiveAcqFailed),
        }
    }

    fn transact_mut_with<T>(&self, origin: T) -> TransactionMut
    where
        T: Into<Origin>,
    {
        let lock = self.store.write_blocking();
        TransactionMut::new(self.clone(), lock, Some(origin.into()))
    }

    fn transact(&self) -> Transaction {
        let lock = self.store.read_blocking();
        Transaction::new(lock)
    }

    fn transact_mut(&self) -> TransactionMut {
        let lock = self.store.write_blocking();
        TransactionMut::new(self.clone(), lock, None)
    }
}

/// Trait implemented by [Doc] and shared types, used for carrying over the responsibilities of
/// creating new transactions, used as a unit of work in Yrs.
pub trait AsyncTransact<'doc> {
    type Read: Future<Output = Transaction<'doc>>;
    type Write: Future<Output = TransactionMut<'doc>>;

    fn transact(&'doc self) -> Self::Read;
    fn transact_mut(&'doc self) -> Self::Write;

    /// Creates and returns a read-write capable transaction with an `origin` classifier attached.
    /// This transaction can be used to mutate the contents of underlying document store and upon
    /// dropping or committing it may subscription callbacks.
    ///
    /// An `origin` may be used to identify context of operations made (example updates performed
    /// locally vs. incoming from remote replicas) and it's used i.e. by [`UndoManager`][crate::undo::UndoManager].
    fn transact_mut_with<T>(&'doc self, origin: T) -> Self::Write
    where
        T: Into<Origin>;
}

impl<'doc> AsyncTransact<'doc> for Doc {
    type Read = AcquireTransaction<'doc>;
    type Write = AcquireTransactionMut<'doc>;

    fn transact(&'doc self) -> Self::Read {
        let fut = self.store.read_async();
        AcquireTransaction { fut }
    }

    fn transact_mut(&'doc self) -> Self::Write {
        let fut = self.store.write_async();
        AcquireTransactionMut {
            doc: self.clone(),
            origin: None,
            fut,
        }
    }

    fn transact_mut_with<T>(&'doc self, origin: T) -> Self::Write
    where
        T: Into<Origin>,
    {
        let fut = self.store.write_async();
        AcquireTransactionMut {
            doc: self.clone(),
            origin: Some(origin.into()),
            fut,
        }
    }
}

pub struct AcquireTransaction<'doc> {
    fut: Read<'doc, Store>,
}

impl<'doc> Unpin for AcquireTransaction<'doc> {}

impl<'doc> Future for AcquireTransaction<'doc> {
    type Output = Transaction<'doc>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pinned = unsafe { Pin::new_unchecked(&mut self.fut) };
        pinned.poll(cx).map(Transaction::new)
    }
}

pub struct AcquireTransactionMut<'doc> {
    doc: Doc,
    origin: Option<Origin>,
    fut: Write<'doc, Store>,
}

impl<'doc> Unpin for AcquireTransactionMut<'doc> {}

impl<'doc> Future for AcquireTransactionMut<'doc> {
    type Output = TransactionMut<'doc>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pinned = unsafe { Pin::new_unchecked(&mut self.fut) };
        match pinned.poll(cx) {
            Poll::Ready(store) => {
                let doc = self.doc.clone();
                let origin = self.origin.take();
                Poll::Ready(TransactionMut::new(doc, store, origin))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Error, Debug)]
pub enum TransactionAcqError {
    #[error("Failed to acquire read-only transaction. Drop read-write transaction and retry.")]
    SharedAcqFailed,
    #[error("Failed to acquire read-write transaction. Drop other transactions and retry.")]
    ExclusiveAcqFailed,
    #[error("All references to a parent document containing this structure has been dropped.")]
    DocumentDropped,
}

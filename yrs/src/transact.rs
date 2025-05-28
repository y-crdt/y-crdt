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
    /// be it a read-write or read-only one - is active at the same time, this method will
    /// block the thread until exclusive access can be acquired, unless building for wasm
    /// which panics. If blocking is undesirable, use `try_transact_mut_with(origin).unwrap()` instead`.
    fn transact_mut_with<T>(&self, origin: T) -> TransactionMut
    where
        T: Into<Origin>;

    /// Creates and returns a lightweight read-only transaction.
    ///
    /// # Errors
    ///
    /// While it's possible to have multiple read-only transactions active at the same time,
    /// this method will block the thread until exclusive access can be acquired, unless building for wasm
    /// which panics. If blocking is undesirable, use `try_transact(origin).unwrap()` instead`.
    fn transact(&self) -> Transaction;

    /// Creates and returns a read-write capable transaction. This transaction can be used to
    /// mutate the contents of underlying document store and upon dropping or committing it may
    /// subscription callbacks.
    ///
    /// # Errors
    ///
    /// Only one read-write transaction can be active at the same time. If any other transaction -
    /// be it a read-write or read-only one - is active at the same time, this method will block
    /// the thread until exclusive access can be acquired, unless building for wasm
    /// which panics. If blocking is undesirable, use `try_transact_mut(origin).unwrap()` instead`.
    fn transact_mut(&self) -> TransactionMut;
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

#[cfg(test)]
mod test {
    use crate::{Doc, GetString, Text, Transact};
    use rand::random;
    use std::sync::{Arc, Barrier};
    use std::time::{Duration, Instant};

    #[test]
    fn multi_thread_transact_mut() {
        let doc = Doc::new();
        let txt = doc.get_or_insert_text("text");

        const N: usize = 3;
        let barrier = Arc::new(Barrier::new(N + 1));

        let start = Instant::now();
        for _ in 0..N {
            let d = doc.clone();
            let t = txt.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                // let mut txn = d.try_transact_mut().unwrap(); // this will hang forever
                let mut txn = d.transact_mut();
                let n = random::<u64>() % 5;
                std::thread::sleep(Duration::from_millis(n * 100));
                t.insert(&mut txn, 0, "a");
                drop(txn);
                b.wait();
            });
        }

        barrier.wait();
        println!("{} threads executed in {:?}", N, Instant::now() - start);

        let expected: String = (0..N).map(|_| 'a').collect();
        let txn = doc.transact();
        let str = txt.get_string(&txn);
        assert_eq!(str, expected);
    }
}

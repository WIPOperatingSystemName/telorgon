use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// External revisioned snapshot handle. Runtime dependency registration is added by
/// `Component::watch` without exposing a stream or callback model to components.
#[derive(Clone)]
pub struct Signal<T> {
    inner: Arc<RwLock<SignalValue<T>>>,
}

#[derive(Clone)]
pub struct SignalWriter<T> {
    inner: Arc<RwLock<SignalValue<T>>>,
}

struct SignalValue<T> {
    revision: u64,
    value: Arc<T>,
    next_subscriber: u64,
    subscribers: HashMap<u64, Arc<dyn Fn() + Send + Sync>>,
}

impl<T> Signal<T> {
    pub fn new(value: T) -> (Self, SignalWriter<T>) {
        let inner = Arc::new(RwLock::new(SignalValue {
            revision: 1,
            value: Arc::new(value),
            next_subscriber: 1,
            subscribers: HashMap::new(),
        }));
        (
            Self {
                inner: inner.clone(),
            },
            SignalWriter { inner },
        )
    }

    pub fn snapshot(&self) -> SignalSnapshot<T> {
        let value = self.inner.read().expect("signal publication lock poisoned");
        SignalSnapshot {
            revision: value.revision,
            value: value.value.clone(),
        }
    }
}

impl<T> Signal<T>
where
    T: Send + Sync + 'static,
{
    pub(crate) fn dependency(&self, observed_revision: u64) -> SignalDependency {
        let inner = self.inner.clone();
        let subscription_inner = self.inner.clone();
        SignalDependency {
            identity: Arc::as_ptr(&self.inner) as usize,
            observed_revision,
            current_revision: Arc::new(move || {
                inner
                    .read()
                    .expect("signal publication lock poisoned")
                    .revision
            }),
            subscribe: Arc::new(move |callback| {
                let subscriber = {
                    let mut value = subscription_inner
                        .write()
                        .expect("signal publication lock poisoned");
                    let subscriber = value.next_subscriber;
                    value.next_subscriber = value.next_subscriber.wrapping_add(1).max(1);
                    value.subscribers.insert(subscriber, callback);
                    subscriber
                };
                let inner = subscription_inner.clone();
                SignalSubscription::new(move || {
                    inner
                        .write()
                        .expect("signal publication lock poisoned")
                        .subscribers
                        .remove(&subscriber);
                })
            }),
        }
    }
}

impl<T> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl<T> Eq for Signal<T> {}

impl<T> SignalWriter<T> {
    pub fn publish(&self, value: T) -> u64 {
        let (revision, subscribers) = {
            let mut current = self
                .inner
                .write()
                .expect("signal publication lock poisoned");
            current.revision = current.revision.wrapping_add(1).max(1);
            current.value = Arc::new(value);
            (
                current.revision,
                current.subscribers.values().cloned().collect::<Vec<_>>(),
            )
        };
        for subscriber in subscribers {
            subscriber();
        }
        revision
    }

    pub fn publish_if_changed(&self, value: T) -> u64
    where
        T: PartialEq,
    {
        let (revision, subscribers) = {
            let mut current = self
                .inner
                .write()
                .expect("signal publication lock poisoned");
            if current.value.as_ref() == &value {
                return current.revision;
            }
            current.revision = current.revision.wrapping_add(1).max(1);
            current.value = Arc::new(value);
            (
                current.revision,
                current.subscribers.values().cloned().collect::<Vec<_>>(),
            )
        };
        for subscriber in subscribers {
            subscriber();
        }
        revision
    }
}

#[derive(Clone)]
pub struct SignalSnapshot<T> {
    pub revision: u64,
    pub value: Arc<T>,
}

impl<T> std::ops::Deref for SignalSnapshot<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

#[doc(hidden)]
#[derive(Clone)]
pub struct SignalDependency {
    identity: usize,
    observed_revision: u64,
    current_revision: Arc<dyn Fn() -> u64 + Send + Sync>,
    subscribe: Arc<dyn Fn(Arc<dyn Fn() + Send + Sync>) -> SignalSubscription + Send + Sync>,
}

impl SignalDependency {
    pub fn identity(&self) -> usize {
        self.identity
    }

    pub fn observed_revision(&self) -> u64 {
        self.observed_revision
    }

    pub fn changed(&self) -> bool {
        (self.current_revision)() != self.observed_revision
    }

    pub fn subscribe(&self, callback: Arc<dyn Fn() + Send + Sync>) -> SignalSubscription {
        (self.subscribe)(callback)
    }
}

/// Keeps one signal invalidation callback registered until the dependent view is rerendered or
/// unmounted. This is runtime plumbing; application code normally only uses `Component::watch`.
#[doc(hidden)]
pub struct SignalSubscription {
    unsubscribe: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl SignalSubscription {
    fn new(unsubscribe: impl FnOnce() + Send + Sync + 'static) -> Self {
        Self {
            unsubscribe: Some(Box::new(unsubscribe)),
        }
    }
}

impl Drop for SignalSubscription {
    fn drop(&mut self) {
        if let Some(unsubscribe) = self.unsubscribe.take() {
            unsubscribe();
        }
    }
}

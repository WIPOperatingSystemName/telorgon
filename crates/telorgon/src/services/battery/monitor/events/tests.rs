use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Wake;

use super::*;

#[derive(Default)]
struct WakeCount(AtomicUsize);
impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn waiting_receivers_are_woken_and_cancellation_preserves_events() {
    let subscriber = Arc::new(Subscriber::new(2));
    let mut events = BatteryEvents::new(subscriber.clone());
    let counter = Arc::new(WakeCount::default());
    let waker = Waker::from(counter.clone());
    let mut cx = Context::from_waker(&waker);
    let mut next = Box::pin(events.next());
    assert!(next.as_mut().poll(&mut cx).is_pending());
    let event = BatteryEvent::Added { id: "BAT0".into() };
    subscriber.emit(std::slice::from_ref(&event));
    assert_eq!(counter.0.load(Ordering::SeqCst), 1);
    drop(next); // Cancel after wake, before consuming the event.
    assert_eq!(events.try_recv(), Some(event));
    let mut next = Box::pin(events.next());
    assert!(next.as_mut().poll(&mut cx).is_pending());
    drop(next);
    assert!(subscriber.queue.lock().unwrap().waker.is_none());
    let mut next = Box::pin(events.next());
    assert!(next.as_mut().poll(&mut cx).is_pending());
    subscriber.close();
    assert_eq!(counter.0.load(Ordering::SeqCst), 2);
    assert_eq!(next.as_mut().poll(&mut cx), Poll::Ready(None));
}

#[must_use = "UI Interaction must be stabilized. Call .stabilize(&mut harness) to process events."]
pub struct Interaction<T> {
    value: T,
}

pub trait Stabilizer {
    fn stabilize(&mut self);
}

impl<T> Interaction<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }

    pub fn stabilize(self, harness: &mut impl Stabilizer) -> T {
        harness.stabilize();
        self.value
    }

    /// For synchronous paths, where the queue is already drained by the time
    /// `send` returns.
    pub fn now(self) -> T {
        self.value
    }
}

/// Records what an actor pushes through a `#[port]`.
///
/// [`PortSpy::sender`] is the port: `#[port]` blanket-implements the trait for
/// any closure taking the message, so no generated double is involved.
pub struct PortSpy<M> {
    sent: std::rc::Rc<std::cell::RefCell<Vec<M>>>,
}

impl<M> Default for PortSpy<M> {
    fn default() -> Self {
        Self {
            sent: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
        }
    }
}

impl<M: 'static> PortSpy<M> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sender(&self) -> impl Fn(M) + Clone + 'static {
        let sent = self.sent.clone();
        move |msg| sent.borrow_mut().push(msg)
    }

    pub fn count(&self) -> usize {
        self.sent.borrow().len()
    }

    /// Drains everything recorded so far.
    pub fn take(&self) -> Interaction<Vec<M>> {
        Interaction::new(std::mem::take(&mut *self.sent.borrow_mut()))
    }

    pub fn last(&self) -> Interaction<Option<M>> {
        Interaction::new(self.sent.borrow_mut().pop())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{Addr, Cx, Handler, UiThreadToken};

    struct Refresh;

    #[derive(Clone, Debug, PartialEq)]
    enum Ui {
        Items(Vec<&'static str>),
    }

    struct Service<P> {
        port: P,
    }

    impl<P: Fn(Ui) + 'static> Handler<Refresh> for Service<P> {
        fn handle(&mut self, _: Refresh, _cx: Cx<Self, Refresh>) {
            (self.port)(Ui::Items(vec!["sshd", "cron"]));
        }
    }

    #[test]
    fn a_closure_is_the_port_and_the_spy_records_what_went_through_it() {
        let spy = PortSpy::<Ui>::new();
        let addr = Addr::new(
            Service {
                port: spy.sender(),
            },
            UiThreadToken::dangerously_create_token_unchecked(),
        );

        addr.send(Refresh);

        assert_eq!(spy.count(), 1);
        assert_eq!(
            spy.last().now(),
            Some(Ui::Items(vec!["sshd", "cron"]))
        );
    }
}

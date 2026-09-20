//! Shared typed outputs emitted by controlled application components.

pub use crate::input::{Activation, ChangeSource, ValueChangePhase as ChangePhase};

/// A proposed controlled value and the interaction that produced it.
///
/// This record does not commit `value`. The parent remains the authoritative owner and decides
/// whether to publish the proposal through its next transaction.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ValueChange<T> {
    pub value: T,
    pub phase: ChangePhase,
    pub source: ChangeSource,
}

impl<T> ValueChange<T> {
    pub const fn new(value: T, phase: ChangePhase, source: ChangeSource) -> Self {
        Self {
            value,
            phase,
            source,
        }
    }

    pub const fn committed(value: T, source: ChangeSource) -> Self {
        Self::new(value, ChangePhase::Commit, source)
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> ValueChange<U> {
        ValueChange {
            value: map(self.value),
            phase: self.phase,
            source: self.source,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    #[test]
    fn mapped_change_preserves_phase_and_source() {
        let change = ValueChange::new(4_u32, ChangePhase::Update, ChangeSource::Directional)
            .map(|value| value.to_string());
        assert_eq!(change.value, "4");
        assert_eq!(change.phase, ChangePhase::Update);
        assert_eq!(change.source, ChangeSource::Directional);
    }

    #[test]
    fn output_values_do_not_require_clone_or_send() {
        struct LocalOnly(Rc<()>);

        let change = ValueChange::committed(LocalOnly(Rc::new(())), ChangeSource::Programmatic);
        assert_eq!(Rc::strong_count(&change.value.0), 1);
    }

    #[test]
    fn activation_and_source_are_the_canonical_input_types() {
        let activation = Activation {
            source: ChangeSource::Accessibility,
        };
        let canonical: crate::input::Activation = activation;
        assert_eq!(canonical.source, crate::input::ChangeSource::Accessibility);
    }
}

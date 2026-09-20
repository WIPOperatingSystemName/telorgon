//! Canonical controlled checkbox value and explicit cycle policy.

/// Parent-owned semantic value consumed by checkboxes and aggregate check controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CheckState {
    #[default]
    Unchecked,
    Checked,
    Mixed,
}

/// Validated order used to derive a requested next [`CheckState`].
///
/// The default two-state policy never returns `Mixed` and rejects `Mixed` as an incompatible
/// controlled input. A tri-state policy contains each state exactly once in caller-selected order;
/// Telorgon therefore does not invent whether `Mixed` advances to `Checked` or `Unchecked`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CheckCyclePolicy {
    order: [CheckState; 3],
    len: u8,
}

impl CheckCyclePolicy {
    /// Canonical binary toggle: `Unchecked -> Checked -> Unchecked`.
    pub const fn two_state() -> Self {
        Self {
            order: [
                CheckState::Unchecked,
                CheckState::Checked,
                CheckState::Unchecked,
            ],
            len: 2,
        }
    }

    /// Creates a tri-state cycle from a complete caller-selected ordering.
    pub fn tri_state(order: [CheckState; 3]) -> Result<Self, CheckCycleError> {
        if order[0] == order[1] || order[0] == order[2] || order[1] == order[2] {
            return Err(CheckCycleError::InvalidTriStateOrder);
        }
        Ok(Self { order, len: 3 })
    }

    pub const fn is_tri_state(self) -> bool {
        self.len == 3
    }

    pub fn states(&self) -> &[CheckState] {
        &self.order[..usize::from(self.len)]
    }

    /// Derives a proposal without mutating the controlled value.
    pub fn next(self, current: CheckState) -> Result<CheckState, CheckCycleError> {
        let states = self.states();
        let Some(index) = states.iter().position(|state| *state == current) else {
            return Err(CheckCycleError::MixedStateInTwoStateCycle);
        };
        Ok(states[(index + 1) % states.len()])
    }
}

impl Default for CheckCyclePolicy {
    fn default() -> Self {
        Self::two_state()
    }
}

impl TryFrom<[CheckState; 3]> for CheckCyclePolicy {
    type Error = CheckCycleError;

    fn try_from(order: [CheckState; 3]) -> Result<Self, Self::Error> {
        Self::tri_state(order)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckCycleError {
    InvalidTriStateOrder,
    MixedStateInTwoStateCycle,
}

impl std::fmt::Display for CheckCycleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTriStateOrder => {
                formatter.write_str("tri-state check cycle must contain every state exactly once")
            }
            Self::MixedStateInTwoStateCycle => {
                formatter.write_str("two-state check cycle cannot consume Mixed")
            }
        }
    }
}

impl std::error::Error for CheckCycleError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_state_policy_toggles_binary_values_and_never_produces_mixed() {
        let policy = CheckCyclePolicy::two_state();
        assert_eq!(policy.next(CheckState::Unchecked), Ok(CheckState::Checked));
        assert_eq!(policy.next(CheckState::Checked), Ok(CheckState::Unchecked));
        assert_eq!(
            policy.next(CheckState::Mixed),
            Err(CheckCycleError::MixedStateInTwoStateCycle)
        );
        assert!(
            [CheckState::Unchecked, CheckState::Checked]
                .into_iter()
                .map(|state| policy.next(state).unwrap())
                .all(|state| state != CheckState::Mixed)
        );
        assert!(!policy.is_tri_state());
        assert_eq!(
            policy.states(),
            &[CheckState::Unchecked, CheckState::Checked]
        );
    }

    #[test]
    fn tri_state_order_explicitly_controls_both_transitions_around_mixed() {
        let mixed_to_checked = CheckCyclePolicy::tri_state([
            CheckState::Unchecked,
            CheckState::Mixed,
            CheckState::Checked,
        ])
        .unwrap();
        assert_eq!(
            mixed_to_checked.next(CheckState::Mixed),
            Ok(CheckState::Checked)
        );
        assert_eq!(
            mixed_to_checked.next(CheckState::Checked),
            Ok(CheckState::Unchecked)
        );

        let mixed_to_unchecked = CheckCyclePolicy::tri_state([
            CheckState::Checked,
            CheckState::Mixed,
            CheckState::Unchecked,
        ])
        .unwrap();
        assert_eq!(
            mixed_to_unchecked.next(CheckState::Mixed),
            Ok(CheckState::Unchecked)
        );
        assert_eq!(
            mixed_to_unchecked.next(CheckState::Unchecked),
            Ok(CheckState::Checked)
        );
    }

    #[test]
    fn tri_state_policy_requires_each_state_exactly_once() {
        for invalid in [
            [
                CheckState::Unchecked,
                CheckState::Unchecked,
                CheckState::Mixed,
            ],
            [CheckState::Checked, CheckState::Mixed, CheckState::Checked],
            [CheckState::Mixed, CheckState::Mixed, CheckState::Mixed],
        ] {
            assert_eq!(
                CheckCyclePolicy::tri_state(invalid),
                Err(CheckCycleError::InvalidTriStateOrder)
            );
        }
    }

    #[test]
    fn valid_tri_state_policy_visits_every_value_and_wraps() {
        let policy = CheckCyclePolicy::try_from([
            CheckState::Unchecked,
            CheckState::Checked,
            CheckState::Mixed,
        ])
        .unwrap();
        assert!(policy.is_tri_state());
        assert_eq!(
            policy.states(),
            &[
                CheckState::Unchecked,
                CheckState::Checked,
                CheckState::Mixed,
            ]
        );

        let first = policy.next(CheckState::Unchecked).unwrap();
        let second = policy.next(first).unwrap();
        let third = policy.next(second).unwrap();
        assert_eq!(
            [first, second, third],
            [
                CheckState::Checked,
                CheckState::Mixed,
                CheckState::Unchecked,
            ]
        );
    }
}

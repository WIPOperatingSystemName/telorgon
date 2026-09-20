//! Source-neutral gesture competition and recognizer transition engines.
//!
//! These values do not schedule timers, capture pointers, consume native events, or invoke
//! component code. Recognizers return arena and deadline requests for their runtime owner to apply.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Duration;

use crate::foundation::PointF;

use crate::input::{PointerButton, PointerId};

/// A recognizer family participating in gesture competition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureKind {
    Tap,
    LongPress,
    Drag,
}

/// Terminal reasons shared by the portable recognizers and their arena owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureCancelReason {
    SlopExceeded,
    ReleasedBeforeRecognition,
    PointerCancelled,
    CaptureLost,
    ArenaLost,
    ViewDeactivated,
    Disabled,
    Unmounted,
}

/// Arena resolution requested by one recognizer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GestureArenaRequest {
    #[default]
    None,
    Accept(PointerId),
    Reject(PointerId),
}

/// Opaque generation-aware token for a host-scheduled recognizer deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GestureDeadlineId {
    pointer: PointerId,
    generation: u64,
}

impl GestureDeadlineId {
    pub fn from_raw(pointer: PointerId, generation: u64) -> Option<Self> {
        (generation != 0).then_some(Self {
            pointer,
            generation,
        })
    }

    pub const fn pointer(self) -> PointerId {
        self.pointer
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// Deadline handoff produced by a recognizer; no timer is started by `telorgon-input`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GestureDeadlineRequest {
    #[default]
    None,
    Schedule {
        id: GestureDeadlineId,
        after: Duration,
    },
    Cancel(GestureDeadlineId),
}

/// Axis used only to test drag slop. Recognized drag deltas always retain both coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DragAxis {
    Horizontal,
    Vertical,
    #[default]
    Both,
}

/// Source-neutral input understood by each recognizer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GestureInput {
    PointerDown {
        pointer: PointerId,
        button: PointerButton,
        position: PointF,
    },
    PointerMoved {
        pointer: PointerId,
        position: PointF,
    },
    PointerUp {
        pointer: PointerId,
        button: PointerButton,
        position: PointF,
    },
    PointerCancelled {
        pointer: PointerId,
    },
    PointerCaptureLost {
        pointer: PointerId,
    },
    ArenaWon {
        pointer: PointerId,
    },
    ArenaLost {
        pointer: PointerId,
    },
    DeadlineElapsed(GestureDeadlineId),
    SetEnabled(bool),
    ViewDeactivated,
    Unmount,
}

/// Recognized movement in logical coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GestureDelta {
    pub x: f32,
    pub y: f32,
}

/// Observable recognizer transition. A transition describes state only and invokes no callback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GestureTransition {
    None,
    Possible {
        kind: GestureKind,
        pointer: PointerId,
        position: PointF,
    },
    TapRecognized {
        pointer: PointerId,
        position: PointF,
    },
    LongPressStarted {
        pointer: PointerId,
        origin: PointF,
        position: PointF,
    },
    LongPressUpdated {
        pointer: PointerId,
        position: PointF,
        delta: GestureDelta,
        total: GestureDelta,
    },
    LongPressEnded {
        pointer: PointerId,
        position: PointF,
        total: GestureDelta,
    },
    DragStarted {
        pointer: PointerId,
        origin: PointF,
        position: PointF,
        total: GestureDelta,
    },
    DragUpdated {
        pointer: PointerId,
        position: PointF,
        delta: GestureDelta,
        total: GestureDelta,
    },
    DragEnded {
        pointer: PointerId,
        position: PointF,
        total: GestureDelta,
    },
    Cancelled {
        kind: GestureKind,
        pointer: PointerId,
        reason: GestureCancelReason,
    },
}

/// Complete handoff from a recognizer transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureOutcome {
    pub transition: GestureTransition,
    pub arena: GestureArenaRequest,
    pub deadline: GestureDeadlineRequest,
}

impl GestureOutcome {
    pub const fn ignored() -> Self {
        Self {
            transition: GestureTransition::None,
            arena: GestureArenaRequest::None,
            deadline: GestureDeadlineRequest::None,
        }
    }

    fn transition(transition: GestureTransition) -> Self {
        Self {
            transition,
            ..Self::ignored()
        }
    }
}

/// Public coarse state shared by all three recognizers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GestureRecognizerState {
    #[default]
    Idle,
    Possible,
    Accepted,
    Dead,
}

/// Invalid configuration or event data. Errors retain semantic recognizer state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GestureRecognizerError {
    InvalidSlop(f32),
    ZeroLongPressDelay,
    NonFinitePosition(PointF),
}

/// Portable recognizer diagnostics for later per-view aggregation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GestureRecognizerDiagnostics {
    pub sequences_started: u64,
    pub recognized: u64,
    pub cancelled: u64,
    pub arena_claims: u64,
    pub arena_rejections: u64,
    pub stale_deadlines: u64,
    pub failures: u64,
}

/// Why an arena member won.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureArenaWinReason {
    Accepted,
    LastRemaining,
    Swept,
}

/// Why an arena member lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureArenaLossReason<K> {
    SelfRejected,
    Winner(K),
    Cancelled(GestureCancelReason),
}

/// Exactly-once result delivered to an arena participant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureArenaDecision<K> {
    Won {
        participant: K,
        reason: GestureArenaWinReason,
    },
    Lost {
        participant: K,
        reason: GestureArenaLossReason<K>,
    },
}

/// Rejected arena operation. Rejections do not alter active contests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureArenaError<K> {
    ArenaAlreadyClosed(PointerId),
    ArenaStillOpen(PointerId),
    UnknownPointer(PointerId),
    DuplicateParticipant { pointer: PointerId, participant: K },
    UnknownParticipant { pointer: PointerId, participant: K },
}

/// Deterministic arena counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GestureArenaDiagnostics {
    pub opened: u64,
    pub resolved: u64,
    pub wins: u64,
    pub losses: u64,
    pub cancellations: u64,
    pub failures: u64,
}

#[derive(Clone, Debug)]
struct ArenaContest<K> {
    members: Vec<K>,
    open: bool,
    held: bool,
    pending_sweep: bool,
    eager_winner: Option<K>,
}

/// Per-pointer gesture arena. The first acceptor or last nonrejecting member wins.
#[derive(Clone, Debug)]
pub struct GestureArena<K> {
    contests: HashMap<PointerId, ArenaContest<K>>,
    diagnostics: GestureArenaDiagnostics,
}

impl<K> Default for GestureArena<K> {
    fn default() -> Self {
        Self {
            contests: HashMap::new(),
            diagnostics: GestureArenaDiagnostics::default(),
        }
    }
}

impl<K> GestureArena<K>
where
    K: Copy + Eq + Hash,
{
    pub fn new() -> Self {
        Self {
            contests: HashMap::new(),
            diagnostics: GestureArenaDiagnostics::default(),
        }
    }

    pub fn diagnostics(&self) -> GestureArenaDiagnostics {
        self.diagnostics
    }

    pub fn is_active(&self, pointer: PointerId) -> bool {
        self.contests.contains_key(&pointer)
    }

    /// Adds a participant while the pointer's arena is open.
    pub fn add(&mut self, pointer: PointerId, participant: K) -> Result<(), GestureArenaError<K>> {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.contests.entry(pointer) {
            entry.insert(ArenaContest {
                members: Vec::new(),
                open: true,
                held: false,
                pending_sweep: false,
                eager_winner: None,
            });
            self.diagnostics.opened += 1;
        }
        let contest = self.contests.get_mut(&pointer).expect("inserted above");
        if !contest.open {
            self.diagnostics.failures += 1;
            return Err(GestureArenaError::ArenaAlreadyClosed(pointer));
        }
        if contest.members.contains(&participant) {
            self.diagnostics.failures += 1;
            return Err(GestureArenaError::DuplicateParticipant {
                pointer,
                participant,
            });
        }
        contest.members.push(participant);
        Ok(())
    }

    /// Closes registration and resolves an eager winner or last remaining participant.
    pub fn close(
        &mut self,
        pointer: PointerId,
    ) -> Result<Vec<GestureArenaDecision<K>>, GestureArenaError<K>> {
        let contest = self.contest_mut(pointer)?;
        if !contest.open {
            self.diagnostics.failures += 1;
            return Err(GestureArenaError::ArenaAlreadyClosed(pointer));
        }
        contest.open = false;
        Ok(self.try_resolve(pointer))
    }

    pub fn accept(
        &mut self,
        pointer: PointerId,
        participant: K,
    ) -> Result<Vec<GestureArenaDecision<K>>, GestureArenaError<K>> {
        let contest = self.contest_mut(pointer)?;
        if !contest.members.contains(&participant) {
            self.diagnostics.failures += 1;
            return Err(GestureArenaError::UnknownParticipant {
                pointer,
                participant,
            });
        }
        if contest.open {
            contest.eager_winner.get_or_insert(participant);
            return Ok(Vec::new());
        }
        Ok(self.resolve_winner(pointer, participant, GestureArenaWinReason::Accepted))
    }

    pub fn reject(
        &mut self,
        pointer: PointerId,
        participant: K,
    ) -> Result<Vec<GestureArenaDecision<K>>, GestureArenaError<K>> {
        let contest = self.contest_mut(pointer)?;
        let Some(index) = contest
            .members
            .iter()
            .position(|member| *member == participant)
        else {
            self.diagnostics.failures += 1;
            return Err(GestureArenaError::UnknownParticipant {
                pointer,
                participant,
            });
        };
        contest.members.remove(index);
        if contest.eager_winner == Some(participant) {
            contest.eager_winner = None;
        }
        self.diagnostics.losses += 1;
        let mut decisions = vec![GestureArenaDecision::Lost {
            participant,
            reason: GestureArenaLossReason::SelfRejected,
        }];
        decisions.extend(self.try_resolve(pointer));
        Ok(decisions)
    }

    /// Forces the first remaining participant to win after pointer-up processing.
    pub fn sweep(
        &mut self,
        pointer: PointerId,
    ) -> Result<Vec<GestureArenaDecision<K>>, GestureArenaError<K>> {
        let contest = self.contest_mut(pointer)?;
        if contest.open {
            self.diagnostics.failures += 1;
            return Err(GestureArenaError::ArenaStillOpen(pointer));
        }
        if contest.held {
            contest.pending_sweep = true;
            return Ok(Vec::new());
        }
        let Some(winner) = contest.members.first().copied() else {
            self.contests.remove(&pointer);
            return Ok(Vec::new());
        };
        Ok(self.resolve_winner(pointer, winner, GestureArenaWinReason::Swept))
    }

    pub fn hold(&mut self, pointer: PointerId) -> Result<(), GestureArenaError<K>> {
        self.contest_mut(pointer)?.held = true;
        Ok(())
    }

    pub fn release(
        &mut self,
        pointer: PointerId,
    ) -> Result<Vec<GestureArenaDecision<K>>, GestureArenaError<K>> {
        let contest = self.contest_mut(pointer)?;
        contest.held = false;
        if contest.pending_sweep {
            contest.pending_sweep = false;
            self.sweep(pointer)
        } else {
            Ok(Vec::new())
        }
    }

    pub fn cancel(
        &mut self,
        pointer: PointerId,
        reason: GestureCancelReason,
    ) -> Result<Vec<GestureArenaDecision<K>>, GestureArenaError<K>> {
        let Some(contest) = self.contests.remove(&pointer) else {
            self.diagnostics.failures += 1;
            return Err(GestureArenaError::UnknownPointer(pointer));
        };
        self.diagnostics.resolved += 1;
        self.diagnostics.cancellations += 1;
        self.diagnostics.losses += contest.members.len() as u64;
        Ok(contest
            .members
            .into_iter()
            .map(|participant| GestureArenaDecision::Lost {
                participant,
                reason: GestureArenaLossReason::Cancelled(reason),
            })
            .collect())
    }

    fn contest_mut(
        &mut self,
        pointer: PointerId,
    ) -> Result<&mut ArenaContest<K>, GestureArenaError<K>> {
        let Some(contest) = self.contests.get_mut(&pointer) else {
            self.diagnostics.failures += 1;
            return Err(GestureArenaError::UnknownPointer(pointer));
        };
        Ok(contest)
    }

    fn try_resolve(&mut self, pointer: PointerId) -> Vec<GestureArenaDecision<K>> {
        let Some(contest) = self.contests.get(&pointer) else {
            return Vec::new();
        };
        if contest.open {
            return Vec::new();
        }
        if contest.members.is_empty() {
            self.contests.remove(&pointer);
            self.diagnostics.resolved += 1;
            return Vec::new();
        }
        if contest.members.len() == 1 {
            let winner = contest.members[0];
            return self.resolve_winner(pointer, winner, GestureArenaWinReason::LastRemaining);
        }
        if let Some(winner) = contest.eager_winner {
            return self.resolve_winner(pointer, winner, GestureArenaWinReason::Accepted);
        }
        Vec::new()
    }

    fn resolve_winner(
        &mut self,
        pointer: PointerId,
        winner: K,
        reason: GestureArenaWinReason,
    ) -> Vec<GestureArenaDecision<K>> {
        let Some(contest) = self.contests.remove(&pointer) else {
            return Vec::new();
        };
        let mut decisions = Vec::with_capacity(contest.members.len());
        for participant in contest.members {
            if participant != winner {
                decisions.push(GestureArenaDecision::Lost {
                    participant,
                    reason: GestureArenaLossReason::Winner(winner),
                });
            }
        }
        decisions.push(GestureArenaDecision::Won {
            participant: winner,
            reason,
        });
        self.diagnostics.resolved += 1;
        self.diagnostics.wins += 1;
        self.diagnostics.losses += decisions.len().saturating_sub(1) as u64;
        decisions
    }
}

#[derive(Clone, Copy, Debug)]
enum TapState {
    Idle,
    Tracking {
        pointer: PointerId,
        origin: PointF,
        last: PointF,
        arena_won: bool,
    },
    AwaitingArena {
        pointer: PointerId,
        position: PointF,
    },
    Dead,
}

/// Primary-pointer tap recognizer with caller-supplied logical slop.
#[derive(Clone, Debug)]
pub struct TapRecognizer {
    slop_squared: f32,
    enabled: bool,
    state: TapState,
    diagnostics: GestureRecognizerDiagnostics,
}

impl TapRecognizer {
    pub fn new(slop: f32, enabled: bool) -> Result<Self, GestureRecognizerError> {
        Ok(Self {
            slop_squared: checked_slop_squared(slop)?,
            enabled,
            state: TapState::Idle,
            diagnostics: GestureRecognizerDiagnostics::default(),
        })
    }

    pub fn state(&self) -> GestureRecognizerState {
        match self.state {
            TapState::Idle => GestureRecognizerState::Idle,
            TapState::Tracking {
                arena_won: true, ..
            } => GestureRecognizerState::Accepted,
            TapState::Tracking { .. } | TapState::AwaitingArena { .. } => {
                GestureRecognizerState::Possible
            }
            TapState::Dead => GestureRecognizerState::Dead,
        }
    }

    pub fn diagnostics(&self) -> GestureRecognizerDiagnostics {
        self.diagnostics
    }

    pub fn handle(
        &mut self,
        input: GestureInput,
    ) -> Result<GestureOutcome, GestureRecognizerError> {
        if let Some(position) = input_position(input) {
            self.validate_position(position)?;
        }
        match input {
            GestureInput::SetEnabled(enabled) => {
                if self.enabled == enabled {
                    return Ok(GestureOutcome::ignored());
                }
                self.enabled = enabled;
                if enabled {
                    Ok(GestureOutcome::ignored())
                } else {
                    Ok(self.cancel(GestureCancelReason::Disabled, false, true))
                }
            }
            GestureInput::ViewDeactivated => {
                Ok(self.cancel(GestureCancelReason::ViewDeactivated, false, true))
            }
            GestureInput::Unmount => Ok(self.cancel(GestureCancelReason::Unmounted, true, true)),
            GestureInput::PointerDown {
                pointer,
                button: PointerButton::PRIMARY,
                position,
            } if self.enabled && matches!(self.state, TapState::Idle) => {
                self.state = TapState::Tracking {
                    pointer,
                    origin: position,
                    last: position,
                    arena_won: false,
                };
                self.diagnostics.sequences_started += 1;
                Ok(GestureOutcome::transition(GestureTransition::Possible {
                    kind: GestureKind::Tap,
                    pointer,
                    position,
                }))
            }
            GestureInput::PointerMoved { pointer, position } => {
                let TapState::Tracking {
                    pointer: active,
                    origin,
                    arena_won,
                    ..
                } = self.state
                else {
                    return Ok(GestureOutcome::ignored());
                };
                if pointer != active {
                    return Ok(GestureOutcome::ignored());
                }
                if distance_squared(origin, position) > self.slop_squared {
                    return Ok(self.cancel(GestureCancelReason::SlopExceeded, false, true));
                }
                self.state = TapState::Tracking {
                    pointer,
                    origin,
                    last: position,
                    arena_won,
                };
                Ok(GestureOutcome::ignored())
            }
            GestureInput::PointerUp {
                pointer,
                button: PointerButton::PRIMARY,
                position,
            } => match self.state {
                TapState::Tracking {
                    pointer: active,
                    origin,
                    arena_won,
                    ..
                } if active == pointer => {
                    if distance_squared(origin, position) > self.slop_squared {
                        return Ok(self.cancel(GestureCancelReason::SlopExceeded, false, true));
                    }
                    if arena_won {
                        self.state = TapState::Idle;
                        self.diagnostics.recognized += 1;
                        Ok(GestureOutcome::transition(
                            GestureTransition::TapRecognized { pointer, position },
                        ))
                    } else {
                        self.state = TapState::AwaitingArena { pointer, position };
                        self.diagnostics.arena_claims += 1;
                        Ok(GestureOutcome {
                            arena: GestureArenaRequest::Accept(pointer),
                            ..GestureOutcome::ignored()
                        })
                    }
                }
                _ => Ok(GestureOutcome::ignored()),
            },
            GestureInput::ArenaWon { pointer } => match self.state {
                TapState::Tracking {
                    pointer: active,
                    origin,
                    last,
                    ..
                } if active == pointer => {
                    self.state = TapState::Tracking {
                        pointer,
                        origin,
                        last,
                        arena_won: true,
                    };
                    Ok(GestureOutcome::ignored())
                }
                TapState::AwaitingArena {
                    pointer: active,
                    position,
                } if active == pointer => {
                    self.state = TapState::Idle;
                    self.diagnostics.recognized += 1;
                    Ok(GestureOutcome::transition(
                        GestureTransition::TapRecognized { pointer, position },
                    ))
                }
                _ => Ok(GestureOutcome::ignored()),
            },
            GestureInput::ArenaLost { pointer } if self.active_pointer() == Some(pointer) => {
                Ok(self.cancel(GestureCancelReason::ArenaLost, false, false))
            }
            GestureInput::PointerCancelled { pointer }
                if self.active_pointer() == Some(pointer) =>
            {
                Ok(self.cancel(GestureCancelReason::PointerCancelled, false, true))
            }
            GestureInput::PointerCaptureLost { pointer }
                if self.active_pointer() == Some(pointer) =>
            {
                Ok(self.cancel(GestureCancelReason::CaptureLost, false, false))
            }
            _ => Ok(GestureOutcome::ignored()),
        }
    }

    fn active_pointer(&self) -> Option<PointerId> {
        match self.state {
            TapState::Tracking { pointer, .. } | TapState::AwaitingArena { pointer, .. } => {
                Some(pointer)
            }
            TapState::Idle | TapState::Dead => None,
        }
    }

    fn cancel(
        &mut self,
        reason: GestureCancelReason,
        terminal: bool,
        reject_arena: bool,
    ) -> GestureOutcome {
        let previous = self.state;
        self.state = if terminal || matches!(previous, TapState::Dead) {
            TapState::Dead
        } else {
            TapState::Idle
        };
        let Some(pointer) = (match previous {
            TapState::Tracking { pointer, .. } | TapState::AwaitingArena { pointer, .. } => {
                Some(pointer)
            }
            TapState::Idle | TapState::Dead => None,
        }) else {
            return GestureOutcome::ignored();
        };
        let arena_won = matches!(
            previous,
            TapState::Tracking {
                arena_won: true,
                ..
            }
        );
        self.diagnostics.cancelled += 1;
        if reject_arena && !arena_won {
            self.diagnostics.arena_rejections += 1;
        }
        GestureOutcome {
            transition: GestureTransition::Cancelled {
                kind: GestureKind::Tap,
                pointer,
                reason,
            },
            arena: if reject_arena && !arena_won {
                GestureArenaRequest::Reject(pointer)
            } else {
                GestureArenaRequest::None
            },
            deadline: GestureDeadlineRequest::None,
        }
    }

    fn validate_position(&mut self, position: PointF) -> Result<(), GestureRecognizerError> {
        if position.x.is_finite() && position.y.is_finite() {
            Ok(())
        } else {
            self.diagnostics.failures += 1;
            Err(GestureRecognizerError::NonFinitePosition(position))
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum LongPressState {
    Idle,
    Tracking {
        pointer: PointerId,
        origin: PointF,
        last: PointF,
        deadline: GestureDeadlineId,
        arena_won: bool,
    },
    AwaitingArena {
        pointer: PointerId,
        origin: PointF,
        last: PointF,
    },
    Recognized {
        pointer: PointerId,
        origin: PointF,
        last: PointF,
    },
    Dead,
}

/// Long-press recognizer driven by explicit host deadline callbacks.
#[derive(Clone, Debug)]
pub struct LongPressRecognizer {
    slop_squared: f32,
    delay: Duration,
    enabled: bool,
    next_deadline_generation: u64,
    state: LongPressState,
    diagnostics: GestureRecognizerDiagnostics,
}

impl LongPressRecognizer {
    pub fn new(slop: f32, delay: Duration, enabled: bool) -> Result<Self, GestureRecognizerError> {
        if delay.is_zero() {
            return Err(GestureRecognizerError::ZeroLongPressDelay);
        }
        Ok(Self {
            slop_squared: checked_slop_squared(slop)?,
            delay,
            enabled,
            next_deadline_generation: 1,
            state: LongPressState::Idle,
            diagnostics: GestureRecognizerDiagnostics::default(),
        })
    }

    pub fn state(&self) -> GestureRecognizerState {
        match self.state {
            LongPressState::Idle => GestureRecognizerState::Idle,
            LongPressState::Tracking { .. } | LongPressState::AwaitingArena { .. } => {
                GestureRecognizerState::Possible
            }
            LongPressState::Recognized { .. } => GestureRecognizerState::Accepted,
            LongPressState::Dead => GestureRecognizerState::Dead,
        }
    }

    pub fn diagnostics(&self) -> GestureRecognizerDiagnostics {
        self.diagnostics
    }

    pub fn handle(
        &mut self,
        input: GestureInput,
    ) -> Result<GestureOutcome, GestureRecognizerError> {
        if let Some(position) = input_position(input) {
            self.validate_position(position)?;
        }
        match input {
            GestureInput::SetEnabled(enabled) => {
                if self.enabled == enabled {
                    return Ok(GestureOutcome::ignored());
                }
                self.enabled = enabled;
                if enabled {
                    Ok(GestureOutcome::ignored())
                } else {
                    Ok(self.cancel(GestureCancelReason::Disabled, false, true))
                }
            }
            GestureInput::ViewDeactivated => {
                Ok(self.cancel(GestureCancelReason::ViewDeactivated, false, true))
            }
            GestureInput::Unmount => Ok(self.cancel(GestureCancelReason::Unmounted, true, true)),
            GestureInput::PointerDown {
                pointer,
                button: PointerButton::PRIMARY,
                position,
            } if self.enabled && matches!(self.state, LongPressState::Idle) => {
                let deadline = self.next_deadline(pointer);
                self.state = LongPressState::Tracking {
                    pointer,
                    origin: position,
                    last: position,
                    deadline,
                    arena_won: false,
                };
                self.diagnostics.sequences_started += 1;
                Ok(GestureOutcome {
                    transition: GestureTransition::Possible {
                        kind: GestureKind::LongPress,
                        pointer,
                        position,
                    },
                    arena: GestureArenaRequest::None,
                    deadline: GestureDeadlineRequest::Schedule {
                        id: deadline,
                        after: self.delay,
                    },
                })
            }
            GestureInput::PointerMoved { pointer, position } => {
                self.long_press_move(pointer, position)
            }
            GestureInput::DeadlineElapsed(deadline) => self.deadline_elapsed(deadline),
            GestureInput::ArenaWon { pointer } => self.long_press_arena_won(pointer),
            GestureInput::ArenaLost { pointer } if self.active_pointer() == Some(pointer) => {
                Ok(self.cancel(GestureCancelReason::ArenaLost, false, false))
            }
            GestureInput::PointerUp {
                pointer,
                button: PointerButton::PRIMARY,
                position,
            } if self.active_pointer() == Some(pointer) => self.long_press_up(pointer, position),
            GestureInput::PointerCancelled { pointer }
                if self.active_pointer() == Some(pointer) =>
            {
                Ok(self.cancel(GestureCancelReason::PointerCancelled, false, true))
            }
            GestureInput::PointerCaptureLost { pointer }
                if self.active_pointer() == Some(pointer) =>
            {
                Ok(self.cancel(GestureCancelReason::CaptureLost, false, false))
            }
            _ => Ok(GestureOutcome::ignored()),
        }
    }

    fn long_press_move(
        &mut self,
        pointer: PointerId,
        position: PointF,
    ) -> Result<GestureOutcome, GestureRecognizerError> {
        match self.state {
            LongPressState::Tracking {
                pointer: active,
                origin,
                deadline,
                arena_won,
                ..
            } if active == pointer => {
                if distance_squared(origin, position) > self.slop_squared {
                    Ok(self.cancel(GestureCancelReason::SlopExceeded, false, true))
                } else {
                    self.state = LongPressState::Tracking {
                        pointer,
                        origin,
                        last: position,
                        deadline,
                        arena_won,
                    };
                    Ok(GestureOutcome::ignored())
                }
            }
            LongPressState::AwaitingArena {
                pointer: active,
                origin,
                ..
            } if active == pointer => {
                if distance_squared(origin, position) > self.slop_squared {
                    Ok(self.cancel(GestureCancelReason::SlopExceeded, false, true))
                } else {
                    self.state = LongPressState::AwaitingArena {
                        pointer,
                        origin,
                        last: position,
                    };
                    Ok(GestureOutcome::ignored())
                }
            }
            LongPressState::Recognized {
                pointer: active,
                origin,
                last,
            } if active == pointer => {
                self.state = LongPressState::Recognized {
                    pointer,
                    origin,
                    last: position,
                };
                Ok(GestureOutcome::transition(
                    GestureTransition::LongPressUpdated {
                        pointer,
                        position,
                        delta: delta(last, position),
                        total: delta(origin, position),
                    },
                ))
            }
            _ => Ok(GestureOutcome::ignored()),
        }
    }

    fn deadline_elapsed(
        &mut self,
        deadline: GestureDeadlineId,
    ) -> Result<GestureOutcome, GestureRecognizerError> {
        let LongPressState::Tracking {
            pointer,
            origin,
            last,
            deadline: expected,
            arena_won,
        } = self.state
        else {
            self.diagnostics.stale_deadlines += 1;
            return Ok(GestureOutcome::ignored());
        };
        if deadline != expected {
            self.diagnostics.stale_deadlines += 1;
            return Ok(GestureOutcome::ignored());
        }
        if arena_won {
            self.state = LongPressState::Recognized {
                pointer,
                origin,
                last,
            };
            self.diagnostics.recognized += 1;
            Ok(GestureOutcome::transition(
                GestureTransition::LongPressStarted {
                    pointer,
                    origin,
                    position: last,
                },
            ))
        } else {
            self.state = LongPressState::AwaitingArena {
                pointer,
                origin,
                last,
            };
            self.diagnostics.arena_claims += 1;
            Ok(GestureOutcome {
                arena: GestureArenaRequest::Accept(pointer),
                ..GestureOutcome::ignored()
            })
        }
    }

    fn long_press_arena_won(
        &mut self,
        pointer: PointerId,
    ) -> Result<GestureOutcome, GestureRecognizerError> {
        match self.state {
            LongPressState::Tracking {
                pointer: active,
                origin,
                last,
                deadline,
                ..
            } if active == pointer => {
                self.state = LongPressState::Tracking {
                    pointer,
                    origin,
                    last,
                    deadline,
                    arena_won: true,
                };
                Ok(GestureOutcome::ignored())
            }
            LongPressState::AwaitingArena {
                pointer: active,
                origin,
                last,
            } if active == pointer => {
                self.state = LongPressState::Recognized {
                    pointer,
                    origin,
                    last,
                };
                self.diagnostics.recognized += 1;
                Ok(GestureOutcome::transition(
                    GestureTransition::LongPressStarted {
                        pointer,
                        origin,
                        position: last,
                    },
                ))
            }
            _ => Ok(GestureOutcome::ignored()),
        }
    }

    fn long_press_up(
        &mut self,
        pointer: PointerId,
        position: PointF,
    ) -> Result<GestureOutcome, GestureRecognizerError> {
        if let LongPressState::Recognized { origin, .. } = self.state {
            self.state = LongPressState::Idle;
            return Ok(GestureOutcome::transition(
                GestureTransition::LongPressEnded {
                    pointer,
                    position,
                    total: delta(origin, position),
                },
            ));
        }
        Ok(self.cancel(GestureCancelReason::ReleasedBeforeRecognition, false, true))
    }

    fn active_pointer(&self) -> Option<PointerId> {
        match self.state {
            LongPressState::Tracking { pointer, .. }
            | LongPressState::AwaitingArena { pointer, .. }
            | LongPressState::Recognized { pointer, .. } => Some(pointer),
            LongPressState::Idle | LongPressState::Dead => None,
        }
    }

    fn cancel(
        &mut self,
        reason: GestureCancelReason,
        terminal: bool,
        reject_arena: bool,
    ) -> GestureOutcome {
        let previous = self.state;
        self.state = if terminal || matches!(previous, LongPressState::Dead) {
            LongPressState::Dead
        } else {
            LongPressState::Idle
        };
        let (pointer, deadline, arena_won) = match previous {
            LongPressState::Tracking {
                pointer,
                deadline,
                arena_won,
                ..
            } => (pointer, Some(deadline), arena_won),
            LongPressState::AwaitingArena { pointer, .. } => (pointer, None, false),
            LongPressState::Recognized { pointer, .. } => (pointer, None, true),
            LongPressState::Idle | LongPressState::Dead => return GestureOutcome::ignored(),
        };
        self.diagnostics.cancelled += 1;
        if reject_arena && !arena_won {
            self.diagnostics.arena_rejections += 1;
        }
        GestureOutcome {
            transition: GestureTransition::Cancelled {
                kind: GestureKind::LongPress,
                pointer,
                reason,
            },
            arena: if reject_arena && !arena_won {
                GestureArenaRequest::Reject(pointer)
            } else {
                GestureArenaRequest::None
            },
            deadline: deadline.map_or(GestureDeadlineRequest::None, GestureDeadlineRequest::Cancel),
        }
    }

    fn next_deadline(&mut self, pointer: PointerId) -> GestureDeadlineId {
        let generation = self.next_deadline_generation;
        self.next_deadline_generation = self.next_deadline_generation.wrapping_add(1).max(1);
        GestureDeadlineId {
            pointer,
            generation,
        }
    }

    fn validate_position(&mut self, position: PointF) -> Result<(), GestureRecognizerError> {
        if position.x.is_finite() && position.y.is_finite() {
            Ok(())
        } else {
            self.diagnostics.failures += 1;
            Err(GestureRecognizerError::NonFinitePosition(position))
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum DragState {
    Idle,
    Tracking {
        pointer: PointerId,
        origin: PointF,
        last: PointF,
        arena_won: bool,
    },
    AwaitingArena {
        pointer: PointerId,
        origin: PointF,
        last: PointF,
    },
    Dragging {
        pointer: PointerId,
        origin: PointF,
        last: PointF,
    },
    Dead,
}

/// Primary-pointer drag recognizer with explicit axis and logical slop policy.
#[derive(Clone, Debug)]
pub struct DragRecognizer {
    axis: DragAxis,
    slop: f32,
    enabled: bool,
    state: DragState,
    diagnostics: GestureRecognizerDiagnostics,
}

impl DragRecognizer {
    pub fn new(axis: DragAxis, slop: f32, enabled: bool) -> Result<Self, GestureRecognizerError> {
        checked_slop_squared(slop)?;
        Ok(Self {
            axis,
            slop,
            enabled,
            state: DragState::Idle,
            diagnostics: GestureRecognizerDiagnostics::default(),
        })
    }

    pub fn state(&self) -> GestureRecognizerState {
        match self.state {
            DragState::Idle => GestureRecognizerState::Idle,
            DragState::Tracking { .. } | DragState::AwaitingArena { .. } => {
                GestureRecognizerState::Possible
            }
            DragState::Dragging { .. } => GestureRecognizerState::Accepted,
            DragState::Dead => GestureRecognizerState::Dead,
        }
    }

    pub fn diagnostics(&self) -> GestureRecognizerDiagnostics {
        self.diagnostics
    }

    pub fn handle(
        &mut self,
        input: GestureInput,
    ) -> Result<GestureOutcome, GestureRecognizerError> {
        if let Some(position) = input_position(input) {
            self.validate_position(position)?;
        }
        match input {
            GestureInput::SetEnabled(enabled) => {
                if self.enabled == enabled {
                    return Ok(GestureOutcome::ignored());
                }
                self.enabled = enabled;
                if enabled {
                    Ok(GestureOutcome::ignored())
                } else {
                    Ok(self.cancel(GestureCancelReason::Disabled, false, true))
                }
            }
            GestureInput::ViewDeactivated => {
                Ok(self.cancel(GestureCancelReason::ViewDeactivated, false, true))
            }
            GestureInput::Unmount => Ok(self.cancel(GestureCancelReason::Unmounted, true, true)),
            GestureInput::PointerDown {
                pointer,
                button: PointerButton::PRIMARY,
                position,
            } if self.enabled && matches!(self.state, DragState::Idle) => {
                self.state = DragState::Tracking {
                    pointer,
                    origin: position,
                    last: position,
                    arena_won: false,
                };
                self.diagnostics.sequences_started += 1;
                Ok(GestureOutcome::transition(GestureTransition::Possible {
                    kind: GestureKind::Drag,
                    pointer,
                    position,
                }))
            }
            GestureInput::PointerMoved { pointer, position } => self.drag_move(pointer, position),
            GestureInput::ArenaWon { pointer } => self.drag_arena_won(pointer),
            GestureInput::ArenaLost { pointer } if self.active_pointer() == Some(pointer) => {
                Ok(self.cancel(GestureCancelReason::ArenaLost, false, false))
            }
            GestureInput::PointerUp {
                pointer,
                button: PointerButton::PRIMARY,
                position,
            } if self.active_pointer() == Some(pointer) => self.drag_up(pointer, position),
            GestureInput::PointerCancelled { pointer }
                if self.active_pointer() == Some(pointer) =>
            {
                Ok(self.cancel(GestureCancelReason::PointerCancelled, false, true))
            }
            GestureInput::PointerCaptureLost { pointer }
                if self.active_pointer() == Some(pointer) =>
            {
                Ok(self.cancel(GestureCancelReason::CaptureLost, false, false))
            }
            _ => Ok(GestureOutcome::ignored()),
        }
    }

    fn drag_move(
        &mut self,
        pointer: PointerId,
        position: PointF,
    ) -> Result<GestureOutcome, GestureRecognizerError> {
        match self.state {
            DragState::Tracking {
                pointer: active,
                origin,
                arena_won,
                ..
            } if active == pointer => {
                if !self.exceeds_slop(origin, position) {
                    self.state = DragState::Tracking {
                        pointer,
                        origin,
                        last: position,
                        arena_won,
                    };
                    return Ok(GestureOutcome::ignored());
                }
                if arena_won {
                    self.state = DragState::Dragging {
                        pointer,
                        origin,
                        last: position,
                    };
                    self.diagnostics.recognized += 1;
                    Ok(GestureOutcome::transition(GestureTransition::DragStarted {
                        pointer,
                        origin,
                        position,
                        total: delta(origin, position),
                    }))
                } else {
                    self.state = DragState::AwaitingArena {
                        pointer,
                        origin,
                        last: position,
                    };
                    self.diagnostics.arena_claims += 1;
                    Ok(GestureOutcome {
                        arena: GestureArenaRequest::Accept(pointer),
                        ..GestureOutcome::ignored()
                    })
                }
            }
            DragState::AwaitingArena {
                pointer: active,
                origin,
                ..
            } if active == pointer => {
                self.state = DragState::AwaitingArena {
                    pointer,
                    origin,
                    last: position,
                };
                Ok(GestureOutcome::ignored())
            }
            DragState::Dragging {
                pointer: active,
                origin,
                last,
            } if active == pointer => {
                self.state = DragState::Dragging {
                    pointer,
                    origin,
                    last: position,
                };
                Ok(GestureOutcome::transition(GestureTransition::DragUpdated {
                    pointer,
                    position,
                    delta: delta(last, position),
                    total: delta(origin, position),
                }))
            }
            _ => Ok(GestureOutcome::ignored()),
        }
    }

    fn drag_arena_won(
        &mut self,
        pointer: PointerId,
    ) -> Result<GestureOutcome, GestureRecognizerError> {
        match self.state {
            DragState::Tracking {
                pointer: active,
                origin,
                last,
                ..
            } if active == pointer => {
                self.state = DragState::Tracking {
                    pointer,
                    origin,
                    last,
                    arena_won: true,
                };
                Ok(GestureOutcome::ignored())
            }
            DragState::AwaitingArena {
                pointer: active,
                origin,
                last,
            } if active == pointer => {
                self.state = DragState::Dragging {
                    pointer,
                    origin,
                    last,
                };
                self.diagnostics.recognized += 1;
                Ok(GestureOutcome::transition(GestureTransition::DragStarted {
                    pointer,
                    origin,
                    position: last,
                    total: delta(origin, last),
                }))
            }
            _ => Ok(GestureOutcome::ignored()),
        }
    }

    fn drag_up(
        &mut self,
        pointer: PointerId,
        position: PointF,
    ) -> Result<GestureOutcome, GestureRecognizerError> {
        if let DragState::Dragging { origin, .. } = self.state {
            self.state = DragState::Idle;
            return Ok(GestureOutcome::transition(GestureTransition::DragEnded {
                pointer,
                position,
                total: delta(origin, position),
            }));
        }
        Ok(self.cancel(GestureCancelReason::ReleasedBeforeRecognition, false, true))
    }

    fn active_pointer(&self) -> Option<PointerId> {
        match self.state {
            DragState::Tracking { pointer, .. }
            | DragState::AwaitingArena { pointer, .. }
            | DragState::Dragging { pointer, .. } => Some(pointer),
            DragState::Idle | DragState::Dead => None,
        }
    }

    fn cancel(
        &mut self,
        reason: GestureCancelReason,
        terminal: bool,
        reject_arena: bool,
    ) -> GestureOutcome {
        let previous = self.state;
        self.state = if terminal || matches!(previous, DragState::Dead) {
            DragState::Dead
        } else {
            DragState::Idle
        };
        let (pointer, arena_won) = match previous {
            DragState::Tracking {
                pointer, arena_won, ..
            } => (pointer, arena_won),
            DragState::AwaitingArena { pointer, .. } => (pointer, false),
            DragState::Dragging { pointer, .. } => (pointer, true),
            DragState::Idle | DragState::Dead => return GestureOutcome::ignored(),
        };
        self.diagnostics.cancelled += 1;
        if reject_arena && !arena_won {
            self.diagnostics.arena_rejections += 1;
        }
        GestureOutcome {
            transition: GestureTransition::Cancelled {
                kind: GestureKind::Drag,
                pointer,
                reason,
            },
            arena: if reject_arena && !arena_won {
                GestureArenaRequest::Reject(pointer)
            } else {
                GestureArenaRequest::None
            },
            deadline: GestureDeadlineRequest::None,
        }
    }

    fn exceeds_slop(&self, origin: PointF, position: PointF) -> bool {
        let movement = delta(origin, position);
        match self.axis {
            DragAxis::Horizontal => movement.x.abs() > self.slop,
            DragAxis::Vertical => movement.y.abs() > self.slop,
            DragAxis::Both => {
                movement.x * movement.x + movement.y * movement.y > self.slop * self.slop
            }
        }
    }

    fn validate_position(&mut self, position: PointF) -> Result<(), GestureRecognizerError> {
        if position.x.is_finite() && position.y.is_finite() {
            Ok(())
        } else {
            self.diagnostics.failures += 1;
            Err(GestureRecognizerError::NonFinitePosition(position))
        }
    }
}

fn checked_slop_squared(slop: f32) -> Result<f32, GestureRecognizerError> {
    if slop.is_finite() && slop >= 0.0 {
        Ok(slop * slop)
    } else {
        Err(GestureRecognizerError::InvalidSlop(slop))
    }
}

fn input_position(input: GestureInput) -> Option<PointF> {
    match input {
        GestureInput::PointerDown { position, .. }
        | GestureInput::PointerMoved { position, .. }
        | GestureInput::PointerUp { position, .. } => Some(position),
        _ => None,
    }
}

fn distance_squared(a: PointF, b: PointF) -> f32 {
    let movement = delta(a, b);
    movement.x * movement.x + movement.y * movement.y
}

fn delta(from: PointF, to: PointF) -> GestureDelta {
    GestureDelta {
        x: to.x - from.x,
        y: to.y - from.y,
    }
}

#[cfg(test)]
mod tests;

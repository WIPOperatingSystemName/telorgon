use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU64;

use crate::integrations::wayland::compositor::{ProtocolObjectId, WaylandBufferId, WaylandSurfaceId};

pub(crate) fn take_surface_commits_through<T>(
    commits: &mut BTreeMap<(WaylandSurfaceId, u64), Vec<T>>,
    surface: WaylandSurfaceId,
    through_revision: u64,
) -> Vec<(u64, Vec<T>)> {
    let keys = commits
        .range((surface, 0)..=(surface, through_revision))
        .map(|(key, _)| *key)
        .collect::<Vec<_>>();
    keys.into_iter()
        .filter_map(|key| commits.remove(&key).map(|objects| (key.1, objects)))
        .collect()
}

/// Completes feedback only on presentation. Callback-only wakes leave feedback pending until an
/// actual frame is displayed or supersedes it, including older frames still in flight at the wake.
pub(crate) fn take_surface_feedbacks_through<T>(
    commits: &mut BTreeMap<(WaylandSurfaceId, u64), Vec<T>>,
    surface: WaylandSurfaceId,
    through_revision: u64,
    visible: bool,
) -> (Vec<T>, Vec<T>) {
    if !visible {
        return (Vec::new(), Vec::new());
    }
    let mut presented = Vec::new();
    let mut discarded = Vec::new();
    for (revision, feedbacks) in take_surface_commits_through(commits, surface, through_revision) {
        if visible && revision == through_revision {
            presented.extend(feedbacks);
        } else {
            discarded.extend(feedbacks);
        }
    }
    (presented, discarded)
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferUseId(NonZeroU64);

impl BufferUseId {
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceFrameCallback {
    pub object: ProtocolObjectId,
    pub surface: WaylandSurfaceId,
    pub commit_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferRelease {
    pub use_id: BufferUseId,
    pub buffer: WaylandBufferId,
    pub release_fence_token: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct BufferUse {
    buffer: WaylandBufferId,
    surface: WaylandSurfaceId,
    commit_revision: u64,
    pending_outputs: usize,
    explicit_release: bool,
}

#[derive(Debug, Default)]
pub struct BufferUseTracker {
    next_id: u64,
    uses: BTreeMap<BufferUseId, BufferUse>,
    callbacks: Vec<SurfaceFrameCallback>,
}

impl BufferUseTracker {
    pub fn begin_use(
        &mut self,
        buffer: WaylandBufferId,
        surface: WaylandSurfaceId,
        commit_revision: u64,
        output_count: usize,
        explicit_release: bool,
    ) -> Result<BufferUseId, BufferUseError> {
        if commit_revision == 0 || output_count == 0 {
            return Err(BufferUseError::InvalidUse);
        }
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(BufferUseError::IdentityExhausted)?;
        let id =
            BufferUseId(NonZeroU64::new(self.next_id).ok_or(BufferUseError::IdentityExhausted)?);
        self.uses.insert(
            id,
            BufferUse {
                buffer,
                surface,
                commit_revision,
                pending_outputs: output_count,
                explicit_release,
            },
        );
        Ok(id)
    }

    pub fn add_frame_callback(&mut self, callback: SurfaceFrameCallback) {
        self.callbacks.push(callback);
    }

    pub fn presented(
        &mut self,
        use_id: BufferUseId,
        release_fence_token: Option<u64>,
    ) -> Result<Option<BufferRelease>, BufferUseError> {
        let use_state = self
            .uses
            .get_mut(&use_id)
            .ok_or(BufferUseError::UnknownUse)?;
        if use_state.pending_outputs == 0 {
            return Err(BufferUseError::AlreadyCompleted);
        }
        use_state.pending_outputs -= 1;
        if use_state.pending_outputs != 0 {
            return Ok(None);
        }
        let use_state = self.uses.remove(&use_id).expect("use remains present");
        if use_state.explicit_release && release_fence_token.is_none() {
            return Err(BufferUseError::MissingReleaseFence);
        }
        Ok(Some(BufferRelease {
            use_id,
            buffer: use_state.buffer,
            release_fence_token,
        }))
    }

    pub fn take_callbacks_for(
        &mut self,
        surface: WaylandSurfaceId,
        through_revision: u64,
    ) -> Vec<SurfaceFrameCallback> {
        let mut ready = Vec::new();
        self.callbacks.retain(|callback| {
            if callback.surface == surface && callback.commit_revision <= through_revision {
                ready.push(*callback);
                false
            } else {
                true
            }
        });
        ready
    }

    pub fn cancel_surface(&mut self, surface: WaylandSurfaceId) -> usize {
        let before = self.uses.len() + self.callbacks.len();
        self.uses
            .retain(|_, use_state| use_state.surface != surface);
        self.callbacks
            .retain(|callback| callback.surface != surface);
        before - self.uses.len() - self.callbacks.len()
    }

    pub fn use_revision(&self, use_id: BufferUseId) -> Option<u64> {
        self.uses
            .get(&use_id)
            .map(|use_state| use_state.commit_revision)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferUseError {
    InvalidUse,
    IdentityExhausted,
    UnknownUse,
    AlreadyCompleted,
    MissingReleaseFence,
}

impl fmt::Display for BufferUseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Wayland buffer-use tracking failed: {self:?}")
    }
}

impl std::error::Error for BufferUseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_only_wake_preserves_feedback_until_actual_presentation() {
        let surface = WaylandSurfaceId::from_raw(2).unwrap();
        let other = WaylandSurfaceId::from_raw(3).unwrap();
        let mut commits = BTreeMap::from([
            ((surface, 4), vec![40]),
            ((surface, 6), vec![60]),
            ((surface, 8), vec![80]),
            ((other, 4), vec![400]),
        ]);
        assert_eq!(
            take_surface_feedbacks_through(&mut commits, surface, 6, false),
            (vec![], vec![])
        );
        assert_eq!(commits.get(&(surface, 8)), Some(&vec![80]));
        assert_eq!(commits.get(&(other, 4)), Some(&vec![400]));
        assert_eq!(
            take_surface_feedbacks_through(&mut commits, surface, 8, true),
            (vec![80], vec![40, 60])
        );
    }

    #[test]
    fn visible_feedback_reports_only_the_displayed_revision() {
        let surface = WaylandSurfaceId::from_raw(2).unwrap();
        let mut commits = BTreeMap::from([((surface, 4), vec![40]), ((surface, 6), vec![60])]);
        assert_eq!(
            take_surface_feedbacks_through(&mut commits, surface, 6, true),
            (vec![60], vec![40])
        );
    }

    #[test]
    fn a_buffer_is_released_only_after_every_output_finishes() {
        let mut tracker = BufferUseTracker::default();
        let id = tracker
            .begin_use(
                WaylandBufferId::from_raw(1).unwrap(),
                WaylandSurfaceId::from_raw(2).unwrap(),
                3,
                2,
                false,
            )
            .unwrap();
        assert_eq!(tracker.presented(id, None).unwrap(), None);
        assert!(tracker.presented(id, None).unwrap().is_some());
    }

    #[test]
    fn committed_objects_are_drained_through_the_presented_revision_only() {
        let surface = WaylandSurfaceId::from_raw(2).unwrap();
        let other = WaylandSurfaceId::from_raw(3).unwrap();
        let mut commits = BTreeMap::from([
            ((surface, 4), vec![40, 41]),
            ((surface, 6), vec![60]),
            ((surface, 8), vec![80]),
            ((other, 4), vec![400]),
        ]);

        assert_eq!(
            take_surface_commits_through(&mut commits, surface, 6),
            vec![(4, vec![40, 41]), (6, vec![60])]
        );
        assert_eq!(commits.get(&(surface, 8)), Some(&vec![80]));
        assert_eq!(commits.get(&(other, 4)), Some(&vec![400]));
    }
}

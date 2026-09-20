//! Selection is presentation state, never authority. All identities retain the host mapping epoch.
use super::{CaptureSource, CaptureUiSnapshot};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Screens,
    Windows,
}
impl Tab {
    pub fn contains(self, source: CaptureSource) -> bool {
        matches!(
            (self, source),
            (Self::Screens, CaptureSource::Output(_)) | (Self::Windows, CaptureSource::Window(_))
        )
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Selection {
    pub request: u64,
    pub tab: Tab,
    pub selected: Option<(CaptureSource, u64)>,
    pub page: usize,
    pub submitted: bool,
}
impl Selection {
    pub fn current(&self, snapshot: &CaptureUiSnapshot, page_size: usize) -> Self {
        let request = snapshot.pending.as_ref().map_or(0, |p| p.0);
        let mut next = if self.request == request {
            self.clone()
        } else {
            Self {
                request,
                ..Self::default()
            }
        };
        if !snapshot.sources.iter().any(|s| next.tab.contains(s.0)) {
            next.tab = if snapshot
                .sources
                .iter()
                .any(|s| matches!(s.0, CaptureSource::Output(_)))
            {
                Tab::Screens
            } else {
                Tab::Windows
            };
        }
        if next
            .selected
            .is_some_and(|selected| !snapshot.sources.iter().any(|s| (s.0, s.1) == selected))
        {
            next.selected = None;
            next.submitted = false;
        }
        let count = snapshot
            .sources
            .iter()
            .filter(|s| next.tab.contains(s.0))
            .count();
        next.page = next.page.min(count.saturating_sub(1) / page_size.max(1));
        next
    }
    pub fn switch(&mut self, tab: Tab) {
        self.tab = tab;
        self.page = 0;
        self.selected = None;
        self.submitted = false;
    }
}

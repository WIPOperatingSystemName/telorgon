use super::*;

/// Active effects take precedence; frontmost idle windows retain their exit snapshots first.
pub(super) fn candidates(
    states: BTreeMap<u32, WindowState>,
    windows: &BTreeMap<u32, WindowVisual>,
    first_presentations: &BTreeSet<u32>,
    groups: &BTreeMap<u32, Vec<(usize, ShellPlacement)>>,
) -> Vec<(u32, WindowState, bool)> {
    let mut candidates = states
        .into_iter()
        .filter(|(_, s)| s.style.enabled())
        .map(|(id, state)| {
            let active = (first_presentations.contains(&id)
                && !state.minimized
                && state.style.open_transition().duration_ms > 0)
                || windows.get(&id).is_some_and(|v| {
                    v.state.placement_changed(state)
                        || v.state.minimized != state.minimized
                        || v.state.veiled != state.veiled
                        || v.geometry.pending()
                        || v.opening.from != v.opening.to
                        || v.visibility.from != v.visibility.to
                        || v.content.from != v.content.to
                });
            (id, state, active)
        })
        .collect::<Vec<_>>();
    // Admission must follow presentation activity, not native surface-ID allocation order.
    candidates.sort_by_key(|(id, _, active)| {
        let order = groups
            .get(id)
            .and_then(|group| group.last())
            .map_or(0, |(index, _)| *index);
        (!*active, std::cmp::Reverse(order), *id)
    });
    candidates
}

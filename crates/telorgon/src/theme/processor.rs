//! Per-view style resolution, interruption-safe retargeting, and arena-driven sampling.

use std::collections::HashMap;

use crate::foundation::{ColorRgba8, MonotonicInstant, PointF, Transform2D};
use crate::ui::{
    Background, Border, BorderSide, CornerRadii, InteractionFlags, MountedUi, Outline, Shadow, ShadowList,
    StylePropertyPatch, StyleSlotId, ThemeScopeId, UiNodeId as NodeId,
};

use crate::theme::{
    MotionPreference, ThemeRuntime, ThemeRuntimeDiagnostics, ThemeUpdate, TransitionSpec,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TrackKey {
    scope: ThemeScopeId,
    state_root: NodeId,
    slot: StyleSlotId,
    node: NodeId,
}

#[derive(Clone, Copy, Debug)]
struct StyleTrack {
    from: StylePropertyPatch,
    target: StylePropertyPatch,
    start: MonotonicInstant,
    spec: TransitionSpec,
    suspended_at: Option<MonotonicInstant>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StyleProcessor {
    tracks: HashMap<TrackKey, StyleTrack>,
    baselines: HashMap<TrackKey, StylePropertyPatch>,
    binding_scratch: Vec<usize>,
    pub diagnostics: ThemeRuntimeDiagnostics,
}

impl StyleProcessor {
    pub fn update(
        &mut self,
        runtime: &ThemeRuntime,
        ui: &mut MountedUi,
        now: MonotonicInstant,
        preference: MotionPreference,
    ) -> ThemeUpdate {
        self.tracks
            .retain(|key, _| ui.nodes.contains(key.state_root) && ui.nodes.contains(key.node));
        self.baselines
            .retain(|key, _| ui.nodes.contains(key.state_root) && ui.nodes.contains(key.node));
        self.binding_scratch.clear();
        ui.swap_dirty_style_bindings(&mut self.binding_scratch);
        let mut bindings = ui.take_style_bindings_for_processing();
        for index in &self.binding_scratch {
            let Some(binding) = bindings.get_mut(*index) else { continue; };
            if std::mem::take(&mut binding.reset_style_motion) {
                // Reconciliation supplied a new authored root style. Do not overwrite it
                // with an old animation sample, especially after an effect is removed.
                self.tracks.retain(|key, _| key.node != binding.state_root);
                self.baselines.retain(|key, _| key.node != binding.state_root);
            }
            for slot in std::mem::take(&mut binding.reset_style_slots) {
                if let Some(target) = binding.slots.iter().find(|target| target.slot == slot) {
                    self.tracks.retain(|key, _| key.node != target.node || key.slot != slot);
                    self.baselines.retain(|key, _| key.node != target.node || key.slot != slot);
                }
            }
        }
        let mut changed = {
            #[cfg(feature = "instrumentation")]
            let _span = crate::runtime::instrumentation::span!("theme.motion");
            self.sample_tracks(ui, now, preference)
        };
        if !runtime.pending_changed_styles.is_empty() {
            for (index, binding) in bindings.iter().enumerate() {
                if (binding.local_style.is_none() || binding.local_style_overlay)
                    && runtime
                        .pending_changed_styles
                        .contains(&binding.component_style)
                    && !self.binding_scratch.contains(&index)
                {
                    self.binding_scratch.push(index);
                }
            }
        }
        while let Some(index) = self.binding_scratch.pop() {
            let Some(binding) = bindings.get_mut(index) else {
                continue;
            };
            if !ui.nodes.contains(binding.state_root) {
                self.diagnostics.stale_controls_rejected += 1;
                continue;
            }
            let interaction = ui
                .interactions
                .get(binding.state_root)
                .copied()
                .unwrap_or_default();
            let inherited = binding.inherited_state_root
                .and_then(|node| ui.interactions.get(node));
            let inherited_revision = inherited.map_or(0, |state| state.revision);
            let flags = InteractionFlags::from_bits(interaction.flags.bits()
                | inherited.map_or(0, |state| state.flags.bits() & !InteractionFlags::ROUTER_OWNED.bits()));
            let mut merged_style;
            let (compiled_style, style_revision) =
                if let Some(style) = binding.local_style.as_deref() {
                    if binding.local_style_overlay {
                        if let Some((base, revision)) = runtime.resolve_binding_style(binding.scope, binding.component_style) {
                            merged_style = base.clone();
                            merged_style.overlay_effects(style);
                            (&merged_style, revision)
                        } else { (style, 1) }
                    } else { (style, 1) }
                } else {
                    let Some(resolved) =
                        runtime.resolve_binding_style(binding.scope, binding.component_style)
                    else {
                        self.diagnostics.stale_scopes_rejected += 1;
                        continue;
                    };
                    resolved
                };
            if binding.theme_revision == style_revision
                && binding.interaction_revision == interaction.revision
                && binding.inherited_interaction_revision == inherited_revision
            {
                self.diagnostics.bindings_skipped += 1;
                continue;
            }
            self.diagnostics.bindings_evaluated += 1;
            for slot in &binding.slots {
                let Some(resolved) =
                    compiled_style.resolve_slot(&binding.variants, flags, slot.slot)
                else {
                    continue;
                };
                let key = TrackKey {
                    scope: binding.scope,
                    state_root: binding.state_root,
                    slot: slot.slot,
                    node: slot.node,
                };
                let baseline = *self
                    .baselines
                    .entry(key)
                    .or_insert_with(|| snapshot(ui, slot.node));
                let mut ownership = compiled_style
                    .controlled_slots
                    .get(&slot.slot)
                    .copied()
                    .unwrap_or_default();
                if compiled_style.controlled_font_families.contains(&slot.slot) {
                    ownership.text_family = baseline.text_family;
                }
                if let Some((_, local)) = binding
                    .local_overrides
                    .iter()
                    .find(|(local_slot, _)| *local_slot == slot.slot)
                {
                    ownership.overlay(*local);
                }
                if ownership == StylePropertyPatch::default() {
                    continue;
                }
                let mut target = select_owned_properties(baseline, ownership);
                target.overlay(resolved.patch);
                if let Some(family) = resolved.font_family {
                    target.text_family = Some(ui.intern(family));
                }
                if let Some((_, local)) = binding
                    .local_overrides
                    .iter()
                    .find(|(local_slot, _)| *local_slot == slot.slot)
                {
                    target.overlay(*local);
                }
                materialize_partial_properties(&mut target);
                enforce_invariants(&mut target);
                changed |= self.retarget(ui, key, target, resolved.transition, now, preference);
            }
            binding.theme_revision = style_revision;
            binding.interaction_revision = interaction.revision;
            binding.inherited_interaction_revision = inherited_revision;
        }
        ui.restore_style_bindings_after_processing(bindings);
        let active = self
            .tracks
            .iter()
            .filter(|(key, _)| {
                ui.interactions
                    .get(key.state_root)
                    .is_none_or(|interaction| interaction.visible)
            })
            .count();
        self.diagnostics.active_animations = active as u64;
        ThemeUpdate {
            changed,
            active_animations: active > 0,
            diagnostics: self.diagnostics,
        }
    }

    fn sample_tracks(
        &mut self,
        ui: &mut MountedUi,
        now: MonotonicInstant,
        preference: MotionPreference,
    ) -> bool {
        let mut changed = false;
        self.tracks.retain(|key, track| {
            let visible = ui
                .interactions
                .get(key.state_root)
                .is_none_or(|interaction| interaction.visible);
            if !visible {
                track.suspended_at.get_or_insert(now);
                return true;
            }
            if let Some(suspended) = track.suspended_at.take() {
                let pause = now.as_nanos().saturating_sub(suspended.as_nanos());
                track.start =
                    MonotonicInstant::from_nanos(track.start.as_nanos().saturating_add(pause));
            }
            let (sample, complete) = sample_track(*track, now, preference);
            changed |= ui.apply_style_patch(key.node, sample);
            !complete
        });
        changed
    }

    fn retarget(
        &mut self,
        ui: &mut MountedUi,
        key: TrackKey,
        target: StylePropertyPatch,
        mut spec: TransitionSpec,
        now: MonotonicInstant,
        preference: MotionPreference,
    ) -> bool {
        // A previous track may own fewer properties than the new target (for example when
        // a local control style replaces a catalog style). Preserve mounted values for newly
        // animated properties instead of interpolating missing values from their defaults.
        let mut current = select_owned_properties(snapshot(ui, key.node), target);
        if let Some(track) = self.tracks.get(&key) {
            current.overlay(select_owned_properties(
                sample_track(*track, now, preference).0,
                target,
            ));
        }
        if preference == MotionPreference::Reduced && spec.repeat {
            spec.duration_ms = 0;
            spec.repeat = false;
        }
        // Value changes and unrelated state revisions may resolve the same appearance.
        // Keep its original deadline instead of restarting the animation on every drag update.
        if self.tracks.get(&key).is_some_and(|track| track.target == target && track.spec == spec) {
            return ui.apply_style_patch(key.node, snapped_properties(target));
        }
        let mut changed = ui.apply_style_patch(key.node, snapped_properties(target));
        if spec.duration_ms == 0 || !has_animatable_difference(current, target, preference) {
            self.tracks.remove(&key);
            changed |= ui.apply_style_patch(key.node, target);
            return changed;
        }
        if self.tracks.contains_key(&key) {
            self.diagnostics.retargets += 1;
        }
        self.tracks.insert(
            key,
            StyleTrack {
                from: current,
                target,
                start: now,
                spec,
                suspended_at: None,
            },
        );
        changed
    }
}

fn snapshot(ui: &MountedUi, node: NodeId) -> StylePropertyPatch {
    let style = ui.box_styles.get(node).copied().unwrap_or_default();
    let mut patch = StylePropertyPatch {
        sizing: Some(style.sizing),
        width: Some(style.width),
        height: Some(style.height),
        min_size: Some(style.min_size),
        max_size: Some(style.max_size),
        margin: Some(style.margin),
        padding: Some(style.padding),
        background: Some(style.decoration.background),
        border: Some(style.decoration.border),
        outline: Some(style.decoration.outline),
        corner_radii: Some(style.decoration.corner_radii),
        shadows: Some(style.decoration.shadows),
        overflow: Some(style.overflow),
        opacity: Some(style.opacity),
        transform: Some(style.transform),
        ..StylePropertyPatch::default()
    };
    if let Some(text) = ui.texts.get(node) {
        patch.text_color = Some(text.style.color);
        patch.text_size = Some(text.style.size);
        patch.text_line_height = Some(text.style.line_height);
        patch.text_family = Some(text.style.family);
        patch.text_weight = Some(text.style.weight);
    }
    if let Some(image) = ui.images.get(node) {
        patch.image_tint = Some(image.tint);
    }
    patch
}

fn select_owned_properties(
    source: StylePropertyPatch,
    ownership: StylePropertyPatch,
) -> StylePropertyPatch {
    let mut selected = StylePropertyPatch::default();
    macro_rules! select {
        ($($field:ident),+ $(,)?) => {$ (
            if ownership.$field.is_some() {
                selected.$field = source.$field;
            }
        )+ };
    }
    select!(
        sizing,
        width,
        height,
        min_size,
        max_size,
        margin,
        padding,
        background,
        shadows,
        overflow,
        opacity,
        text_color,
        text_size,
        text_line_height,
        text_family,
        text_weight,
        image_tint,
    );
    if ownership.border.is_some()
        || ownership.border_width.is_some()
        || ownership.border_color.is_some()
    {
        selected.border = source.border;
    }
    if ownership.outline.is_some()
        || ownership.outline_width.is_some()
        || ownership.outline_offset.is_some()
        || ownership.outline_color.is_some()
    {
        selected.outline = source.outline;
    }
    if ownership.corner_radii.is_some() || ownership.radius.is_some() {
        selected.corner_radii = source.corner_radii;
    }
    if ownership.transform.is_some()
        || ownership.translation_x.is_some()
        || ownership.translation_y.is_some()
        || ownership.scale_x.is_some()
        || ownership.scale_y.is_some()
        || ownership.rotation.is_some()
        || ownership.origin_x.is_some()
        || ownership.origin_y.is_some()
    {
        selected.transform = source.transform;
    }
    selected
}

fn enforce_invariants(patch: &mut StylePropertyPatch) {
    if let Some(opacity) = &mut patch.opacity {
        *opacity = opacity.clamp(0.0, 1.0);
    }
    if let Some(width) = &mut patch.border_width {
        *width = width.max(0.0);
    }
    if let Some(width) = &mut patch.outline_width {
        *width = width.max(0.0);
    }
    if let Some(radius) = &mut patch.radius {
        *radius = radius.max(0.0);
    }
}

fn materialize_partial_properties(patch: &mut StylePropertyPatch) {
    if patch.border.is_some() || patch.border_width.is_some() || patch.border_color.is_some() {
        let mut border = patch.border.unwrap_or_default();
        if let Some(width) = patch.border_width.take() {
            border.top.width = width;
            border.right.width = width;
            border.bottom.width = width;
            border.left.width = width;
        }
        if let Some(color) = patch.border_color.take() {
            border.top.color = color;
            border.right.color = color;
            border.bottom.color = color;
            border.left.color = color;
        }
        patch.border = Some(border);
    }

    if patch.outline.is_some()
        || patch.outline_width.is_some()
        || patch.outline_offset.is_some()
        || patch.outline_color.is_some()
    {
        let mut outline = patch.outline.unwrap_or_default();
        if let Some(width) = patch.outline_width.take() {
            outline.width = width;
        }
        if let Some(offset) = patch.outline_offset.take() {
            outline.offset = offset;
        }
        if let Some(color) = patch.outline_color.take() {
            outline.color = color;
        }
        patch.outline = Some(outline);
    }

    if let Some(radius) = patch.radius.take() {
        patch.corner_radii = Some(CornerRadii::all(radius));
    }
    if patch.transform.is_some()
        || patch.translation_x.is_some()
        || patch.translation_y.is_some()
        || patch.scale_x.is_some()
        || patch.scale_y.is_some()
        || patch.rotation.is_some()
        || patch.origin_x.is_some()
        || patch.origin_y.is_some()
    {
        let mut transform = patch.transform.unwrap_or_default();
        if let Some(value) = patch.translation_x.take() {
            transform.translation.x = value;
        }
        if let Some(value) = patch.translation_y.take() {
            transform.translation.y = value;
        }
        if let Some(value) = patch.scale_x.take() {
            transform.scale.x = value;
        }
        if let Some(value) = patch.scale_y.take() {
            transform.scale.y = value;
        }
        if let Some(value) = patch.rotation.take() {
            transform.rotation = value;
        }
        if let Some(value) = patch.origin_x.take() {
            transform.origin.x = value;
        }
        if let Some(value) = patch.origin_y.take() {
            transform.origin.y = value;
        }
        patch.transform = Some(transform);
    }
}

fn snapped_properties(mut patch: StylePropertyPatch) -> StylePropertyPatch {
    patch.background = None;
    patch.border = None;
    patch.border_width = None;
    patch.border_color = None;
    patch.outline = None;
    patch.outline_width = None;
    patch.outline_offset = None;
    patch.outline_color = None;
    patch.corner_radii = None;
    patch.radius = None;
    patch.shadows = None;
    patch.opacity = None;
    patch.transform = None;
    patch.translation_x = None;
    patch.translation_y = None;
    patch.scale_x = None;
    patch.scale_y = None;
    patch.rotation = None;
    patch.origin_x = None;
    patch.origin_y = None;
    patch.text_color = None;
    patch.image_tint = None;
    patch
}

fn has_animatable_difference(
    from: StylePropertyPatch,
    target: StylePropertyPatch,
    preference: MotionPreference,
) -> bool {
    from.background != target.background
        || from.border != target.border
        || from.border_width != target.border_width
        || from.border_color != target.border_color
        || from.outline != target.outline
        || from.outline_width != target.outline_width
        || from.outline_offset != target.outline_offset
        || from.outline_color != target.outline_color
        || from.corner_radii != target.corner_radii
        || from.radius != target.radius
        || from.shadows != target.shadows
        || from.opacity != target.opacity
        || from.text_color != target.text_color
        || from.image_tint != target.image_tint
        || (preference == MotionPreference::Full
            && (from.transform != target.transform
                || from.translation_x != target.translation_x
                || from.translation_y != target.translation_y
                || from.scale_x != target.scale_x
                || from.scale_y != target.scale_y
                || from.rotation != target.rotation
                || from.origin_x != target.origin_x
                || from.origin_y != target.origin_y))
}

fn sample_track(
    track: StyleTrack,
    now: MonotonicInstant,
    preference: MotionPreference,
) -> (StylePropertyPatch, bool) {
    let elapsed_ns = now.as_nanos().saturating_sub(track.start.as_nanos());
    let duration_ns = u64::from(track.spec.duration_ms)
        .saturating_mul(1_000_000)
        .max(1);
    let raw = elapsed_ns as f32 / duration_ns as f32;
    let cycle = if track.spec.repeat {
        raw.fract()
    } else {
        raw.min(1.0)
    };
    let paint_t = track.spec.easing.sample(cycle);
    let opacity_duration = duration_ns.min(100_000_000);
    let opacity_raw = if preference == MotionPreference::Reduced {
        elapsed_ns as f32 / opacity_duration as f32
    } else {
        cycle
    };
    let opacity_t = track.spec.easing.sample(opacity_raw.min(1.0));
    let spatial_t = if preference == MotionPreference::Reduced {
        1.0
    } else {
        paint_t
    };
    (
        interpolate_patch(track.from, track.target, paint_t, opacity_t, spatial_t),
        !track.spec.repeat && raw >= 1.0,
    )
}

fn interpolate_patch(
    from: StylePropertyPatch,
    target: StylePropertyPatch,
    paint_t: f32,
    opacity_t: f32,
    spatial_t: f32,
) -> StylePropertyPatch {
    let mut result = target;
    if target.background.is_some() {
        result.background = Some(interpolate_background(
            from.background.unwrap_or_default(),
            target.background.unwrap_or_default(),
            paint_t,
        ));
    }
    if target.border.is_some() {
        result.border = Some(interpolate_border(
            from.border.unwrap_or_default(),
            target.border.unwrap_or_default(),
            paint_t,
        ));
    }
    if target.outline.is_some() {
        result.outline = Some(interpolate_outline(
            from.outline.unwrap_or_default(),
            target.outline.unwrap_or_default(),
            paint_t,
        ));
    }
    if target.corner_radii.is_some() {
        result.corner_radii = Some(interpolate_radii(
            from.corner_radii.unwrap_or_default(),
            target.corner_radii.unwrap_or_default(),
            paint_t,
        ));
    }
    if target.shadows.is_some() {
        result.shadows = Some(interpolate_shadows(
            from.shadows.unwrap_or_default(),
            target.shadows.unwrap_or_default(),
            paint_t,
        ));
    }
    if target.opacity.is_some() {
        result.opacity = Some(lerp(
            from.opacity.unwrap_or(1.0),
            target.opacity.unwrap_or(1.0),
            opacity_t,
        ));
    }
    if target.transform.is_some() {
        result.transform = Some(interpolate_transform(
            from.transform.unwrap_or_default(),
            target.transform.unwrap_or_default(),
            spatial_t,
        ));
    }
    if let (Some(from), Some(target)) = (from.text_color, target.text_color) {
        result.text_color = Some(interpolate_color(from, target, paint_t));
    }
    if let Some(target_tint) = target.image_tint {
        result.image_tint = Some(match (from.image_tint.flatten(), target_tint) {
            (Some(from), Some(target)) => Some(interpolate_color(from, target, paint_t)),
            (_, target) => target,
        });
    }
    // Canonical full fields above supersede source-level partial fields during sampling.
    result.border_width = None;
    result.border_color = None;
    result.outline_width = None;
    result.outline_offset = None;
    result.outline_color = None;
    result.radius = None;
    result.translation_x = None;
    result.translation_y = None;
    result.scale_x = None;
    result.scale_y = None;
    result.rotation = None;
    result.origin_x = None;
    result.origin_y = None;
    result
}

fn interpolate_background(from: Background, target: Background, t: f32) -> Background {
    let color = |background| match background {
        Background::None => ColorRgba8::default(),
        Background::Color(color) => color,
    };
    if t >= 1.0 && target == Background::None {
        Background::None
    } else {
        Background::Color(interpolate_color(color(from), color(target), t))
    }
}

fn interpolate_border(from: Border, target: Border, t: f32) -> Border {
    let side = |from: BorderSide, target: BorderSide| BorderSide {
        width: lerp(from.width, target.width, t),
        color: interpolate_color(from.color, target.color, t),
    };
    Border {
        top: side(from.top, target.top),
        right: side(from.right, target.right),
        bottom: side(from.bottom, target.bottom),
        left: side(from.left, target.left),
    }
}

fn interpolate_outline(from: Outline, target: Outline, t: f32) -> Outline {
    Outline {
        width: lerp(from.width, target.width, t),
        offset: lerp(from.offset, target.offset, t),
        color: interpolate_color(from.color, target.color, t),
    }
}

fn interpolate_radii(from: CornerRadii, target: CornerRadii, t: f32) -> CornerRadii {
    CornerRadii {
        top_left: lerp(from.top_left, target.top_left, t),
        top_right: lerp(from.top_right, target.top_right, t),
        bottom_right: lerp(from.bottom_right, target.bottom_right, t),
        bottom_left: lerp(from.bottom_left, target.bottom_left, t),
    }
}

fn interpolate_shadows(from: ShadowList, target: ShadowList, t: f32) -> ShadowList {
    let transparent = Shadow::default();
    let at = |list: ShadowList, index| list.as_slice().get(index).copied().unwrap_or(transparent);
    let shadow = |from: Shadow, target: Shadow| Shadow {
        offset: PointF {
            x: lerp(from.offset.x, target.offset.x, t),
            y: lerp(from.offset.y, target.offset.y, t),
        },
        blur: lerp(from.blur, target.blur, t),
        spread: lerp(from.spread, target.spread, t),
        color: interpolate_color(from.color, target.color, t),
    };
    let first = shadow(at(from, 0), at(target, 0));
    let second = shadow(at(from, 1), at(target, 1));
    if second.color.a == 0 && second.blur == 0.0 && second.spread == 0.0 {
        if first.color.a == 0 && first.blur == 0.0 && first.spread == 0.0 {
            ShadowList::default()
        } else {
            ShadowList::one(first)
        }
    } else {
        ShadowList::two(first, second)
    }
}

fn interpolate_transform(from: Transform2D, target: Transform2D, t: f32) -> Transform2D {
    Transform2D {
        translation: PointF {
            x: lerp(from.translation.x, target.translation.x, t),
            y: lerp(from.translation.y, target.translation.y, t),
        },
        scale: PointF {
            x: lerp(from.scale.x, target.scale.x, t),
            y: lerp(from.scale.y, target.scale.y, t),
        },
        rotation: lerp(from.rotation, target.rotation, t),
        origin: PointF {
            x: lerp(from.origin.x, target.origin.x, t),
            y: lerp(from.origin.y, target.origin.y, t),
        },
    }
}

fn interpolate_color(from: ColorRgba8, target: ColorRgba8, t: f32) -> ColorRgba8 {
    let linear_premultiplied = |color: ColorRgba8| {
        let alpha = f32::from(color.a) / 255.0;
        [
            (f32::from(color.r) / 255.0).powf(2.2) * alpha,
            (f32::from(color.g) / 255.0).powf(2.2) * alpha,
            (f32::from(color.b) / 255.0).powf(2.2) * alpha,
            alpha,
        ]
    };
    let from = linear_premultiplied(from);
    let target = linear_premultiplied(target);
    let mixed = [
        lerp(from[0], target[0], t),
        lerp(from[1], target[1], t),
        lerp(from[2], target[2], t),
        lerp(from[3], target[3], t),
    ];
    let alpha = mixed[3].clamp(0.0, 1.0);
    let channel = |value: f32| {
        if alpha <= f32::EPSILON {
            0
        } else {
            ((value / alpha).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8
        }
    };
    ColorRgba8::rgba(
        channel(mixed[0]),
        channel(mixed[1]),
        channel(mixed[2]),
        (alpha * 255.0).round() as u8,
    )
}

fn lerp(from: f32, target: f32, t: f32) -> f32 {
    from + (target - from) * t.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests;

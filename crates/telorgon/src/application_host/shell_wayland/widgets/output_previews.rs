//! Desktop thumbnails reuse producer scenes; no readback, new texture, timer or GPU submission.
use super::*;
use crate::RectF;
use std::collections::BTreeSet;

impl WidgetLayer {
    pub(in crate::application_host::shell_wayland) fn output_preview_layers(
        &self,
        sources: &[ShellLayer],
        overlays: &BTreeSet<u32>,
        locked: bool,
    ) -> Vec<ShellLayer> {
        if locked || !self.presented {
            return Vec::new();
        }
        let extent = self.layer.runtime.extent();
        let output = RectI {
            x: 0,
            y: 0,
            width: self.output_extent.width,
            height: self.output_extent.height,
        };
        let mut result = Vec::new();
        for (index, preview) in self.binding.3.borrow().iter().take(8).enumerate() {
            // The current host presents one output. Never alias an unknown monitor to it.
            if preview.output != crate::shell::OutputId::MIN {
                continue;
            }
            let r = preview.rect;
            if ![r.x, r.y, r.width, r.height]
                .into_iter()
                .all(f32::is_finite)
                || r.x < 0.0
                || r.y < 0.0
                || r.width <= 0.0
                || r.height <= 0.0
                || r.x + r.width > extent.width
                || r.y + r.height > extent.height
            {
                continue;
            }
            let sx = self.sampled.width as f32 / extent.width.max(1.0);
            let sy = self.sampled.height as f32 / extent.height.max(1.0);
            let slot = RectI {
                x: self.sampled.x.saturating_add((r.x * sx).round() as i32),
                y: self.sampled.y.saturating_add((r.y * sy).round() as i32),
                width: (r.width * sx).round() as i32,
                height: (r.height * sy).round() as i32,
            };
            let Some(fitted) = fit_preview(self.output_extent, slot) else {
                continue;
            };
            for (source_index, source) in sources.iter().enumerate() {
                if !eligible(source, overlays) {
                    continue;
                }
                let Some(source_clip) = intersect_rect(source.clip.unwrap_or(output), output)
                else {
                    continue;
                };
                let clip = map_preview_rect(source_clip, output, fitted);
                let Some(clip) =
                    intersect_rect(clip, fitted).and_then(|c| intersect_rect(c, self.sampled))
                else {
                    continue;
                };
                // The producer is earlier in this frame and exclusively owns scene publication.
                let content = reference_content(&source.content);
                let scale = fitted.width as f32 / output.width.max(1) as f32;
                let mut rounded_clips = source.rounded_clips;
                for rounded in rounded_clips.iter_mut().flatten() {
                    rounded.rect = RectF {
                        x: fitted.x as f32 + rounded.rect.x * scale,
                        y: fitted.y as f32 + rounded.rect.y * scale,
                        width: rounded.rect.width * scale,
                        height: rounded.rect.height * scale,
                    };
                    rounded.radii.top_left *= scale;
                    rounded.radii.top_right *= scale;
                    rounded.radii.bottom_left *= scale;
                    rounded.radii.bottom_right *= scale;
                }
                result.push(ShellLayer {
                    key: ShellLayerKey::OutputPreview(self.id, index as u32, source_index as u32),
                    content,
                    source_extent: source.source_extent,
                    target: map_preview_rect(source.target, output, fitted),
                    clip: Some(clip),
                    rounded_clips,
                    visible: true,
                    // Thumbnail copies don't become backdrop/glass producers.
                    glass: None,
                });
            }
        }
        result
    }
}

fn eligible(source: &ShellLayer, overlays: &BTreeSet<u32>) -> bool {
    source.visible
        && source.glass.is_none()
        && match source.key {
            ShellLayerKey::Widget(id) => !overlays.contains(&id),
            ShellLayerKey::OutputPreview(..)
            | ShellLayerKey::WindowPreview(..)
            | ShellLayerKey::Cursor
            | ShellLayerKey::DragIcon(_)
            | ShellLayerKey::TilePreview(_)
            | ShellLayerKey::ResizeVeil(_)
            | ShellLayerKey::ResizePreviewBorder(_) => false,
            _ => true,
        }
}

fn reference_content(content: &ShellLayerContent) -> ShellLayerContent {
    match content {
        ShellLayerContent::Retained { scene, .. } => ShellLayerContent::Retained {
            scene: *scene,
            deltas: Vec::new(),
        },
        ShellLayerContent::Image {
            scene,
            content_version,
            alpha_mode,
            pixel_format,
            ..
        } => ShellLayerContent::Image {
            scene: *scene,
            content_version: *content_version,
            update: ShellImageUpdate::Unchanged,
            alpha_mode: *alpha_mode,
            pixel_format: *pixel_format,
        },
        ShellLayerContent::Decoration { scene, instance } => ShellLayerContent::Decoration {
            scene: *scene,
            instance: instance.clone(),
        },
        ShellLayerContent::Solid {
            scene,
            color,
            corner_radius,
        } => ShellLayerContent::Solid {
            scene: *scene,
            color: *color,
            corner_radius: *corner_radius,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::*;
    use crate::core::ColorRgba8;

    #[crate::component]
    struct Preview {}
    impl Component for Preview {
        fn view(&self) -> impl View {
            column().width(300.0).height(200.0)
        }
    }
    impl ShellWidget for Preview {
        fn surface(&self) -> ShellSurfaceSpec {
            ShellSurfaceSpec::new().placement(WidgetPlacement::center().width(300.0).height(200.0))
        }
    }
    fn widget() -> WidgetLayer {
        let host = crate::compose::shell_services::ShellServiceHost::new();
        let mut widget = WidgetLayer::new(
            7,
            crate::application_host::declaration::RegisteredShellWidget::new(Preview {}),
            SizeI {
                width: 800,
                height: 600,
            },
            AssetBundle::default(),
            crate::platform::ScaleFactor::new(1.0).unwrap(),
            &EventNotifier::new("preview test").unwrap(),
            host.services.clone(),
        )
        .unwrap();
        *widget.binding.3.borrow_mut() = vec![ShellOutputPreview::new(
            crate::shell::OutputId::MIN,
            RectF {
                x: 10.0,
                y: 10.0,
                width: 200.0,
                height: 150.0,
            },
        )];
        widget
            .prepare(
                SizeI {
                    width: 800,
                    height: 600,
                },
                shell_work_area_for_spec(SizeI {
                    width: 800,
                    height: 600,
                }),
                1,
            )
            .unwrap();
        widget
    }
    fn source(key: ShellLayerKey, scene: ShellSceneKey, visible: bool) -> ShellLayer {
        ShellLayer {
            key,
            content: ShellLayerContent::Solid {
                scene,
                color: ColorRgba8::rgba(30, 60, 90, 255),
                corner_radius: 0.0,
            },
            source_extent: SizeI {
                width: 800,
                height: 600,
            },
            target: RectI {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            clip: None,
            rounded_clips: [None, None],
            visible,
            glass: None,
        }
    }
    #[test]
    fn output_preview_excludes_overlays_cursors_other_previews_and_hidden_content() {
        let widget = widget();
        let sources = vec![
            source(ShellLayerKey::Background, ShellSceneKey::Background, true),
            source(ShellLayerKey::Widget(4), ShellSceneKey::Widget(4), true),
            source(ShellLayerKey::Widget(7), ShellSceneKey::Widget(7), true),
            source(
                ShellLayerKey::WindowPreview(7, 0, 1),
                ShellSceneKey::Surface(1),
                true,
            ),
            source(
                ShellLayerKey::OutputPreview(8, 0, 0),
                ShellSceneKey::Background,
                true,
            ),
            source(ShellLayerKey::Cursor, ShellSceneKey::CursorImage, true),
            source(ShellLayerKey::Surface(2), ShellSceneKey::Surface(2), false),
        ];
        let overlays = BTreeSet::from([7]);
        let previews = widget.output_preview_layers(&sources, &overlays, false);
        assert_eq!(previews.len(), 2);
        assert!(
            previews
                .iter()
                .all(|p| p.target.width == 200 && p.target.height == 150 && p.glass.is_none())
        );
        assert!(
            widget
                .output_preview_layers(&sources, &overlays, true)
                .is_empty()
        );
    }
    #[test]
    fn unavailable_output_invalid_slot_and_hidden_widget_never_show_primary_output() {
        let mut widget = widget();
        let sources = vec![source(
            ShellLayerKey::Background,
            ShellSceneKey::Background,
            true,
        )];
        widget.binding.3.borrow_mut()[0].output = crate::shell::OutputId::from_raw(2).unwrap();
        assert!(
            widget
                .output_preview_layers(&sources, &BTreeSet::new(), false)
                .is_empty()
        );
        widget.binding.3.borrow_mut()[0].output = crate::shell::OutputId::MIN;
        widget.binding.3.borrow_mut()[0].rect.x = f32::NAN;
        assert!(
            widget
                .output_preview_layers(&sources, &BTreeSet::new(), false)
                .is_empty()
        );
        widget.binding.3.borrow_mut()[0].rect.x = 200.0;
        assert!(
            widget
                .output_preview_layers(&sources, &BTreeSet::new(), false)
                .is_empty()
        );
        widget.binding.3.borrow_mut()[0].rect.x = 0.0;
        widget.presented = false;
        assert!(
            widget
                .output_preview_layers(&sources, &BTreeSet::new(), false)
                .is_empty()
        );
    }
    #[test]
    fn preview_references_do_not_republish_scene_deltas_and_stable_frames_are_idle() {
        let widget = widget();
        let extent = SizeI {
            width: 800,
            height: 600,
        };
        let mut scene =
            crate::application_host::shell_wayland::scene::ShellComposition::new(extent);
        let mut sources = vec![source(
            ShellLayerKey::Background,
            ShellSceneKey::Background,
            true,
        )];
        let previews = widget.output_preview_layers(&sources, &BTreeSet::new(), false);
        sources.extend(previews);
        let frame = scene.synchronize(extent, sources).unwrap();
        assert_eq!(frame.updates.len(), 1);
        assert_eq!(frame.placements.len(), 2);
        let mut sources = vec![source(
            ShellLayerKey::Background,
            ShellSceneKey::Background,
            true,
        )];
        sources.extend(widget.output_preview_layers(&sources, &BTreeSet::new(), false));
        assert!(scene.synchronize(extent, sources).is_none());
    }
    #[test]
    fn changing_only_an_output_slot_marks_widget_dirty() {
        let mut widget = widget();
        widget.scene(false);
        widget.binding.3.borrow_mut()[0].rect.x = 15.0;
        widget
            .prepare(
                SizeI {
                    width: 800,
                    height: 600,
                },
                shell_work_area_for_spec(SizeI {
                    width: 800,
                    height: 600,
                }),
                2,
            )
            .unwrap();
        assert!(widget.dirty());
    }
}

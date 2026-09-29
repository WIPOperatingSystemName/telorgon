use super::*;
use crate::authoring::compose::*;
use std::{cell::RefCell, num::NonZeroU32, rc::Rc};
type Seen = Rc<RefCell<Vec<(crate::shell::WindowId, ImageId)>>>;
struct Panel(Seen);
struct Entries(Seen);
macro_rules! fields {
    ($t:ty) => {
        impl ComponentFields for $t {
            type InputSnapshot = ();
            fn capture_inputs(&self) {}
            fn restore_inputs(&mut self, _: ()) -> bool {
                false
            }
            fn update_inputs(&mut self, _: Self) -> bool {
                false
            }
        }
    };
}
fields!(Panel);
fields!(Entries);
impl Component for Panel {
    fn view(&self) -> impl View {
        Entries(self.0.clone())
    }
}
impl ShellWidget for Panel {
    fn surface(&self) -> ShellSurfaceSpec {
        ShellSurfaceSpec::new().placement(WidgetPlacement::fill())
    }
}
impl Component for Entries {
    fn view(&self) -> impl View {
        let windows = self.context::<ShellContext>().windows();
        let mut row = row();
        let mut seen = Vec::new();
        for window in windows.open() {
            let id = window.id;
            let icon = windows.icon(id);
            seen.push((id, icon.image_id()));
            row = row.child(
                button().child(text(window.title).color(crate::ColorRgba8::rgba(248, 249, 252, 255)))
                    .key(format!("{}:{}", id.slot(), id.generation()))
                    .child(image(icon).width(24.0).height(24.0))
                    .width(40.0)
                    .height(40.0)
                    .on_press(move |this: &mut Self| {
                        this.context::<ShellContext>()
                            .windows()
                            .activate(id)
                            .unwrap();
                    }),
            );
        }
        *self.0.borrow_mut() = seen;
        row
    }
}
fn window(slot: u32) -> ShellWindow {
    ShellWindow {
        preview_size: None,
        id: crate::shell::WindowId::new(
            NonZeroU32::new(slot).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ),
        title: format!("Window {slot}"),
        application_id: None,
        application_identity: String::new(),
        icon: None,
        icon_name: None,
        active: false,
        minimized: false,
        maximized: false,
    }
}
#[test]
fn descendant_taskbar_tracks_existing_open_closed_and_delayed_icons_and_dispatches_actions() {
    let host = crate::authoring::compose::shell_services::ShellServiceHost::new();
    host.publish(vec![window(1)]);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let (root, binding) = crate::authoring::compose::shell_widget::erase(Panel(seen.clone()));
    let output = SizeI {
        width: 800,
        height: 600,
    };
    let layer = WidgetLayer::new(
        1,
        crate::host::application::declaration::RegisteredShellWidget {
            content: CompositionDriver::from_erased_for_target(
                root,
                RuntimeTarget::ShellWidget,
            ),
            surface: binding,
        },
        output,
        &LayerAssets::new(AssetBundle::default()).unwrap(),
        crate::platform::contracts::ScaleFactor::new(1.0).unwrap(),
        &EventNotifier::new("taskbar-test").unwrap(),
        host.services.clone(),
    )
    .unwrap();
    let mut layers = vec![layer];
    let mut retained = crate::graphics::renderers::vulkan::VulkanScene::default();
    let admit = |layer: &mut WidgetLayer,
                 retained: &mut crate::graphics::renderers::vulkan::VulkanScene| {
        for delta in layer.layer.take_deltas() {
            retained.apply_delta_checked(&delta).unwrap();
        }
    };
    admit(&mut layers[0], &mut retained);
    assert_eq!(seen.borrow().len(), 1);
    let now = MonotonicInstant::from_nanos(1);
    widget_pointer_button(&mut layers, PointF { x: 20.0, y: 20.0 }, 0x110, true, now, false).unwrap();
    widget_pointer_button(&mut layers, PointF { x: 20.0, y: 20.0 }, 0x110, false, now, false).unwrap();
    layers[0]
        .prepare(output, shell_work_area_for_spec(output), 2)
        .unwrap();
    assert_eq!(host.drain()[0].window, window(1).id);
    let mut second = window(2);
    second.minimized = true;
    host.publish(vec![window(1), second.clone()]);
    layers[0]
        .prepare(output, shell_work_area_for_spec(output), 3)
        .unwrap();
    admit(&mut layers[0], &mut retained);
    assert_eq!(seen.borrow().len(), 2);
    let mut icon = crate::authoring::compose::applications::fallback_image();
    icon.image = ImageId(0x6800_ffff);
    second.icon = Some(icon.clone());
    host.publish(vec![second]);
    layers[0]
        .prepare(output, shell_work_area_for_spec(output), 4)
        .unwrap();
    admit(&mut layers[0], &mut retained);
    assert_eq!(seen.borrow().as_slice(), &[(window(2).id, icon.image)]);
    host.publish(Vec::new());
    layers[0]
        .prepare(output, shell_work_area_for_spec(output), 5)
        .unwrap();
    admit(&mut layers[0], &mut retained);
    assert!(seen.borrow().is_empty());
}

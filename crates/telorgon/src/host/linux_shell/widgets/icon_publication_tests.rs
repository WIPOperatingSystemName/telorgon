use super::*;
use crate::authoring::compose::*;
use std::{num::NonZeroU32, rc::Rc};
struct LateIcon {
    host: Rc<crate::authoring::compose::shell_services::ShellServiceHost>,
    next: Signal<Option<ShellWindow>>,
}
impl ComponentFields for LateIcon {
    type InputSnapshot = ();
    fn capture_inputs(&self) {}
    fn restore_inputs(&mut self, _: ()) -> bool {
        false
    }
    fn update_inputs(&mut self, _: Self) -> bool {
        false
    }
}
impl Component for LateIcon {
    fn view(&self) -> impl View {
        // Simulate an icon completion after the host's pre-view resource sync.
        if let Some(window) = self.watch(&self.next).as_ref() {
            self.host.publish(vec![window.clone()]);
        }
        let windows = self.context::<ShellContext>().windows();
        let window = windows.open().remove(0);
        button().accessible_label(window.title).child(crate::compose::image(windows.icon(window.id)).width(18.0).height(18.0))
    }
}
impl ShellWidget for LateIcon {
    fn surface(&self) -> ShellSurfaceSpec {
        ShellSurfaceSpec::new().placement(WidgetPlacement::fill())
    }
}
#[test]
fn icon_published_during_view_is_in_the_same_delta_as_its_draw() {
    let host = Rc::new(crate::authoring::compose::shell_services::ShellServiceHost::new());
    let mut window = ShellWindow {
        preview_size: None,
        id: crate::shell::WindowId::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ),
        title: "App".into(),
        application_id: None,
        application_identity: String::new(),
        icon: None,
        icon_name: None,
        active: false,
        minimized: false,
        maximized: false,
    };
    host.publish(vec![window.clone()]);
    let (next, writer) = Signal::new(None);
    let (root, binding) = crate::authoring::compose::shell_widget::erase(LateIcon {
        host: host.clone(),
        next,
    });
    let output = SizeI {
        width: 800,
        height: 600,
    };
    let mut layer = WidgetLayer::new(
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
        &EventNotifier::new("late-icon-test").unwrap(),
        host.services.clone(),
    )
    .unwrap();
    let mut retained = crate::graphics::renderers::vulkan::VulkanScene::default();
    for delta in layer.layer.take_deltas() {
        retained
            .apply_delta_checked(&delta)
            .unwrap_or_else(|e| panic!("{e:?} delta={delta:?}"));
    }
    let mut icon = crate::authoring::compose::applications::fallback_image();
    icon.image = ImageId(0x6800_ffff);
    window.icon = Some(icon);
    writer.publish(Some(window));
    layer
        .prepare(output, shell_work_area_for_spec(output), 1)
        .unwrap();
    for delta in layer.layer.take_deltas() {
        retained
            .apply_delta_checked(&delta)
            .unwrap_or_else(|e| panic!("{e:?} delta={delta:?}"));
    }
}

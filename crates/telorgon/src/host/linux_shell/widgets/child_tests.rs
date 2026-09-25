use super::*;
use crate::authoring::compose::*;
use std::{cell::Cell, rc::Rc};
struct Parent {
    items: Signal<Vec<u32>>,
    motion: bool,
    mounted: Rc<Cell<u32>>,
    unmounted: Rc<Cell<u32>>,
}
struct Child {
    motion: bool,
    mounted: Rc<Cell<u32>>,
    unmounted: Rc<Cell<u32>>,
}
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
fields!(Parent);
fields!(Child);
impl Component for Parent {
    fn view(&self) -> impl View {
        text("parent")
    }
}
impl Component for Child {
    fn view(&self) -> impl View {
        text("child")
    }
    fn mounted(&mut self, _: &mut MountContext<Self>) {
        self.mounted.set(self.mounted.get() + 1);
    }
    fn unmounted(&mut self, _: &mut UnmountContext<Self>) {
        self.unmounted.set(self.unmounted.get() + 1);
    }
}
impl ShellWidget for Parent {
    fn surface(&self) -> ShellSurfaceSpec {
        ShellSurfaceSpec::new().placement(WidgetPlacement::center().width(200.0).height(80.0))
    }
    fn children(&self) -> Vec<ShellChild> {
        self.watch(&self.items)
            .iter()
            .map(|id| {
                ShellChild::new(
                    id.to_string(),
                    Child {
                        motion: self.motion,
                        mounted: self.mounted.clone(),
                        unmounted: self.unmounted.clone(),
                    },
                )
            })
            .collect()
    }
}
impl ShellWidget for Child {
    fn surface(&self) -> ShellSurfaceSpec {
        let mut spec = ShellSurfaceSpec::new()
            .placement(
                WidgetPlacement::attached(ShellEdge::Bottom)
                    .width(100.0)
                    .height(40.0),
            )
            .layer(ShellSurfaceLayer::Overlay);
        if self.motion {
            spec = spec.visibility_motion(crate::Minimize::shrink_and_fade(130));
        }
        spec
    }
}
#[test]
fn keyed_children_reorder_without_remount_and_unmount_on_removal() {
    child_lifecycle(false);
}
#[test]
fn fading_children_retire_after_exit_and_can_reopen_without_remount() {
    child_lifecycle(true);
}
fn child_lifecycle(motion: bool) {
    let (items, writer) = Signal::new(vec![1, 2]);
    let mounted = Rc::new(Cell::new(0));
    let unmounted = Rc::new(Cell::new(0));
    let (root, surface) = crate::authoring::compose::shell_widget::erase(Parent {
        items,
        motion,
        mounted: mounted.clone(),
        unmounted: unmounted.clone(),
    });
    let output = SizeI {
        width: 800,
        height: 600,
    };
    let scale = crate::platform::contracts::ScaleFactor::new(1.0).unwrap();
    let wake = EventNotifier::new("child test").unwrap();
    let host = crate::authoring::compose::shell_services::ShellServiceHost::new();
    let registered = crate::host::application::declaration::RegisteredShellWidget {
        content: CompositionDriver::from_erased_for_target(root, RuntimeTarget::ShellWidget),
        surface,
    };
    let mut widgets = vec![
        WidgetLayer::new(
            0,
            registered,
            output,
            &LayerAssets::new(AssetBundle::default()).unwrap(),
            scale,
            &wake,
            host.services.clone(),
        )
        .unwrap(),
    ];
    let mut next = 1;
    sync_widget_children(
        &mut widgets,
        &mut next,
        output,
        &LayerAssets::new(AssetBundle::default()).unwrap(),
        scale,
        &wake,
        &host.services,
    )
    .unwrap();
    // An attached trigger can toggle its popup without outside dismissal racing it.
    {
        let child = widgets.iter_mut().find(|w| w.child_key == "1").unwrap();
        child.parent_bounds = Some(RectI { x: 300, y: 260, width: 200, height: 80 });
        child.sampled = RectI { x: 300, y: 340, width: 100, height: 40 };
        let anchor_press = PointF { x: 350.0, y: 280.0 };
        assert!(child.outside_press(anchor_press));
        child.spec.outside_press_excludes_anchor = true;
        assert!(!child.outside_press(anchor_press));
        assert!(child.outside_press(PointF { x: 250.0, y: 280.0 }));
    }
    assert_eq!(mounted.get(), 2);
    let first = widgets.iter().find(|w| w.child_key == "1").unwrap().id;
    writer.publish_if_changed(vec![2, 1]);
    prepare_widget_surfaces(
        &mut widgets,
        output,
        shell_work_area_for_spec(output),
        1_000_000,
        crate::theme::MotionPreference::Full,
    )
    .unwrap();
    sync_widget_children(
        &mut widgets,
        &mut next,
        output,
        &LayerAssets::new(AssetBundle::default()).unwrap(),
        scale,
        &wake,
        &host.services,
    )
    .unwrap();
    assert_eq!(mounted.get(), 2);
    assert_eq!(
        widgets.iter().find(|w| w.child_key == "1").unwrap().id,
        first
    );
    writer.publish_if_changed(vec![2]);
    prepare_widget_surfaces(
        &mut widgets,
        output,
        shell_work_area_for_spec(output),
        2_000_000,
        crate::theme::MotionPreference::Full,
    )
    .unwrap();
    sync_widget_children(
        &mut widgets,
        &mut next,
        output,
        &LayerAssets::new(AssetBundle::default()).unwrap(),
        scale,
        &wake,
        &host.services,
    )
    .unwrap();
    if motion {
        assert_eq!(unmounted.get(), 0);
        assert_eq!(widgets.len(), 3);
        prepare_widget_surfaces(
            &mut widgets,
            output,
            shell_work_area_for_spec(output),
            3_000_000,
            crate::theme::MotionPreference::Full,
        )
        .unwrap();
        let retiring = widgets.iter().find(|w| w.id == first).unwrap();
        assert!(retiring.retiring);
        assert!(!retiring.input_visible());
        writer.publish_if_changed(vec![1, 2]);
        prepare_widget_surfaces(
            &mut widgets,
            output,
            shell_work_area_for_spec(output),
            4_000_000,
            crate::theme::MotionPreference::Full,
        )
        .unwrap();
        sync_widget_children(
            &mut widgets,
            &mut next,
            output,
            &LayerAssets::new(AssetBundle::default()).unwrap(),
            scale,
            &wake,
            &host.services,
        )
        .unwrap();
        assert!(!widgets.iter().find(|w| w.id == first).unwrap().retiring);
        assert_eq!(mounted.get(), 2);
        writer.publish_if_changed(vec![2]);
        prepare_widget_surfaces(
            &mut widgets,
            output,
            shell_work_area_for_spec(output),
            5_000_000,
            crate::theme::MotionPreference::Full,
        )
        .unwrap();
        sync_widget_children(
            &mut widgets,
            &mut next,
            output,
            &LayerAssets::new(AssetBundle::default()).unwrap(),
            scale,
            &wake,
            &host.services,
        )
        .unwrap();
        prepare_widget_surfaces(
            &mut widgets,
            output,
            shell_work_area_for_spec(output),
            6_000_000,
            crate::theme::MotionPreference::Full,
        )
        .unwrap();
        prepare_widget_surfaces(
            &mut widgets,
            output,
            shell_work_area_for_spec(output),
            200_000_000,
            crate::theme::MotionPreference::Full,
        )
        .unwrap();
        sync_widget_children(
            &mut widgets,
            &mut next,
            output,
            &LayerAssets::new(AssetBundle::default()).unwrap(),
            scale,
            &wake,
            &host.services,
        )
        .unwrap();
    }
    assert_eq!(unmounted.get(), 1);
    assert_eq!(widgets.len(), 2);
    let child = &widgets[1];
    assert_eq!(child.sampled.y, 340);
}

use crate::input::ChangeSource;

use crate::authoring::compose::*;

struct Example {
    title: String,
    count: u32,
}

impl ComponentFields for Example {
    type InputSnapshot = (String,);

    fn update_inputs(&mut self, incoming: Self) -> bool {
        let changed = self.title != incoming.title;
        self.title = incoming.title;
        changed
    }

    fn capture_inputs(&self) -> Self::InputSnapshot {
        (self.title.clone(),)
    }

    fn restore_inputs(&mut self, snapshot: Self::InputSnapshot) -> bool {
        let changed = self.title != snapshot.0;
        self.title = snapshot.0;
        changed
    }
}

impl Component for Example {
    fn view(&self) -> impl View {
        column()
            .child(text(format!("{}: {}", self.title, self.count)))
            .child(button().child(text("Increment").color(crate::ColorRgba8::rgba(248, 249, 252, 255))).on_press(|this: &mut Self| this.count += 1))
    }
}

#[test]
fn component_view_is_valid_and_input_mutation_is_restored() {
    let mut component: Box<dyn ErasedComponent> = Box::new(Example {
        title: "Counter".to_owned(),
        count: 0,
    });
    let id = ComponentInstanceId::new(1, 1);
    let rendered = component.render(id, RuntimeTarget::Application);
    rendered.element.validate().unwrap();
    let ElementKind::Container(container) = rendered.element.kind() else {
        panic!("expected a container")
    };
    let ElementKind::Button(button) = container.children[1].kind() else {
        panic!("expected a button")
    };
    let handler = button.on_press.clone().unwrap().bind(id);
    assert_eq!(
        handler.dispatch(component.as_mut(), ChangeSource::Pointer),
        EventDispatch::Delivered {
            input_mutated: false
        }
    );
    let next = component.render(id, RuntimeTarget::Application);
    let ElementKind::Container(container) = next.element.kind() else {
        panic!("expected a container")
    };
    let ElementKind::Text(text) = container.children[0].kind() else {
        panic!("expected text")
    };
    assert_eq!(text.content, "Counter: 1");
}

#[test]
fn button_content_is_authored_as_children() {
    let button = crate::authoring::compose::button().child(text("Increment").color(crate::ColorRgba8::rgba(248, 249, 252, 255))).into_element();
    let crate::authoring::compose::ElementKind::Button(button) = button.kind() else {
        panic!("expected a button")
    };

    assert_eq!(button.children.len(), 1);
}

use super::{Component, ComponentInstanceId, ErasedComponent, ViewError};
use crate::ui::UiEvent;
use std::{any::TypeId, fmt, rc::Rc};

trait Callback {
    fn type_id(&self) -> TypeId;
    fn type_name(&self) -> &'static str;
    fn dispatch(
        &self,
        component: &mut dyn ErasedComponent,
        event: &UiEvent,
    ) -> Option<(bool, bool)>;
}
struct Typed<C, F>(F, std::marker::PhantomData<fn(C)>);
impl<C: Component, F: Fn(&mut C, &UiEvent) -> bool + 'static> Callback for Typed<C, F> {
    fn type_id(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn type_name(&self) -> &'static str {
        std::any::type_name::<C>()
    }
    fn dispatch(
        &self,
        component: &mut dyn ErasedComponent,
        event: &UiEvent,
    ) -> Option<(bool, bool)> {
        let component = component.as_any_mut().downcast_mut::<C>()?;
        let inputs = component.capture_inputs();
        let changed = (self.0)(component, event);
        Some((changed, component.restore_inputs(inputs)))
    }
}

/// Targeted input for composed controls, including keyboard and focus events.
#[derive(Clone)]
pub struct InputCallback(Rc<dyn Callback>);
impl fmt::Debug for InputCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputCallback")
            .field("component", &self.0.type_name())
            .finish()
    }
}
impl InputCallback {
    pub(crate) fn new<C: Component, F: Fn(&mut C, &UiEvent) -> bool + 'static>(
        callback: F,
    ) -> Self {
        Self(Rc::new(Typed(callback, std::marker::PhantomData::<fn(C)>)))
    }
    pub(crate) fn validate(
        &self,
        component: Option<(TypeId, &'static str)>,
    ) -> Result<(), ViewError> {
        if let Some((id, name)) = component {
            if self.0.type_id() != id {
                return Err(ViewError::CallbackTypeMismatch {
                    expected: name,
                    actual: self.0.type_name(),
                });
            }
        }
        Ok(())
    }
    pub(crate) fn bind(&self, owner: ComponentInstanceId) -> InputHandler {
        InputHandler {
            owner,
            callback: self.clone(),
        }
    }
}
#[derive(Clone)]
pub(crate) struct InputHandler {
    pub owner: ComponentInstanceId,
    callback: InputCallback,
}
impl InputHandler {
    pub fn dispatch(
        &self,
        component: &mut dyn ErasedComponent,
        event: &UiEvent,
    ) -> Option<(bool, bool)> {
        self.callback.0.dispatch(component, event)
    }
}

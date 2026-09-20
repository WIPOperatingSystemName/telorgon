use std::any::TypeId;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use crate::input::{ChangeSource, ValueChangePhase};
use crate::ui::SemanticCheckState;

use crate::authoring::compose::{Component, ComponentInstanceId, ErasedComponent};

/// Context available while an event callback mutates its owning component.
#[derive(Clone, Copy, Debug)]
pub struct EventContext {
    source: ChangeSource,
    value: Option<f32>,
    value_phase: Option<ValueChangePhase>,
    checked: Option<SemanticCheckState>,
}

impl EventContext {
    #[doc(hidden)]
    pub const fn new(source: ChangeSource) -> Self {
        Self {
            source,
            value: None,
            value_phase: None,
            checked: None,
        }
    }

    pub const fn source(&self) -> ChangeSource {
        self.source
    }

    pub const fn value(&self) -> Option<f32> {
        self.value
    }

    pub const fn value_phase(&self) -> Option<ValueChangePhase> {
        self.value_phase
    }

    pub const fn checked(&self) -> Option<SemanticCheckState> {
        self.checked
    }
}

trait ErasedComponentCallback {
    fn component_type_id(&self) -> TypeId;
    fn component_type_name(&self) -> &'static str;
    fn dispatch(
        &self,
        component: &mut dyn ErasedComponent,
        event: &mut EventContext,
    ) -> EventDispatch;
}

struct TypedComponentCallback<C, F> {
    callback: F,
    marker: PhantomData<fn(C)>,
}

impl<C, F> ErasedComponentCallback for TypedComponentCallback<C, F>
where
    C: Component,
    F: Fn(&mut C, &mut EventContext) + 'static,
{
    fn component_type_id(&self) -> TypeId {
        TypeId::of::<C>()
    }

    fn component_type_name(&self) -> &'static str {
        std::any::type_name::<C>()
    }

    fn dispatch(
        &self,
        component: &mut dyn ErasedComponent,
        event: &mut EventContext,
    ) -> EventDispatch {
        let Some(component) = component.as_any_mut().downcast_mut::<C>() else {
            return EventDispatch::WrongComponentType;
        };
        let inputs = component.capture_inputs();
        (self.callback)(component, event);
        EventDispatch::Delivered {
            input_mutated: component.restore_inputs(inputs),
        }
    }
}

/// An owner-independent callback in a short-lived component view description.
#[derive(Clone)]
pub struct ComponentCallback {
    callback: Rc<dyn ErasedComponentCallback>,
}

impl fmt::Debug for ComponentCallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComponentCallback")
            .field("component", &self.component_type_name())
            .finish_non_exhaustive()
    }
}

impl ComponentCallback {
    pub(crate) fn for_component<C, F>(callback: F) -> Self
    where
        C: Component,
        F: Fn(&mut C, &mut EventContext) + 'static,
    {
        Self {
            callback: Rc::new(TypedComponentCallback::<C, F> {
                callback,
                marker: PhantomData,
            }),
        }
    }

    pub(crate) fn component_type_id(&self) -> TypeId {
        self.callback.component_type_id()
    }

    pub(crate) fn component_type_name(&self) -> &'static str {
        self.callback.component_type_name()
    }

    #[doc(hidden)]
    pub fn bind(&self, owner: ComponentInstanceId) -> EventHandler {
        EventHandler {
            owner,
            callback: Rc::clone(&self.callback),
        }
    }
}

/// A generation-bound callback installed in the retained interaction route table.
#[derive(Clone)]
pub struct EventHandler {
    owner: ComponentInstanceId,
    callback: Rc<dyn ErasedComponentCallback>,
}

impl fmt::Debug for EventHandler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventHandler")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl EventHandler {
    pub const fn owner(&self) -> ComponentInstanceId {
        self.owner
    }

    #[doc(hidden)]
    pub fn dispatch(
        &self,
        component: &mut dyn ErasedComponent,
        source: ChangeSource,
    ) -> EventDispatch {
        let mut event = EventContext::new(source);
        self.callback.dispatch(component, &mut event)
    }

    #[doc(hidden)]
    pub fn dispatch_value(
        &self,
        component: &mut dyn ErasedComponent,
        source: ChangeSource,
        value: f32,
        phase: ValueChangePhase,
    ) -> EventDispatch {
        let mut event = EventContext {
            source,
            value: Some(value.clamp(0.0, 1.0)),
            value_phase: Some(phase),
            checked: None,
        };
        self.callback.dispatch(component, &mut event)
    }

    #[doc(hidden)]
    pub fn dispatch_checked(
        &self,
        component: &mut dyn ErasedComponent,
        source: ChangeSource,
        checked: SemanticCheckState,
    ) -> EventDispatch {
        let mut event = EventContext {
            source,
            value: None,
            value_phase: None,
            checked: Some(checked),
        };
        self.callback.dispatch(component, &mut event)
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventDispatch {
    Delivered { input_mutated: bool },
    WrongComponentType,
}

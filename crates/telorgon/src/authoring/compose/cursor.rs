use crate::assets::PointerRequest;
use crate::platform::contracts::PointerIcon;

// Hidden is an authoring choice, not a platform standard cursor shape.
macro_rules! cursor_icons {
    ($($name:ident),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum CursorIcon {
            /// Hide the cursor while the view owns the pointer request.
            None,
            /// Explicitly request the default cursor; omission preserves automatic behavior.
            #[default]
            Default,
            $($name,)*
        }

        impl From<CursorIcon> for PointerRequest {
            fn from(icon: CursorIcon) -> Self {
                match icon {
                    CursorIcon::None => Self::Hidden,
                    CursorIcon::Default => Self::Semantic(PointerIcon::Default),
                    $(CursorIcon::$name => Self::Semantic(PointerIcon::$name),)*
                }
            }
        }

        impl From<PointerIcon> for CursorIcon {
            fn from(icon: PointerIcon) -> Self {
                match icon {
                    PointerIcon::Default => Self::Default,
                    $(PointerIcon::$name => Self::$name,)*
                }
            }
        }
    };
}

cursor_icons! {
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    EResize,
    NResize,
    NeResize,
    NwResize,
    SResize,
    SeResize,
    SwResize,
    WResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
    DndAsk,
    AllResize,
}

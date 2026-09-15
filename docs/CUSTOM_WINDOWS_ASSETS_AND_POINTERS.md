# Custom Windows, Assets, Icons, and Pointers

## Document role

This document describes Telorgon's current composable window-chrome, project-asset, application-icon,
and pointer-theme APIs. It also records the host/protocol boundary and compatibility decisions made
during the refactor.

## Complete managed-GUI example

`asset_catalog!` belongs to `telorgon-macros` and is re-exported by `telorgon`, so applications add
only the normal `telorgon` dependency. The macro embeds a project directory, generates nested typed
constants from its paths, and exposes one cheap `AssetBundle`.

```text
assets/
  icons/app.svg
  icons/app-32.png
  icons/close.svg
  icons/minimize.svg
  icons/maximize.svg
  icons/restore.svg
  cursors/default.toml
  cursors/arrow.svg
  cursors/link.svg
  cursors/text.svg
  cursors/resize-ew.svg
```

```rust,ignore
use telorgon::app::*;
use telorgon::{
    Background, ClientCursorMode, CursorGraphic, PointerIcon, PointerThemeOverrides,
};

asset_catalog! { pub mod assets = "assets"; }

const WHITE: ColorRgba8 = ColorRgba8::rgba(255, 255, 255, 255);

fn app_icons() -> AppIconProfile {
    AppIconProfile::new()
        // Desktop icon-theme name used when the environment can resolve one.
        .named("com.example.studio")
        // A scalable project-owned fallback recolored from its alpha mask.
        .icon(Icon::new(assets::icons::APP).tint(WHITE))
        // Exact raster variants win for their declared size.
        .icon_at(32, assets::icons::APP_32)
}

fn pointers() -> PointerThemeOverrides {
    // Override only exceptional states here; all other states come from default.toml or the OS.
    PointerThemeOverrides::new().set(
        PointerIcon::EwResize,
        CursorGraphic::new(assets::cursors::RESIZE_EW)
            .size(32)
            .hotspot(16, 16)
            .tint(WHITE),
    )
}

#[component]
struct Editor {}

impl Component for Editor {
    fn view(&self) -> impl View {
        column()
            .padding(24.0)
            .child(text("Document title").pointer_icon(PointerIcon::Text))
            .child(text("The application owns this content."))
    }
}

#[component]
struct ManagedFrame {}

impl Component for ManagedFrame {
    fn view(&self) -> impl View {
        let frame = BoxDecoration::new()
            .background(Background::Color(ColorRgba8::rgba(24, 27, 35, 255)))
            .uniform_border(1.0, ColorRgba8::rgba(71, 78, 96, 255))
            .corner_radius(14.0)
            .shadow(Shadow {
                offset: PointF { x: 0.0, y: 10.0 },
                blur: 28.0,
                spread: 0.0,
                color: ColorRgba8::rgba(0, 0, 0, 112),
            });
        let control = |label, icon, action| {
            button(label)
                .icon(icon)
                .icon_tint(WHITE)
                .icon_size(16.0)
                .width(38.0)
                .height(30.0)
                .decoration(
                    BoxDecoration::new()
                        .background(Background::Color(ColorRgba8::rgba(42, 47, 59, 255)))
                        .corner_radius(8.0),
                )
                .window_action(action)
        };

        window_frame()
            .decoration(frame)
            .child(
                row()
                    .height(46.0)
                    .padding((8.0, 8.0))
                    .gap(6.0)
                    .child(
                        image(assets::icons::APP)
                            .tint(WHITE)
                            .width(20.0)
                            .height(20.0)
                            .window_app_icon(),
                    )
                    .child(text("Studio").window_title())
                    .child(spacer())
                    .child(control(
                        "Minimize",
                        assets::icons::MINIMIZE,
                        WindowAction::Minimize,
                    ))
                    .child(control(
                        "Maximize",
                        assets::icons::MAXIMIZE,
                        WindowAction::ToggleMaximize,
                    ))
                    .child(control("Close", assets::icons::CLOSE, WindowAction::Close))
                    .window_drag_region(),
            )
            // Thin transparent overlays can be placed wherever resize affordances belong.
            .child(
                stack()
                    .width(6.0)
                    .window_resize(WindowResizeEdge::Left),
            )
            .content_slot(
                window_content_slot()
                    .margin((46.0, 6.0, 6.0, 6.0))
                    .child(Editor::default()),
            )
    }
}

fn main() -> telorgon::Result<()> {
    Application::gui("Studio")
        .assets(assets::bundle())
        .cursor_theme(assets::cursors::DEFAULT)
        .pointer_overrides(pointers())
        .window(
            Window::new("Studio")
                .size(1100, 720)
                .custom_frame()
                .icon(app_icons())
                .content(ManagedFrame::default()),
        )
        .run()
}
```

The visible label passed to `.icon(...)` becomes the icon-only button's accessible name. Button
hover, focus, pressed, busy, and disabled visuals continue to come from normal component styling;
the window host consumes only the semantic `WindowAction` after hit testing.

## Complete Wayland desktop example

The convenience path is a reusable value, not a closure. `EasyWindowFrame` receives one immutable
`WindowChromeModel` per toplevel and resolves it against a complete `WindowChromeDesign`. Control
icons live beside their control visuals in that design; they do not need separate compositor icon
declarations.

```rust,ignore
use telorgon::app::*;
use telorgon::{Easing, TransitionSpec};

const RESTING: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new()
        .background(Background::Color(ColorRgba8::rgba(35, 40, 52, 255)))
        .corner_radius(7.0),
    icon_tint: ColorRgba8::rgba(225, 230, 242, 255),
};

const HOVERED: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new()
        .background(Background::Color(ColorRgba8::rgba(60, 70, 96, 255)))
        .corner_radius(7.0),
    icon_tint: ColorRgba8::rgba(255, 255, 255, 255),
};

const PRESSED: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new()
        .background(Background::Color(ColorRgba8::rgba(82, 96, 132, 255)))
        .corner_radius(7.0),
    icon_tint: ColorRgba8::rgba(255, 255, 255, 255),
};

const FOCUSED: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new()
        .uniform_border(2.0, ColorRgba8::rgba(130, 155, 255, 255))
        .corner_radius(7.0),
    icon_tint: ColorRgba8::rgba(255, 255, 255, 255),
};

const DISABLED: WindowControlVisual = WindowControlVisual {
    decoration: BoxDecoration::new().corner_radius(7.0),
    icon_tint: ColorRgba8::rgba(120, 126, 142, 255),
};

const BUTTON: WindowControlButtonStyle = WindowControlButtonStyle {
    width: Dimension::Logical(38.0),
    height: Dimension::Logical(30.0),
    icon_size: 15.0,
    resting: RESTING,
    hovered: Some(HOVERED),
    pressed: Some(PRESSED),
    focused: Some(FOCUSED),
    disabled: Some(DISABLED),
    transition: Some(TransitionSpec {
        duration_ms: 90,
        easing: Easing::EaseOut,
        repeat: false,
    }),
};

const CLOSE_BUTTON: WindowControlButtonStyle = WindowControlButtonStyle {
    resting: WindowControlVisual {
        decoration: BoxDecoration::new()
            .background(Background::Color(ColorRgba8::rgba(77, 35, 45, 255)))
            .corner_radius(7.0),
        icon_tint: ColorRgba8::rgba(255, 220, 225, 255),
    },
    hovered: Some(WindowControlVisual {
        decoration: BoxDecoration::new()
            .background(Background::Color(ColorRgba8::rgba(196, 52, 72, 255)))
            .corner_radius(7.0),
        icon_tint: ColorRgba8::rgba(255, 255, 255, 255),
    }),
    ..BUTTON
};

const NORMAL: WindowChromeStateStyle = WindowChromeStateStyle {
    title_bar_visible: true,
    frame_radius: 14.0,
    shadow: Some(Shadow {
        offset: PointF { x: 0.0, y: 12.0 },
        blur: 30.0,
        spread: 0.0,
        color: ColorRgba8::rgba(0, 0, 0, 128),
    }),
    resize_regions: true,
    resize_edge: 6.0,
    resize_hit_slop: Insets::all(3.0),
};

const TEST_CHROME: WindowChromeDesign = WindowChromeDesign {
    motion: WindowMotion::none(),
    active: WindowChromePalette {
        frame_background: ColorRgba8::rgba(23, 27, 38, 255),
        frame_border: ColorRgba8::rgba(101, 119, 184, 255),
        frame_border_width: 1.0,
        title_color: ColorRgba8::rgba(245, 247, 255, 255),
        title_weight: 650,
    },
    inactive: WindowChromePalette {
        frame_background: ColorRgba8::rgba(31, 34, 43, 255),
        frame_border: ColorRgba8::rgba(65, 70, 85, 255),
        frame_border_width: 1.0,
        title_color: ColorRgba8::rgba(174, 179, 193, 255),
        title_weight: 450,
    },
    normal: NORMAL,
    maximized: WindowChromeStateStyle {
        frame_radius: 0.0,
        shadow: None,
        resize_regions: false,
        resize_edge: 0.0,
        resize_hit_slop: Insets::ZERO,
        ..NORMAL
    },
    tiled: WindowChromeStateStyle {
        frame_radius: 0.0,
        shadow: None,
        ..NORMAL
    },
    fullscreen: WindowChromeStateStyle {
        title_bar_visible: false,
        frame_radius: 0.0,
        shadow: None,
        resize_regions: false,
        resize_edge: 0.0,
        resize_hit_slop: Insets::ZERO,
    },
    title_bar: WindowTitleBarStyle {
        height: 44.0,
        padding: Insets::symmetric(7.0, 8.0),
        gap: 8.0,
        title_size: 14.0,
        app_icon_region_size: 30.0,
        app_icon_size: 20.0,
        show_client_icon: true,
        fallback_app_icon: Some(assets::icons::APP),
        app_icon_opens_system_menu: true,
    },
    controls: WindowControlsDesign {
        minimize: WindowControlDesign { icon: assets::icons::MINIMIZE, style: BUTTON },
        maximize: WindowControlDesign { icon: assets::icons::MAXIMIZE, style: BUTTON },
        restore: WindowControlDesign { icon: assets::icons::RESTORE, style: BUTTON },
        close: WindowControlDesign { icon: assets::icons::CLOSE, style: CLOSE_BUTTON },
        gap: 6.0,
    },
    content_background: ColorRgba8::rgba(15, 18, 26, 255),
    resize_preview: None, // Inherit LinuxDesktopConfig; Some(ResizePreview::Color(rgba(...))) overrides it.
};

#[component]
struct DesktopBackground {}

impl Component for DesktopBackground {
    fn view(&self) -> impl View {
        stack().background(ColorRgba8::rgba(10, 12, 18, 255))
    }
}

Application::desktop_environment("Telorgon")
    .assets(assets::bundle())
    .app_icon(app_icons())
    .compositor(
        Compositor::new()
            .cursor_theme(CursorTheme::from_asset(assets::cursors::DEFAULT))
            .client_cursor_mode(ClientCursorMode::Allow)
            .window_frame(easy_window_frame(TEST_CHROME))
            .background(DesktopBackground::default()),
    )
    .shell_widget(ShellWidget::new("panel").content(MyPanel::default()))
    .run()?;
```

`WindowChromeDesign::validate()` can be called before assembling the declaration to report invalid
nonfinite or negative metrics early. At runtime the easy frame selects active/inactive palettes,
normal/maximized/tiled/fullscreen geometry, client/fallback app icons, capability-filtered controls,
maximize versus restore artwork, resizable tiled edges, and every declared control state. The state
styles are code-local bindings, so no theme-catalog entry is required.

Control button `width` and `height` accept the composition `Dimension` primitives: `Logical` (fixed logical units),
`Percent` (a fraction from 0 to 1), weighted `Fill`, and `Shrink`. Migrate numeric struct fields to
`Dimension::Logical(value)` (or `value.into()` outside constants). To span the title bar vertically,
use `height: Dimension::FILL` for each control and zero vertical title-bar padding. Fill respects
padding; fixed and percentage heights are centered within the remaining height. Chrome controls
have no inherited 32px minimum, so compact bars can use smaller buttons; keep icons small enough to fit.

Maximized and fullscreen easy frames suppress `frame_border_width`; normal and tiled frames
retain it. Maximized custom frames fill the shell work area, and the host derives the client size
from their laid-out content slot. See [Maximized window geometry](MAXIMIZED_WINDOW_GEOMETRY.md).

Controls participate directly in the title-bar row. Percentage widths resolve against the bar's
padded width; fill weights share remaining width with the flexible spacer (weight 1). Pixel and
shrink widths leave the spacer to push controls to the right. The title-bar height itself remains
a pixel metric because it also determines the client content inset.

Easy-frame layout is derived from the actual border and title bar. The app content begins at
`(frame_border_width, frame_border_width + title_bar.height)` when the bar is visible, and at
`(frame_border_width, frame_border_width)` when it is hidden. Right and bottom insets equal the border
width; the layout engine applies that border once. There is no separate decorative margin band.

Migration: remove `content_margin` and `content_radius` from `WindowChromeStateStyle` literals.
Use `title_bar.height` for the bar, `frame_border_width` for the visible outline, and `frame_radius`
for its outer curve. The inner radius follows the inset border automatically. Increase the actual
border width if a thicker surround is wanted; do not use resize hit width as visible padding.
Custom `WindowFrameTemplate` layouts can still use margins and independent aperture rounding.

The easy frame exposes independent normal-content and resize-preview RGBA colors:

```rust
let chrome = WindowChromeDesign {
    // Allow transparent application pixels to reveal the desktop or lower windows.
    content_background: ColorRgba8::rgba(0, 0, 0, 0),
    // A translucent slate resize placeholder; 0 alpha gives a frame-only preview.
    resize_preview: Some(ResizePreview::Color(ColorRgba8::rgba(38, 42, 48, 160))),
    ..TEST_CHROME
};
let frame = easy_window_frame(chrome);
```

`resize_preview: None` inherits `LinuxDesktopConfig::resize_preview` (opaque slate
by default); add this field to existing complete `WindowChromeDesign` literals when upgrading.
All alpha values from 0 through 255 are accepted. The preview replaces the client surface tree and
normal backing, so its alpha reveals lower desktop layers, not the old client image. Normal
`content_background` alpha is independent: an opaque value intentionally fills transparent app
pixels. The app must provide an alpha-capable buffer with transparent pixels; XRGB and fully opaque
app content remain opaque. This does not make transparent pixels click-through or add blur/fades.

For externally supplied Wayland content, the host excludes the entire composed frame from the
content rectangle, including its root fill and shadow, and paints the content backing separately.
The easy frame's content slot carries layout and clipping only; `content_style()` supplies its
background to the host. Painting that background in the slot as well lets fractional border/title
metrics leak a thin strip outside the host's integer client cutout. Fractional outline widths remain
unchanged, and backing, client placement, and the cutout share the host's content rectangle.
Client pixels, backing, and resize preview are clipped to the inner frame-border contour. For a
uniform border the inner radius is `max(frame_radius - frame_border_width, 0)`, with its rectangle
inset by the border width. Zero-radius frames still clip to their rectangular interior; zero-width
borders use the outer curve. Easy frames use this one full-window inner contour, not another rounded
rectangle around the app: the title-bar seam stays square and the bottom corners follow the actual
border thickness. The border-only patch preserves the curved outline inside the rectangular cutout.
Custom templates may additionally set `WindowContentStyle::corner_radius` for a narrower aperture;
only those templates need the inverse-clipped frame-fill corner patch. Its contour starts at the
window's inner top edge rather than the app/title-bar seam.
Subsurfaces inherit the window clip; popups keep independent bounds. Easy-frame composed children
also clip to the frame/slot overflow bounds, without clipping away the frame's own outer shadow.
Custom `WindowFrameTemplate` implementations can opt into the same transparent-backing
separation by returning `Some(WindowContentStyle { background, corner_radius,
resize_preview })` from `content_style(&WindowChromeModel)`. Its default `None` preserves
their normal composed backing; during resize the host still excludes that backing from the preview.
The curved border segment inside the rectangular content slot is retained even when content is
transparent. Undecorated/client-decorated windows keep their client-authored shape and use the Linux
configuration's preview color. No new design fields are required for rounded clipping.
The same placeholder is shown while maximizing until newly configured client content is published.
It uses the existing preview color and alpha settings; no extra application configuration is needed.

Fractional-seam repair audit (2026-09-15): inspected `easy_window_frame.rs`, desktop
`layers.rs`/`geometry.rs`/`scene.rs`, and the software renderer's composition path. The adjacent
`other-rendering-libs` library was unavailable, so no external implementation comparison was made.
The [Wayland surface specification](https://wayland.freedesktop.org/docs/html/apa.html#protocol-spec-wl_surface)
distinguishes surface-local geometry from buffer pixels; the fix preserves that mapping and client
sizes. The adopted invariant is that changing content backing cannot change pixels outside the
integer client cutout. Rounding the authored border width or moving the client was rejected because
the duplicate slot fill can be removed without changing either. The CPU framebuffer regression
compares black and white backing around all four frame strips for integer/fractional borders and
title heights, including hidden title bars; it failed before the repair and passes afterward.
Live Vulkan/foot visual confirmation remains user-run.

See [resize-preview behavior and verification](WAYLAND_RESIZE_PREVIEW.md) and
[maximized window geometry](MAXIMIZED_WINDOW_GEOMETRY.md).

`Compositor::icon(name, component)` remains a separate semantic icon registry for compositor-wide
artwork such as cursor or legacy fallback names. It is not where `EasyWindowFrame` receives its
button icons. Likewise, `.background(...)` takes a widget because it describes pixels behind client
windows; focus, placement, security, workspace, and activation decisions remain host policy.

## The window-frame contract

`WindowFrame` is an overlay stack specialized by a type-state contract. It cannot become a `View`
until exactly one `WindowContentSlot` has been supplied. All visuals remain ordinary composition:

- `BoxDecoration` owns reusable background, per-side `Border`, `Outline`, `CornerRadii`, and up to
  two `Shadow` values. The same values style containers, controls, popups, and frames.
- Normal layout owns title-bar height, control size, gaps, padding, title placement, icon placement,
  and resize-region placement.
- `WindowChromeViewExt` adds only meaning: frame title, app icon, drag region, resize edge, system
  menu, built-in `WindowAction`, or declaration-authorized `ShellActionId`.
- Default precedence is control/action over resize over drag. `window_hit_priority(...)` can
  override it, while `window_hit_slop(...)` enlarges input geometry without changing layout or
  paint. `PointerViewExt::pointer_icon(...)` can override the semantic cursor.
- `WindowEdgeMask` and `WindowTilingState` describe tiled adjacency and the subset of edges that
  remain resizable.

`EasyWindowFrame` uses the actual rounded border as its resize target, excluding the inner contour
from both edge and corner handles. `resize_edge` is the minimum total grab thickness, including the
visible border; any extra width extends outward. Per-side outside tolerance is
`max(resize_edge - frame_border_width, 0) + resize_hit_slop.side`. The Wayland host checks published
resize targets before rejecting points outside the window; arbitrary outside controls are not
activated. Four edge and four corner regions intersect the shared rounded band; corner spans cover
the radius and are bounded to their window quadrant. App/title pixels inside the inner curve are
never captured by these resize regions. Disabled/tiled edges and control-over-resize priority remain
in force. Custom templates retain their authored rectangular regions unless they opt into a border
constraint.

`window_system_menu()` is the semantic hook for a project-defined menu. Telorgon's managed host
does not inject a stock menu, so the component's normal callback owns the menu content and behavior.

Corner radius, border, outline, and shadow are paint properties; they are not advertised to Wayland
clients. Wayland receives protocol globals, configure state, and icon-size preferences. The host
derives semantic hit regions from the final computed layout and performs the corresponding native or
xdg-toplevel operation.

For expert frames, implement `WindowFrameTemplate` as an ordinary named value. Closures still
implement the trait for compatibility, but are not required:

```rust,ignore
const PIN: ShellActionId = ShellActionId::named("window.pin");

#[component(no_default)]
struct StudioFrame {
    #[input]
    model: WindowChromeModel,
}

impl Component for StudioFrame {
    fn view(&self) -> impl View {
        window_frame()
            .gap(4.0)
            .child(
                row()
                    .height(42.0)
                    .child(text(&self.model.title).window_title())
                    .child(spacer())
                    .child(button("Pin").window_shell_action(PIN))
                    .child(button("Close").window_action(WindowAction::Close))
                    .window_drag_region(),
            )
            .child(
                stack()
                    .height(5.0)
                    .window_resize(WindowResizeEdge::Top)
                    .window_hit_slop(Insets::all(4.0))
                    .window_hit_priority(500),
            )
            .content_slot(window_content_slot().margin((42.0, 5.0, 5.0, 5.0)))
    }
}

struct StudioFrameTemplate;

impl WindowFrameTemplate for StudioFrameTemplate {
    type Component = StudioFrame;

    fn compose(&self, model: WindowChromeModel) -> StudioFrame {
        StudioFrame { model }
    }
}

fn pin_window(model: WindowChromeModel) {
    // The handler exists only because this action ID was authorized by the declaration.
}

Compositor::new()
    .window_frame(StudioFrameTemplate)
    .shell_action(PIN, pin_window)
    .background(DesktopBackground::default());
```

An unregistered shell action is inert. This keeps arbitrary frame regions from becoming an
unrestricted host-command channel while still permitting project-specific controls.

## Assets and icons

The generated catalog uses `include_bytes!`, normalized slash-separated keys, compile-time typed
handles (`IconAsset`, `ImageAsset`, `CursorAsset`, and `CursorThemeAsset`), and a validated immutable
bundle. SVG parsing disables file and string href resolution, and both encoded input and decoded
dimensions have hard bounds. Raster and SVG sources share a bounded decode/raster cache.

`Icon` is the target-neutral value for icon artwork. It converts directly into `image(...)`, the
composable `.icon(...)` button slot, legacy `IconButton`, `AppIconProfile`, managed Winit window
metadata, and compositor-owned chrome. `AppIconProfile` prefers an exact-sized source, then a
scalable source, then the nearest raster size.

Tinting treats the source as an alpha mask: opaque SVG artwork becomes the requested color while
transparent and antialiased edges retain their alpha. This makes a black SVG turn truly white,
rather than multiplying its existing RGB channels. The same contract works through software and
Vulkan rendering, native window icons, and compositor cursors. Tinting is available at the level
that owns the artwork:

```rust,ignore
const WHITE: ColorRgba8 = ColorRgba8::rgba(255, 255, 255, 255);

let standalone = image(assets::icons::APP).tint(WHITE);
let close = button("Close")
    .icon(assets::icons::CLOSE)
    .icon_tint(WHITE);
let native_icon = Icon::new(assets::icons::APP).tint(WHITE);
let arrow = CursorGraphic::new(assets::cursors::ARROW)
    .size(32)
    .hotspot(2, 2)
    .tint(WHITE);
```

Use `.without_tint()` (or `.without_icon_tint()` for a button) to return to the source colors.
Although the operation works for any decoded image, it is intended primarily for monochrome SVG
icons and cursors.

On Wayland, `xdg_toplevel_icon_v1` is commit-synchronized and follows the protocol's lifetime rules:

- only square `wl_shm` buffers with a positive scale are accepted;
- one size/scale pair replaces the earlier pair;
- an icon becomes immutable after `set_icon`;
- an empty assignment resets the toplevel icon;
- the committed snapshot survives destruction of the temporary icon object; and
- a referenced `wl_buffer` must remain alive until that icon object is destroyed.

The manager advertises preferred logical sizes 16, 24, 32, 48, and 64. The compositor selects the
closest submitted image for its frame, otherwise using the desktop environment's fallback
`AppIconProfile`.

## Cursor themes and pointer requests

`PointerViewExt::pointer_icon` selects a semantic role; `hide_pointer` explicitly hides the
cursor. Drag and resize regions select their corresponding roles automatically.

Desktop compositors require `.cursor_theme(CursorTheme)` and a background, in either order.
`CursorTheme::new()` returns a declaration, without `Default`, `build`, or a required manual
validation call. Desktop startup invokes compositor validation before entering the runtime.
Errors propagate through `.run()` to the program's `main` result.

```rust,ignore
fn cursor_theme() -> CursorTheme {
    CursorTheme::new()
        .size(24.0)
        .default_pointer(cursor(assets::cursors::ARROW).hotspot(6, 4))
        .pointer(cursor(assets::cursors::POINTER).hotspot(14, 5))
        .text(cursor(assets::cursors::TEXT).hotspot(16, 16))
        // Add every other role from REQUIRED_CURSOR_ROLES.
}

Compositor::new()
    .cursor_theme(cursor_theme())
    .client_cursor_mode(ClientCursorMode::Allow)
    .background(DesktopBackground::default());
```

The abbreviated theme above deliberately omits roles and will fail startup until completed.
Every one of the 36 supported roles requires an explicit assignment through its named method.
`CursorTheme` has no generic `.set(...)` method. Several roles may explicitly share artwork. The normal
`default_pointer` role never fills missing roles. There is no desktop system or generic-resize
fallback, nor a separate `.pointer(component)` artwork source.

`PointerGraphic` is renamed to `CursorGraphic`; `cursor(asset)` constructs one and is exported
through `telorgon::app::*`. Hotspots on desktop theme graphics use source-artwork coordinates.
Startup checks source bounds and equal animation frame dimensions, then scales hotspots with the
artwork. Callers must remove manual hotspot scaling from old cursor helpers.

The theme requires a finite positive `.size(f32)` in logical units (maximum 65535). A graphic may
set its own nominal `.size(...)`. `.cursor_size(f32)` on a compositor after `.cursor_theme(...)`
changes the effective base size proportionally: a 24-unit theme with a 30-unit text graphic becomes
32 and 40 units respectively. This is a declaration modifier; it does not introduce a live runtime
control handle. Output density determines raster dimensions; integer presentation geometry rounds
logical sizes and hotspots to the nearest supported unit. Tint and animation remain available.

`CursorTheme::from_asset(asset)` defers manifest loading until startup using the desktop's shared
asset catalog. Manifests require `size` and all role tables with `asset` and optional `hotspot`,
`size`, or animation frames. Fallback declarations are rejected. Code setters can customize loaded
entries before validation. Missing roles are reported together, and asset errors include role and
asset names.

Explicit hiding and allowed client surfaces remain supported. `ClientCursorMode::ThemeOnly` uses
compositor artwork, while `Allow` honors client artwork with its protocol-defined sizing.
Ordinary GUI applications retain optional `PointerThemeOverrides` and asset-theme configuration;
they do not inherit the compositor's complete-theme requirement.

## Vocabulary and compatibility

The normalized vocabulary is `BoxDecoration`, `box_style`, `corner_radius`, `uniform_border`,
`WindowFrame`, `WindowContentSlot`, `WindowFrameTemplate`, `EasyWindowFrame`,
`WindowChromeDesign`, `WindowChromeModel`, `WindowAction`, `PointerIcon`, `AppIconProfile`, and
`Compositor::background`. `Compositor::policy` remains a deprecated compatibility alias. Earlier
`style`, `radius`, and `border` composition builders remain deprecated forwarders. `TitleBar`
aliases are available alongside the earlier `Titlebar` spellings in the imperative shell API.
`ShadowFrame` remains a compatibility adapter only; new frames and ordinary containers use
`BoxDecoration::shadow` or `BoxDecoration::shadows`.

## Rejected alternatives and verification

- A single global frame component was rejected because title, activation, state, capability, and
  icon data are per toplevel.
- A mandatory fixed `WindowControls` composer was rejected. `EasyWindowFrame` is now an optional
  convenience layer with a complete design value; the low-level frame keeps normal composition.
- Window-only `Border` and `WindowShadow` values were rejected in favor of reusable box paint
  primitives.
- Raw string asset lookup at call sites was rejected in favor of typed generated handles.
- A general cursor-policy service was rejected in favor of immutable pointer configuration plus the
  narrowly scoped `ClientCursorMode` decision.
- Native frame geometry constants were rejected for custom frames; hit regions come from retained
  layout.

Portable tests cover asset-catalog generation, asset/profile validation, pointer precedence,
semantic pointer retention, window-frame type-state and hit regions, and exact protocol-profile
lookup. Host builds compile the Winit icon/frame/pointer integration, and an
`x86_64-unknown-linux-gnu` feature check compiles the Wayland protocol, compositor, and KMS path
without launching an application or server.

### Flush controls and the inner window curve

Easy-frame title-bar children are clipped by a full-window container inside the root border,
with radius `max(frame_radius - frame_border_width, 0)`. Square, full-height controls can therefore
sit flush against the right edge without covering the curved border. The contour uses the inner
window extent rather than the title-bar extent so a short bar does not flatten a larger radius.
Resize regions remain outside this visual-only container.

Reference review (2026-09-05): inspected Vello's `../other-rendering-libs/vello/vello/src/scene.rs`
(`push_clip_layer`) and Qt Quick's
`../other-rendering-libs/qtdeclarative/src/quick/scenegraph/coreapi/qsgnode.cpp` (`QSGClipNode`).
Both distinguish a subtree's clip geometry from a rectangular optimization. The
[Vulkan scissor specification](https://docs.vulkan.org/spec/latest/chapters/fragops.html#fragops-scissor)
confirms that scissoring only bounds a rectangle. Telorgon reuses its existing analytic rounded
clip in both renderers; no GPU ABI or backend lifetime changes are needed. Rejected alternatives:
rounding each button (changes shared-edge styling), clipping to the outer curve (allows border
coverage), or using a title-bar-sized rounded rectangle (changes the curve for short bars).
The CPU framebuffer regression checks a flush square button, curved and straight border pixels,
transparent outer corners, zero border width, and square windows. Hardware presentation remains
user-run.

### Border/fill antialias coverage (2026-09-05)

An opaque rounded box must stay opaque across its internal border/fill boundary. The software
renderer and Vulkan box shader now sum the premultiplied contributions of the fill (`inner`)
and border (`outer - inner`) before compositing the body once over shadows and the destination.
Treating those complementary areas as sequential source-over layers produced alpha 194/255 at
a fully covered corner pixel. Moving or expanding the clip would hide rather than repair this error.

Reference review: Qt's `../other-rendering-libs/qtdeclarative/src/quick/scenegraph/qsgbasicinternalrectanglenode.cpp`
partitions fill, border, and antialias geometry; Vello's
`../other-rendering-libs/vello/vello_shaders/shader/fine.wgsl` multiplies a source by area coverage
before source-over. The [Vulkan blending equations](https://docs.vulkan.org/spec/latest/chapters/framebuffer.html#framebuffer-blending)
explain why sequentially blending complementary coverage attenuates alpha twice. Telorgon retains
its existing analytic geometry and blending state and combines disjoint body coverage in the shader.
Tests cover all pixels of a rounded box with transparent, translucent, and opaque colors and local
opacity, plus the flush control corner regression. The offline shader builder regenerates and
validates SPIR-V, reflection, and manifest hashes; live GPU presentation is left to the user.

### Fractional rounded clipping (2026-09-05)

The retained rounded clip previously made a binary inside/outside decision while the frame border
used analytic antialiasing. A partially covered inner-curve pixel could remain fully opaque yet
contain no control color, exposing the title-bar fill as a visible fringe. Rounded clips now supply
fractional coverage to boxes, images, glyphs, and materials in both backends. The software path
uses the existing output-pixel rounded contour; Vulkan uses the same signed-distance/fwidth filter
as the analytic box edge. Rectangular scissors and geometric input containment are unchanged.

The reference invariants above still apply: Qt's `qsgbasicinternalrectanglenode.cpp` explicitly
models inner antialias geometry, and Vello's `fine.wgsl` applies area coverage before blending a
source. The [GLSL derivative specification](https://github.khronos.org/Vulkan-Site/glsl/latest/chapters/builtinfunctions.html)
defines `fwidth` as the pixel-local derivative magnitude used by the existing Vulkan box filter.
Expanding the clip radius was rejected because it changes geometry rather than restoring missing
fractional coverage. New checks require control color at the reproduced boundary pixel and compare
all four circular corners against independent coverage calculations at 1x, 1.25x, 1.5x, and 2x,
including fractional insets. Shader artifacts are rebuilt, reflected, validated, and hash-checked.
These are CPU pixel and shader-validation results; the reported live Vulkan appearance has not
been visually verified on the user's device.

### Rounded-clip derivative control flow (2026-09-05)

Windowed controls exposed diagonal seams after fractional rounded clipping was enabled. The
fragment shaders evaluated `fwidth` after a per-fragment clip-bounds return and after clipping
discards; image/glyph texture sampling also followed those discards. This violates the
[Vulkan derivative control-flow requirements](https://docs.vulkan.org/spec/latest/chapters/shaders.html#shaders-derivative-operations).
The rounded-distance calculation now runs without the bounds return, and all explicit derivatives
and implicit-derivative texture samples finish before any discard. The signed distance itself
rejects pixels outside rounded bounds. Clip-mode branches remain per primitive. Rectangular
clipping, geometry, `Dimension::FILL`, and the shader ABI are unchanged.

Reference review: Qt Quick revision `3e2d6bd456a8e850bcf641de77d1d5d8bc8419ef`,
`../other-rendering-libs/qtdeclarative/src/quick/scenegraph/shaders_ng/distancefieldtext_fwidth.frag`
(`main`), samples and computes derivatives before applying coverage. Zed revision
`f4178619acd0d47ea1f76a2025c42962c6d6638c`,
`../other-rendering-libs/zed/crates/gpui_wgpu/src/shaders.wgsl` (`fs_path_rasterization`), computes
coordinate derivatives before its clip return. Both preserve neighboring/helper invocations for
derivative calculation. Telorgon retains its distance filter and source-over blending; increasing
button height or overlapping triangles was rejected because neither corrects divergent shader
execution. No reference code was copied.

The layout regression includes odd and fractional title-bar heights in normal and maximized
states. The opt-in Vulkan regression checks every fully covered pixel of three flush controls
over a green backing, including their triangle diagonals and straight top/bottom edges, at odd/even
window widths, fractional insets, square/rounded clips, and 1x/1.25x/1.5x/2x target scales.
It is compiled only during this task; hardware execution and visual confirmation remain user-run.
The offline shader builder compiles, validates, reflects, and regenerates the shipped artifacts.

### Application-owned headers inside compositor frames

`WindowChromeModel::title_bar_visible` separates title/control ownership from outer appearance.
Custom frame templates should omit title bars, move regions and window-control buttons when it is
false, leave the content slot the full inner frame area, and keep their outer border, radius, colors,
shadows and resize regions. `EasyWindowFrame` does this automatically, including rounding all inner
content corners when the title bar is omitted. State-specific design rules (such as maximized border
width or radius) still apply. The desktop currently uses this mode for managed X11 clients requesting
client decorations; fullscreen and unmanaged popup windows do not receive an outer frame.

See [Desktop window motion](WINDOW_MOTION.md) to enable `WindowMotion::smooth()` and customize
maximize, minimize, and resize-content fades on the chrome design.

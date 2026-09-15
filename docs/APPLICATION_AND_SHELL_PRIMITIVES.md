# Telorgon Application and Shell Primitive Contract

## Status and authority

This is a **target implementation contract** produced by planning Gate 8. It fixes the cut between
shared foundation mechanisms, application primitives/components, shell primitives/components, and
host policy. It also fixes the baseline behavior, accessibility semantics, controllers, adaptive
rules, source ownership, implementation order, and acceptance cases for those domains.

It does not claim these packages or APIs exist in the current repository. Current capability status
remains in [Implementation status](IMPLEMENTATION_STATUS.md), and the current builder/control set
remains in [Current mounted UI](UI_WIDGETS.md).

[Gate 7](AUTHORING_AND_COMPONENT_RUNTIME.md) controls component identity, state, reads, actions,
transactions, reconciliation, tasks, and editable-text storage. This document builds on that one
runtime. [Gate 9](PLATFORM_INTEGRATION_CONTRACT.md) defines platform lifecycle, input/service
adapters, accessibility export, native IME, clipboard, drag/drop, haptics, and window-system
integration. A Gate 9 adapter may supply
capabilities to these components but may not create a second component or control model.

The Rust names below are the initial implementation target. Small compiler-driven spelling changes
are allowed, but moving behavior or policy across the package boundaries requires updating this
contract first.

## 1. Decisions fixed by Gate 8

1. Telorgon exposes exactly two public design domains: **application** and **shell**. They share
   foundation implementations but neither domain depends on the other.
2. The public layers are distinct: foundation mechanisms, domain primitives, standard components,
   and composed facilities. A visually simple control is not automatically a primitive.
3. Boxes, text, layout, focus, pointer/gesture routing, semantics, portals, scrolling mechanics,
   editable-text mechanics, and external-content placement are shared foundation mechanisms.
4. Application primitives express application/game coordinate regions and hosted content. Standard
   controls such as buttons, fields, menus, lists, tables, docks, and scaffolds are application
   components composed from foundations and application primitives.
5. Shell primitives express outputs, ordered shell layers, external client surfaces, input/reserved
   geometry, and chrome interaction regions. Taskbars, titlebars, launchers, workspace views, and
   notification centers are shell components.
6. Semantic values are controlled inputs: components receive `Read<T>` and emit typed requested
   changes. They do not silently mutate parent state. Complex interaction protocols use focused,
   owner-scoped controllers rather than a general two-way binding system.
7. Transient interaction state such as hover, armed press, focus-visible, active drag, and menu
   highlight is component/runtime state. It is not application business state and is not encoded by
   rebuilding the component.
8. Every interactive standard component has a defined pointer, keyboard/directional-navigation,
   cancellation, disabled/read-only, focus, semantics, and action contract before it is styled.
9. Focus and selection are separate. `Tab`/reverse-`Tab` move between components; directional keys
   normally move within composite components. Selection follows focus only where the component
   contract explicitly enables it.
10. Adaptation follows available space, text scale, direction, accessibility preferences, and
    currently available input capabilities. It does not branch on an operating-system name.
11. Every component emits a retained semantic description from its meaning and state. Semantics are
    never reverse-engineered from paint, and an icon-only action requires an accessible name.
12. One overlay/focus mechanism implements popups, menus, dialogs, sheets, tooltips, drag previews,
    and notifications. Application and shell domains expose different policy-bearing wrappers over
    that mechanism.
13. `render_target_view`, `video_surface`, and `client_surface` place host-owned content without
    acquiring devices, submitting work, presenting, polling, or copying pixels to the CPU.
14. Shell models and requests are protocol-neutral. The host owns display protocols, surface
    validity, window/focus policy, authorization, and command execution.
15. Component catalogs are delivered in explicit tiers. A long aspirational catalog cannot delay a
    complete accessible baseline or be reported as implemented because a constructor name exists.
16. No primitive/component starts a server, event loop, renderer thread, sleeping timer thread, or
    background executor. Scheduling uses Gate 7 runtime deadlines/tasks and Gate 9 host capabilities.

## 2. Layer and ownership test

### 2.1 The four layers

| Layer | Test | Examples | Owner |
| --- | --- | --- | --- |
| Foundation mechanism | Useful unchanged in both domains and has no product vocabulary | box, row, text, focus scope, scroll mechanics, semantic node, portal | focused core UI/input/layout/text/accessibility packages |
| Domain primitive | Irreducible geometry/content/interaction concept of one domain | render target, world anchor, output layer, client surface, resize region | corresponding primitive package |
| Standard component | Accessible behavior assembled from mechanisms/primitives and styled as one control | button, checkbox, field, menu, titlebar, taskbar | corresponding component package |
| Facility | Several components plus a controller/model and coordinated behavior | adaptive scaffold, data grid, dock area, workspace overview, notification center | corresponding component package |

A type belongs at the lowest layer that can express its complete stable meaning without importing
business or host policy. It is promoted upward when it owns selection rules, command routing,
validation, overlay policy, coordinated focus, or a user-facing style contract.

The following are therefore **not** baseline primitives: button, checkbox, radio, slider, text
field, combo box, tabs, menu, table, tree, dialog, dock panel, titlebar, taskbar, launcher, and
workspace switcher. Their paint may be analytic and cheap, but their behavior is component-level.

### 2.2 Foundation exposure

Foundation implementations remain in `telorgon-ui`, `telorgon-layout`, `telorgon-input`, `telorgon-text`,
and `telorgon-accessibility`. Advanced users may import their focused modules. Ordinary users receive
the appropriate constructors and value types through exactly one selected domain prelude:

```rust,ignore
use telorgon::application::prelude::*;
// or
use telorgon::shell::prelude::*;
```

There is no third umbrella “widgets” domain. Re-exporting the same foundation type through both
preludes does not duplicate its implementation or runtime identity.

### 2.3 Dependency direction

```text
core / ui / input / layout / text / accessibility / theme / runtime
                 ^                              ^
                 |                              |
     primitives-application          telorgon-shell models/requests
                 ^                              ^
                 |                              |
     components-application             primitives-shell
                                                ^
                                                |
                                       components-shell
```

The application branch never imports `telorgon-shell` or either shell domain package. The shell
branch never imports application primitives/components. A deliberately combined product assembles
both branches above them.

## 3. Public authoring contract

### 3.1 Curated preludes

`telorgon::application::prelude` exports:

- `Component`, `State`, `Read`, application `Ui` extension traits, common layout/content values;
- Tier A application primitives/components and their typed actions/controllers;
- application environment, command, selection, overlay, text-field, and navigation values; and
- no shell surface, shell request, Winit, renderer, or graphics-API type.

`telorgon::shell::prelude` exports:

- the same component/runtime and curated foundation values;
- shell primitive/component extension traits, stable shell model IDs, and typed shell requests; and
- no application editor/game component or display-protocol/native-handle type.

Large expert types, implementation traits, diagnostic internals, raw node/property handles, and
backend interop remain in focused modules instead of either prelude.

### 3.2 Mount-time extension traits

Domain methods extend Gate 7's mount-only `Ui<'_, Action>`:

```rust,ignore
pub trait ApplicationUiExt<A> {
    fn button(
        &mut self,
        label: impl Into<Label>,
        map: impl Fn(Activation) -> A + 'static,
    ) -> ButtonRef;
    fn checkbox(
        &mut self,
        label: impl Into<Label>,
        value: Read<CheckState>,
        map: impl Fn(CheckState) -> A + 'static,
    ) -> CheckboxRef;
}

pub trait ShellUiExt<A> {
    fn client_surface(
        &mut self,
        surface: Read<ClientSurfaceSnapshot>,
        map: impl Fn(SurfaceRequest) -> A + 'static,
    ) -> ClientSurfaceRef;
}
```

The exact returned reference types are focused advanced handles, not mutable component objects.
They may expose diagnostic identity and safe property bindings but cannot bypass the owning
component transaction.

Convenience methods mount ordinary components; they do not put their implementations into
`telorgon-runtime::Ui` or rerun the caller's mount function on updates.

### 3.3 Controlled semantic values

The standard input direction is:

```text
parent Read<T> -> component
user/semantic input -> typed requested T -> parent Action
parent transaction -> parent State<T>
```

For example, a checkbox derives the requested next `CheckState` and emits it. It does not change the
authoritative checked value before the parent transaction accepts it. The component may show
short-lived pressed feedback, but the checked semantic/visual state follows its input.

Components that can validly reject or normalize a value emit a typed change containing the proposed
value and source. They never report a value as committed before the controlling state changes.

### 3.4 Focused controllers

Controllers exist only when several operations must share durable protocol state or identity. The
names below are the application-domain public controllers; shell facilities use focused
shell-specific controllers or private component state over the same foundation transition engines:

| Controller/model | Owns | Does not own |
| --- | --- | --- |
| `TextController` | revisioned buffer, selection, composition, edit history policy, editor session | application document persistence or platform IME object |
| `ScrollController` | offset/anchor, extents, activity, reveal requests | layout engine or background animation thread |
| `SelectionModel<K>` | selected keys, anchor, focus-selection policy | collection data or row components |
| `OverlayController` | open generation, anchor, dismissal state, focus restoration | native window or event loop |
| `NavigationController<R>` | route stack/selection and restoration keys | URL/platform navigation service |
| `DockController<P>` | pane tree, active pane, split/tab placement requests | editor document model or host window policy |

Controllers are owner-scoped, generational, non-`Send` UI-runtime values created through a typed
create context. Mutation occurs through an authorized update context or controller-specific action,
and reads are dependency tracked. They are not a general writable signal escape hatch.

Controller wrappers live in the domain that exposes them. Their reusable algorithms and data—text
edits/history transitions, scroll physics/anchors, selection operations, overlay placement/lifecycle,
and focus movement—live below both domains. A shell launcher does not import application
`TextController`, but it also does not implement a second text editor engine.

Simple controls do not require controllers. A button emits an action; a checkbox consumes a read;
a progress indicator is read-only.

### 3.5 Common output values

Reusable value types preserve interaction meaning without forcing a global event enum:

```rust,ignore
pub enum ChangePhase { Begin, Update, Commit, Cancel }
pub enum ChangeSource { Pointer, Keyboard, Directional, Accessibility, Programmatic }

pub struct Activation {
    pub source: ChangeSource,
}

pub struct ValueChange<T> {
    pub value: T,
    pub phase: ChangePhase,
    pub source: ChangeSource,
}

pub enum DismissReason {
    Accepted,
    Cancelled,
    Escape,
    OutsidePress,
    AnchorRemoved,
    FocusLost,
    Replaced,
}
```

Component-specific outputs remain component-specific (`TextChanged`, `MenuChosen<Id>`,
`SurfaceRequest`, and so on). Actions do not require `Clone` or `Send`.

## 4. Environment, density, and adaptation

### 4.1 Neutral environment

Each view exposes dependency-tracked neutral environment reads:

- available logical size and local component constraints;
- device scale and logical density class;
- text scale, locale, writing direction, and preferred reading order;
- safe-area/inset and occlusion values;
- available pointer kinds, hover, keyboard, directional controller, and text-input capabilities;
- reduced-motion, increased-contrast, color-scheme, and focus-indicator preferences; and
- view active/focused/visible state needed to pause nonessential motion.

Gate 9 adapters produce these values. Components consume them without importing a platform enum.
Input capabilities are a set, not a single permanent “mobile” or “desktop” mode; a touchscreen
laptop may have touch, mouse, keyboard, and controller active together.

### 4.2 Density and targets

The standard theme provides three density profiles:

| Profile | Baseline minimum interactive target | Intended use |
| --- | ---: | --- |
| `Compact` | 24 logical px on each axis | pointer-dense tools where touch is not claimed |
| `Standard` | 32 logical px on each axis | mixed desktop application UI |
| `Touch` | 44 logical px on each axis | touch-first or accessibility-enlarged UI |

Visible artwork may be smaller than its interaction target. Expanded hit bounds must not create
ambiguous overlap; when two targets would overlap, layout spacing or an explicit hit-priority rule
must resolve it. Themes may enlarge these minima. A selected accessibility/platform profile may set
a stricter minimum, which components cannot theme below.

Dragging is never the only way to perform a nonessential action. Sliders have keyboard/semantic
increment actions; splitters and dock rearrangement expose step/menu alternatives; list reorder has
move-before/move-after actions where reorder is offered.

### 4.3 Adaptive composition

Components adapt from local constraints and environment reads. Baseline width classes are named
`Compact`, `Medium`, and `Expanded`; their default thresholds are theme/profile values rather than
public constants applications must hard-code.

An adaptive facility retains one logical slot owner while changing its placement. It uses layout,
portals, or an explicit keyed branch so content state does not disappear merely because a rail
became a bottom bar or a split view became a route. If two variants genuinely have different
semantics, the structural replacement is explicit and documented.

Text scaling may force reflow, wrapping, scrolling, or a larger layout class. It may not clip labels,
hide focused content, or replace a text label with an unlabeled icon solely to preserve density.

## 5. Universal interaction and semantic behavior

### 5.1 State ownership

| State | Canonical owner | Meaning |
| --- | --- | --- |
| `disabled` / `read_only` | component input | unavailable versus viewable/noneditable |
| `hovered` | input processor | eligible pointer currently within hover region |
| `armed` / `pressed` | component default behavior | activation is in progress and may still cancel |
| `focused` / `focus_visible` | focus processor | keyboard/assistive focus and whether indicator is required |
| `checked` / `selected` / `expanded` / `invalid` | controlled semantic input | application-visible value/state |
| `highlighted` | composite controller | current navigable menu/list candidate, distinct from focus/selection |
| `dragging` / `scrolling` | gesture/controller | active operation with begin/update/commit/cancel phases |
| `busy` | component input | operation in progress; not synonymous with disabled |

Themes resolve these states to paint/motion. Hover, pressed, and focus styling must not change the
measured border box of a standard component; reserving focus-ring/border space prevents layout
jitter. Structural states such as expanded content change layout through explicit component logic.

### 5.2 Activation state machine

Standard buttons, toggle controls, menu items, and action rows use one baseline activation model:

```text
Idle
  -- eligible primary pointer down --> Armed(pointer, capture)
  -- Space key down ----------------> Armed(keyboard)

Armed
  -- leave/re-enter ----------------> visually disarmed/rearmed, capture retained
  -- eligible pointer up inside ----> Activate once -> Idle
  -- Space key up while eligible ---> Activate once -> Idle
  -- cancel/disable/unmount --------> Cancel -> Idle/Dead

Idle -- nonrepeat Enter key down ----> Activate once
Idle -- semantic Activate ----------> Activate once
```

Secondary pointer activation never invokes the primary action unless the component explicitly maps
it. Long press, double activation, and context menu are distinct recognizers; recognizing one cannot
also emit an accidental normal activation. Pointer cancellation, lost capture, view deactivation,
or unmount clears armed state without action.

The component emits its typed action inside the input transaction after default behavior has
validated eligibility. It does not call user component code inline during hit traversal.

### 5.3 Focus and composite navigation

- `Tab` and reverse-`Tab` traverse top-level focusable components in canonical layout/reading order.
- A composite contributes one stop to that traversal unless its documented pattern requires more.
- Directional keys, `Home`, `End`, and typeahead navigate inside menus, radio groups, tabs,
  listboxes, trees, grids, toolbars, and similar composites.
- Focus, active descendant/highlight, and selected keys are distinct stored values.
- On re-entry, a composite restores the last valid focused key, otherwise its selected key, otherwise
  the first enabled key according to its pattern.
- Removing the focused item chooses the nearest valid successor, then predecessor, then the composite
  itself; it never sends focus to a reused generation.
- Left/right directional meaning follows writing direction for start/end navigation. Physical window
  resize edges and game-world axes do not mirror unless their own contract says so.
- Focus indicators are visible for keyboard/directional/assistive navigation and may remain visible
  when a platform preference requires them. Pointer use does not permanently disable them.

Disabled controls normally leave the outer tab sequence. Disabled items inside menus, toolbars, and
other discoverability-sensitive composites may remain directionally focusable but cannot activate;
the component pattern fixes this rather than each theme improvising.

### 5.4 Semantics contract

Every standard interactive component must provide:

- a neutral role and stable semantic identity;
- accessible name and, where useful, description/help/error relationship;
- enabled/read-only, checked/pressed/selected/expanded/invalid/busy state as applicable;
- current value, value text, bounds, collection position, and relationships as applicable;
- every operation available through pointer-only presentation as a semantic action; and
- focus/selection updates independent of paint visibility or virtualization.

Visible text is the default accessible name. An explicit name is required for icon-only or
custom-painted actions, and the visible label text must remain included in the accessible name.
Decorative icons, separators, shadows, and duplicate labels do not become separate semantic nodes.

A standard component owns its semantic merge/exclusion rules. Custom canvas/path content is
semantically empty until the author supplies semantic children. Virtual collections report stable
keys plus known count/index/level/set-size metadata; offscreen items can be represented lazily
without mounting their visual subtrees.

Live announcements are explicit, priority typed, coalesced, and redacted for secure content. A toast
or progress update does not automatically steal focus.

### 5.5 Reduced motion and time behavior

Motion communicates state but never owns it. Reduced-motion mode removes nonessential travel,
bounce, parallax, and repeated animation while preserving an immediate or short opacity/state
transition. Cursor blink, indeterminate progress, tooltip delay, long-press delay, and submenu delay
use scheduler deadlines; they do not create one thread or task per component.

## 6. Commands, shortcuts, validation, and undo

### 6.1 Commands

A reusable command is typed and stateful without storing a cloneable application action:

```rust,ignore
pub struct CommandSpec<Id, A> {
    pub id: Id,
    pub label: Label,
    pub description: Option<Label>,
    pub icon: Option<IconId>,
    pub enabled: Read<bool>,
    pub checked: Option<Read<CheckState>>,
    pub shortcuts: ShortcutSet,
    pub invoke: ActionFactory<A>,
}
```

`ActionFactory<A>` is an owner-scoped callable that creates a fresh moved `A` for each invocation;
it does not retain one action value for reuse and does not impose `A: Clone`.

Menus, toolbars, command palettes, and context menus consume the same command specs. Invocation
creates one moved action with a `ChangeSource`; it is not a string event bus. Shortcut resolution is
scope ordered, ignores disabled commands, reports ambiguous equal-priority bindings, and displays
the effective binding separately from physical key matching. Gate 9 maps platform key conventions.

### 6.2 Forms and validation

Fields expose value/change, label, help, required, read-only, enabled, and validation inputs.
Validation results are typed (`Valid`, `Warning`, `Invalid`, `Pending`) and associated with the field
semantics. Invalid submission focuses/reveals the first invalid field by canonical form order and
also exposes a summary action; validation does not rely on color alone.

`numeric_field` preserves a valid intermediate editing string such as `-` or `1.`. It emits a typed
numeric commit only after parsing/constraints accept it, while its controller can expose transient
parse state. It never rewrites the user's text on every key press to force a formatted number.

### 6.3 Basic text undo

`TextController` owns an optional bounded `EditHistory` for standard fields/areas:

- one paste, cut, drop, replacement, or committed composition is one undo unit;
- adjacent compatible typing/deletion may merge until a nonmatching edit, explicit boundary,
  focus/session boundary, or configured deadline;
- programmatic replacement is a separate unit unless explicitly marked as history reset;
- selection-only changes are restored when required by the associated edit but do not create text
  units by themselves; and
- secure fields may disable history or retain it only in redacted protected memory by policy.

The controller exposes typed undo/redo commands and availability reads. Rich-document transactions,
multi-cursor history, collaborative transforms, and application-wide command history remain later
editor extensions.

## 7. Shared foundation mechanism inventory

These are implemented once below both domains. “Foundation” describes ownership, not a third
ordinary-user catalog.

| Family | Mechanisms |
| --- | --- |
| Structure/layout | fragment, box, row, column, flex, grid tracks, stack, positioned, aspect ratio, intrinsic constraints, spacer, separator |
| Scrolling/virtualization | scroll viewport/content transform, scrollbar semantics, reveal, virtual range, keyed materialization, cache budget |
| Content | shaped text, rich spans, image, icon, path, cached canvas, material/effect reference, external-content slot |
| Editing | editable text surface, caret/selection/composition paint, text input session attachment |
| Input | pointer region, gesture arena participant, action region, focusable, focus scope, shortcut scope, drop target |
| Semantics | role/state/value/action node, merge/exclude, collection metadata, live region, relation IDs |
| Overlays | overlay root, portal, anchor geometry, modal barrier, focus containment/restoration, placement solver |

`scroll_view`, `virtual_list` mechanics, and `editable_text` are advanced foundation facilities
because both domains need identical offset, materialization, and text-session behavior. Ordinary
application users normally choose `ScrollView`, `ListView`, `TextField`, or `TextArea`; shell users
normally choose a shell component that wraps the same mechanism.

Foundation mechanisms contain no theme-specific button appearance, taskbar policy, application
route, game HUD policy, or display protocol.

Behavior transition modules at this layer are callable by both domains without mounting a control.
They accept neutral state/input and return typed effects/actions; they do not call domain component
code, access a platform service, or own an event loop.

## 8. Application domain

### 8.1 Application primitives

The application primitive package adds only domain-specific irreducible concepts:

| Primitive | Contract |
| --- | --- |
| `application_root` | establishes application theme/environment/overlay/command scopes for one view |
| `content_region` | named primary content landmark and adaptive slot |
| `navigation_region` | named navigation landmark and adaptive slot, without choosing a navigation component |
| `status_region` | nonmodal application status landmark/live-output boundary |
| `hud_layer` | application/game overlay coordinate layer with explicit hit pass-through policy |
| `viewport_overlay` | content anchored to a host viewport region without owning that viewport renderer |
| `world_anchor` | consumes a host-provided projected transform/visibility/depth hint; never reads a game camera or GPU |
| `render_target_view` | displays a host texture/target token using the render-area contract without submission/presentation |
| `video_surface` | displays a revisioned host media surface with fit/color/protection metadata |

`window_content` and `safe_area` are conveniences over the root/foundation environment. They do not
create a native window. `path`, `canvas`, image, text, and ordinary layout remain foundations.

### 8.2 Tier A — accessible application baseline

Tier A is the first support claim. It must be complete before broad component names are added:

| Family | Components/controllers |
| --- | --- |
| Actions | button, icon button, link action, toggle button |
| Choice | checkbox, radio group/item, switch |
| Range/status | slider, progress indicator, activity indicator, meter |
| Text | label, selectable text, text field, search field, text area, `TextController` |
| Content/layout | image view, scroll view, scrollbar, separator, split view, `ScrollController` |
| Collections | list view, virtual list, listbox, `SelectionModel<K>` |
| Commands | command, shortcut scope, toolbar, menu button, menu, context menu |
| Navigation | tabs/tab panels, breadcrumb, navigation rail/bar, `NavigationController<R>` |
| Overlays | popup, dialog, sheet, tooltip, toast, overlay host/controllers |
| Structure | application root, scaffold, form, content/navigation/status regions |

Tier A supports keyboard, mouse, touch, directional navigation, semantics, text scaling, reduced
motion, compact/standard/touch densities, and headless behavior tests.
It becomes an operational catalog only after implementation slices 1–4 pass together. Earlier
slices are reported by family; they are not a Tier A support claim.

### 8.3 Action and choice behavior

| Component | Controlled input | Output/default behavior | Semantic role/state |
| --- | --- | --- | --- |
| Button | enabled/busy | activate once; busy may remain focusable and suppress duplicate activation by policy | button, name, enabled/busy |
| Toggle button | `Read<bool>` | request inverse on activation; label does not change merely to describe state | toggle button, pressed |
| Checkbox | `Read<CheckState>` | Space/activation requests configured next state; mixed cycle is explicit | checkbox, checked/mixed |
| Radio group | `Read<Option<K>>` | one tab stop; arrows move/select enabled item; Space selects focused item | radio group/items, selected |
| Switch | `Read<bool>` | tap/Space requests inverse; drag is optional enhancement with cancel | switch, checked |
| Link action | destination/action | activate/navigation request; context copy/open operations are commands | link, destination/name |

`CheckState` is `Unchecked`, `Checked`, or `Mixed`. A two-state checkbox never produces `Mixed`.
For a tri-state aggregate, the requested cycle is supplied explicitly; Telorgon does not assume
whether `Mixed` goes to checked or unchecked.

Radio selection is independent from focus in the stored model even when the default single-select
keyboard policy makes selection follow focus. Removing the selected item emits no invented
replacement unless the configured selection policy requests one.

### 8.4 Range and status behavior

`Slider<T>` uses a finite ordered `RangeModel<T>` containing minimum, maximum, step/page step,
formatting, and optional marks. Pointer drag emits `Begin`, bounded/coalesced `Update`, then `Commit`;
cancel emits `Cancel` and the controlling model decides whether to restore. Arrow keys increment,
page keys use the page step, and Home/End request bounds. Direction follows orientation, writing
direction, and explicit reversal.

A range slider has two stable thumb identities. Thumbs cannot cross unless its declared model
allows role swapping; otherwise values clamp while focus remains with the active thumb. Each thumb
has independent semantic increment/decrement/value actions.

Progress and activity indicators are read-only. Determinate progress reports bounded value text;
indeterminate activity reports busy without fabricating a percentage. High-frequency progress
updates coalesce and live announcements are rate limited by policy.

### 8.5 Text components

`TextField` and `TextArea` require a `TextController`, label, and mode. The controller is the single
editing value; passing a changing `Read<String>` that replaces user edits is not the default API.
Programmatic synchronization is an explicit revision-checked controller edit.

The components provide:

- caret, selection, composition, horizontal/vertical reveal, pointer/keyboard selection, and
  semantic text actions;
- pure input filtering and display transformation with source-to-display offset mapping;
- submit/action behavior separated from newline insertion;
- typed `TextChanged`, `SelectionChanged`, `Submitted`, and `EditRejected` outputs;
- label/help/error relationships and read-only versus disabled behavior;
- secure mode with redacted diagnostics/semantics/capture policy and no accidental text exposure; and
- selection/context command availability without directly calling a platform clipboard.

`SearchField` composes a text field with clear/submit semantics. `NumericField` composes the parsing
contract in section 6.2. A later document editor may reuse the same controller/storage but adds rich
spans, multi-cursor, syntax, and large-document facilities separately.

### 8.6 Menus, popups, and dialogs

One mounted `OverlayHost` per view owns ordered overlay entries. Opening returns a generational entry
and records the anchor, focus restoration target, modality, dismissal policy, placement candidates,
and safe bounds. Removing the anchor closes its dependent overlays with `AnchorRemoved`.

Popup placement tries declared start/end/above/below candidates, respects writing direction and
safe/occluded bounds, then shifts/resizes/scrolls according to typed overflow policy. Placement is
recomputed only when anchor, content size, scale, or usable bounds change.

Menus are composite command views:

- opening by keyboard focuses the selected or first enabled item;
- arrows/Home/End and locale-aware typeahead move highlight/focus;
- submenus have explicit parentage and delayed hover opening with cancellation;
- activation closes the configured menu chain before enqueueing the command action;
- Escape closes one menu level and restores the appropriate parent/anchor focus; and
- disabled items may remain directionally discoverable but never invoke.

A modal dialog makes content behind its barrier inert for pointer, focus, and semantics. It contains
Tab traversal, chooses an explicit initial focus policy, exposes a visible/semantic close path, and
restores focus to the opener or nearest live fallback on dismissal. Outside press does not dismiss a
destructive/critical dialog unless explicitly enabled. A nonmodal popup does not claim modal
semantics.

Tooltips never become the only source of an accessible name. They open from hover or sustained
focus after a deadline, close on Escape/pointer departure/focus loss according to policy, remain
readable under text scaling, and do not take focus. Toasts/notifications do not take focus and use
explicit live-region priority.

### 8.7 Collections and selection

The public collection distinctions are semantic, not merely visual:

| Component | Use |
| --- | --- |
| `ListView` | ordinary rows that may contain independently interactive descendants |
| `ListBox<K>` | options with one composite focus/selection model; option descendants are not separate controls |
| `Table` | noninteractive tabular relationships with headers |
| `DataGrid<R,C>` | cell/row navigation, selection, editing, sorting, and column operations |
| `TreeView<K>` | hierarchical expand/collapse, level/parent/set metadata, selection |
| `TreeGrid<R,C>` | hierarchical rows plus grid navigation/editing |

Every data item has an explicit stable key. `SelectionModel<K>` supports `None`, `Single`, or
`Multiple` mode plus an anchor and a declared selection-follows-focus policy. Multi-selection never
silently collapses because focus moved.

Virtualization preserves component/controller/semantic identity by key, measures only required
ranges plus a bounded cache, and reports total/visible semantic metadata when known. Focus/reveal
requests can materialize a keyed offscreen item without linearly mounting every preceding child.
Unknown/infinite collections report that status rather than a false count.

Tree right/start opens or descends and left/end closes or ascends according to writing direction and
orientation. Data-grid navigation hands keys to an editing child while edit mode is active and
returns them to cell navigation on commit/cancel. Pointer drag selection/reorder always has
keyboard/semantic alternatives.

### 8.8 Navigation and adaptive application structure

Tabs separate focused tab, selected route, and panel state. Directional keys move among tabs; the
default activates immediately only when panel presentation is local and latency-free. Otherwise
activation requires Enter/Space. Tab panels retain state by route key according to the navigation
controller's explicit keep-alive budget.

`AdaptiveScaffold` provides named navigation, top/toolbar, content, secondary, status, floating
action, and overlay slots. Constraint/environment changes reposition the same slot owners rather
than remounting business components. It may replace navigation rail with navigation bar, split
content with a route, or sheet with popover using explicit adaptive policies.

`SplitView` exposes keyboard/semantic resize steps, collapse/restore, minimum sizes, and a
`ScrollController`-independent divider. Resizing is a phased change and cancellation restores the
controlled value.

### 8.9 Tier B and Tier C application catalogs

Tier B completes conventional desktop/mobile applications:

- combo box, autocomplete, spin/numeric field, date/time/calendar, segmented choice;
- table/data grid, tree/tree grid, pagination, outline, property grid;
- menu bar, command palette, richer toolbar/status bar, side bar;
- route host, master/detail, pagination, disclosure/accordion;
- notification host, validation summary, file/media presentation adapters; and
- high-level adaptive window/application scaffolds.

Tier C adds tool/game/real-time facilities:

- dock area/panel, inspector, timeline, console, tool palette, graph/performance view;
- document/editor surface extensions, property editors, viewport tool overlays;
- HUD layer, reticle, minimap, meter, hotbar, dialogue, game menu, world anchor; and
- render-target/video views and cached custom visualization.

Game/real-time components consume host snapshots and transforms. They do not poll engine state,
own the render loop, rebuild every frame, insert an independent submission, or make unchanged views
record work. HUD hit pass-through and viewport input capture are explicit per region.

## 9. Shell domain

### 9.1 Protocol-neutral model boundary

`telorgon-shell` owns stable, immutable, revisioned values exchanged with a policy host:

- output identity, usable/logical/physical geometry, scale, transform, color capability, and insets;
- surface identity, parentage, content revision, logical/buffer geometry, regions, opacity, transform,
  external-content token, protection metadata, and synchronization reference;
- workspace identity/order/name and host-provided surface membership/geometry;
- application/launcher entries and typed host action IDs;
- notifications, status indicators, media/session summaries, and extension entries; and
- seat/contact/source identity needed to qualify a user request without embedding a protocol serial.

These are snapshots of host truth. Shell components never discover processes, enumerate Wayland
objects, query network/power services, or invent window geometry by reading pixels.

Requests are divided by authority rather than one ever-growing untyped command enum:

```rust,ignore
pub enum SurfaceRequest {
    Activate { surface: SurfaceId, source: InputSource },
    Close { surface: SurfaceId },
    BeginMove { surface: SurfaceId, contact: ContactId },
    BeginResize { surface: SurfaceId, edge: ResizeEdge, contact: ContactId },
    SetMinimized { surface: SurfaceId, minimized: bool },
}

pub enum WorkspaceRequest { /* select, move surface, reorder, create/remove if capable */ }
pub enum OutputRequest { /* reserved-area proposal, appearance, mode action if exposed */ }
pub enum SystemRequest { /* launcher/status/notification action IDs supplied by host */ }

pub struct ClientInputRequest {
    pub surface: SurfaceId,
    pub event: SurfaceInputEvent, // neutral pointer/touch/wheel event in local coordinates
}
```

At the platform-host boundary each request is wrapped with its observed shell snapshot revision and,
when the native protocol requires input-event causality, Gate 9's opaque single-use
`UserGestureGrant`. Shell code cannot inspect, copy, persist, synthesize, or log a protocol serial,
seat token, or native handle. `ClientInputRequest` preserves the neutral lifecycle; the protocol host
performs serial/grab encoding only after validating that envelope.

The host validates every ID, generation, source, seat, capability, session/lock state, and policy
precondition before executing a request. Returning `Denied`, `Stale`, `Unsupported`, or a request ID
followed by Gate 9's terminal outcome is normal; visual state follows the next host snapshot rather
than optimistic policy mutation.

### 9.2 Shell primitives

| Primitive | Contract |
| --- | --- |
| `shell_root` | establishes one output's authorized layer tokens, shell theme, overlay, and focus domains |
| `output_view` | maps logical output coordinates, scale, safe/usable areas, and output identity |
| `shell_layer` | ordered background/workspace/panel/overlay/lock/cursor placement with capability token |
| `client_surface` | places one external surface revision and maps eligible input to typed requests |
| `surface_tree` | preserves parent/subsurface order, clip/opaque/input regions, transforms, and damage |
| `surface_placeholder` | explicit unavailable/protected/lost content presentation without stale pixels |
| `surface_snapshot` | host-authorized retained visual snapshot with revision/protection policy |
| `reserved_area` | proposes output space reservation; host snapshot decides the accepted usable area |
| `exclusive_region` | blocks lower shell hit routes in declared geometry, without claiming protocol authority |
| `surface_input_region` | maps output pointer coordinates to a surface-local eligible region |
| `drag_region` | emits begin-move request with contact/source; never moves the window directly |
| `resize_region` | emits edge/corner resize request; never applies window geometry directly |
| `output_edge` | stable edge/corner geometry for reveal, snap, reservation, and accessibility alternatives |

Only `shell_root` or the host can mint a token for privileged layers. An arbitrary extension cannot
paint in the lock/cursor/system-modal layer by naming an enum value.

### 9.3 Layer and input order

The baseline order per output is:

```text
cursor / trusted emergency affordance
lock or secure system-modal layer
system overlay (switcher, overview, OSD, critical dialog)
panel / notification / launcher layer
workspace chrome and client-surface trees
background
```

The active lock/system-modal layer makes lower pointer, keyboard, focus, and semantic routes inert.
This is necessary UI behavior but is not a security boundary by itself; the policy host and platform
must also prevent unauthorized composition/input/capture.

Shell UI focus and client-surface focus are distinct. Focusing a titlebar button does not implicitly
focus the client, and forwarding a pointer to a client does not traverse Telorgon component listeners
inside that client. The host decides focus transfer and returns its authoritative state.

Input delivered to a `client_surface` is clipped to its host-provided input region, transformed to
surface-local coordinates, and emitted as `ClientInputRequest` with stable source/contact/time
information. Raw pointer lifecycle is preserved unless an authorized shell gesture wins and emits
cancellation. Keyboard/text delivery follows host-owned client focus rather than hit testing through
the shell component. Protocol event encoding, serial validation, implicit grabs, and client delivery
belong to the host.

### 9.4 Shell components

| Family | Components | Behavior boundary |
| --- | --- | --- |
| Output/root | shell root, output background, output overlay host | layer assembly and output adaptation, not display-mode policy |
| Window chrome | window frame, titlebar, window controls, shadow frame, resize affordance, snap preview | emits typed requests; host supplies capabilities/state/geometry |
| Workspaces | workspace view, window stack, tiling/floating region, switcher, overview | presents policy-owned membership/geometry and selection |
| Panels | panel, taskbar, dock, status area, auto-hide/reveal surface | proposes reservation and emits host actions; host decides exclusivity |
| Launch | launcher, application grid, start menu, search/command surface | consumes host entries/actions; does not enumerate apps/processes |
| Status | clock, power/network/media/input indicators, quick settings, extension slot | presents host models and opaque action IDs; no direct service access |
| Notifications | notification host/center, system dialog, OSD | consumes host models, priority, privacy, and action IDs |
| Secure/system | lock composition, system modal host, accessibility/IME shell slot | requires explicit host capability and Gate 9 service contracts |

`WindowChrome` derives maximize/minimize/close/move/resize availability from the surface snapshot. A
request does not update maximized/focused geometry locally; the host snapshot commits it. Resize
hit regions can extend beyond visible borders but have deterministic precedence over drag/client
regions and never overlap unrelated window controls ambiguously.

`WorkspaceView` is a presentation of policy-owned geometry. Tiling algorithms may be supplied as a
separate policy library, but the standard component does not decide whether a new window floats,
steals focus, changes workspace, or may appear over a lock surface.

Panel auto-hide uses explicit states (`Hidden`, `RevealArmed`, `Revealing`, `Shown`, `Hiding`) and
scheduler deadlines. Output-edge pointer/touch/directional/semantic reveal paths are equivalent.
Accepted reserved area comes from the host; animation does not continuously renegotiate protocol
state unless the host contract explicitly supports it.

### 9.5 External surfaces and rendering

`client_surface` and `surface_tree` lower to the retained scene's external-content records. They:

- preserve surface/subsurface painter order and stable identity;
- apply host-provided crop, transform, opacity, clip, opaque/input region, and damage;
- carry color/protection and acquire/release synchronization metadata without interpreting native
  handles in the component package;
- render a typed placeholder when an import revision is unavailable or rejected; and
- never perform CPU readback, independent submission, presentation, or protocol release.

The selected renderer/import adapter validates and consumes the opaque content token under Gates
3–5. Component teardown releases only its logical reference; external image retirement follows the
host/backend completion contract.

### 9.6 Shell semantics and trust

Shell chrome uses ordinary roles/actions where meanings are universal: button, menu, tab, dialog,
window, list, status, and notification. Shell-only actions such as activate client, begin move,
switch workspace, or reveal panel remain typed shell semantic actions; Gate 9's accessibility
adapter maps only those actions that the active platform API supports.

A client surface is not made accessible by OCR or pixel inspection. Its imported accessibility
subtree, if any, is a separate host-provided semantic attachment merged by Gate 9 with explicit
coordinate/focus ownership. When unavailable, the shell exposes only safe window-level metadata and
actions supplied by the host.

Protected/redacted surface, notification, lock, and user data never enter ordinary diagnostics,
screenshots, semantic dumps, or extension slots. `protected = true` is metadata the renderer/host
must enforce; Telorgon does not claim UI-level redaction alone is secure composition.

Extension slots are capability-limited component hosts. They receive immutable approved models and
opaque action IDs, cannot mint privileged layer tokens, cannot import arbitrary native surfaces,
and cannot issue unrestricted shell requests.

### 9.7 Shell support tiers

Shell support is reported in layers:

| Claim | Required scope |
| --- | --- |
| Shell model/primitive baseline | `telorgon-shell` snapshots/requests plus output/layer/client-surface/input-geometry primitives against a fake host |
| Shell Tier A component baseline | root/output assembly, surface placeholder/tree, window chrome, workspace stack, panel/taskbar, launcher, status area, notification host, system overlay, full cross-input behavior, semantics, themes, and model tests |
| Shell Tier B facilities | overview/switcher, tiling/floating facilities, quick settings, notification center, secure/lock/system-modal hosts, extension slots, and multi-output adaptation |
| Operational shell profile | applicable Tier components plus Gate 5 P4 real host/external-image interop and Gate 9 platform/accessibility/service adapters |

A fake-host Tier A pass proves protocol-neutral UI behavior, not an operational compositor. A lock
composition test proves inert/layer/redaction behavior, not platform security or authentication.

## 10. Styling and component customization

### 10.1 Typed style contracts

Every standard component defines a typed style contract containing named visual slots and state
resolution, for example `ButtonStyle`, `CheckboxStyle`, or `WindowChromeStyle`. The contract maps
compiled domain theme tokens plus supported interaction/semantic states to foundation properties.

Application and shell styles have separate IDs, registries, defaults, and preview scopes. A shared
token compiler may feed both, but an application `ButtonStyleId` cannot resolve a shell window
control and a shell preview cannot mutate application components.

State resolution is deterministic and priority ordered. Invalid/busy/disabled/pressed/focused/
hovered/selected combinations have documented fallback rather than depending on insertion order.
Style resolution cannot change a component's semantic role, action set, controlled value, focus
policy, or host authority.

### 10.2 Custom content and slots

Components expose named slots only where replacement preserves behavior. Replacing a button label
does not replace its action/focus/semantic root. A custom menu row still participates in menu
highlight, typeahead, role, and activation. Authors who need different behavior compose foundation
mechanisms into a custom component instead of disabling half of a standard component contract.

Standard visual slots reserve interaction/focus geometry so hover/focus styles do not reflow layout.
Effects that require offscreen layers declare capability fallbacks and budgets; ordinary rounded
fills, borders, icons, focus rings, and text remain analytic scene content.

## 11. Target package and file layout

Gate 8 fixes the following owners. Files may be split further when a cohesive algorithm grows, but
unrelated responsibilities may not be merged into `lib.rs` or a universal `widgets.rs`.

```text
crates/telorgon/src/input/
  activation.rs          # source-neutral arm/activate/cancel transition engine
  focus.rs               # focus scopes, traversal, restoration, focus-visible inputs
  composite.rs           # active-descendant, keyed selection, and directional navigation operations
  gesture.rs             # arena, cancellation, drag/long-press/tap recognizers
  shortcut.rs            # neutral chord/scope matching; no platform key policy

crates/telorgon/src/ui/
  foundation.rs          # advanced foundation exports only
  external_content.rs    # backend-neutral hosted-content slot record
  overlay.rs             # overlay entries, anchors, modality, focus-lifecycle records
  interaction_state.rs   # compact resolved interaction bits and ownership
  semantics.rs           # mounted neutral role/state/value/action inputs

crates/telorgon/src/layout/
  scroll.rs              # offset/extent/reveal/physics transition mechanics
  virtual_range.rs       # keyed materialization/cache-range calculation
  popup_placement.rs     # anchor/safe-bounds candidate placement solver

crates/telorgon/src/text/
  editor_state.rs        # text/selection/composition editor transition value
  edit_history.rs        # domain-neutral bounded edit-history engine
  transform.rs           # input/display transforms and offset maps

crates/telorgon/src/accessibility/
  tree.rs                # retained semantic tree and relationships
  collection.rs          # virtual collection/index/level metadata
  live_region.rs         # priority/coalescing/redaction records

crates/telorgon/src/application_primitives/
  lib.rs                 # module declarations and curated exports only
  prelude.rs             # application primitive/foundation re-exports
  ext.rs                 # ApplicationUiExt mount conveniences
  root.rs                # application_root scopes
  region.rs              # content/navigation/status regions
  environment.rs         # neutral adaptive application values/policies
  environment_reads.rs   # atomic runtime publication and aspect-specific reads
  hud_layer.rs           # HUD coordinates and pass-through policy
  viewport_overlay.rs    # viewport-relative placement
  world_anchor.rs        # host-projected anchor value/primitive
  render_target_view.rs  # host render-target content primitive
  video_surface.rs       # host media-surface content primitive
  diagnostics.rs

crates/telorgon/src/application_components/
  lib.rs                 # module declarations and curated exports only
  prelude.rs             # Tier A default component exports
  change.rs              # shared ChangePhase/Source/ValueChange values
  density.rs             # Compact/Standard/Touch component metrics
  action/
    button.rs
    icon_button.rs
    toggle_button.rs
    link.rs
  choice/
    check_state.rs
    checkbox.rs
    radio.rs
    switch.rs
  range/
    model.rs
    slider.rs
    range_slider.rs
    progress.rs
    meter.rs
  text/
    controller.rs          # application controller over telorgon-text editor state
    edit_history.rs        # application default grouping/budget policy over shared engine
    field.rs
    area.rs
    search.rs
    numeric.rs
    secure.rs
  scroll/
    controller.rs          # application wrapper over shared scroll transition mechanics
    view.rs
    scrollbar.rs
    split_view.rs
  command/
    model.rs
    shortcut_scope.rs
    toolbar.rs
    menu_controller.rs
    menu.rs
    context_menu.rs
    palette.rs
  overlay/
    host.rs
    controller.rs          # application wrapper over shared overlay entry/lifecycle records
    placement.rs           # application candidate/overflow policy over layout solver
    popup.rs
    dialog.rs
    sheet.rs
    tooltip.rs
    toast.rs
  collection/
    selection.rs
    list.rs
    virtual_list.rs
    listbox.rs
    table.rs
    data_grid.rs
    tree.rs
    tree_grid.rs
  navigation/
    controller.rs
    tabs.rs
    breadcrumb.rs
    rail.rs
    bar.rs
    route_host.rs
  form/
    field.rs
    validation.rs
    form.rs
    summary.rs
  structure/
    scaffold.rs
    adaptive_scaffold.rs
  tool/
    dock_controller.rs
    dock_area.rs
    inspector.rs
    property_grid.rs
    timeline.rs
    console.rs
    graph.rs
  game/
    hud.rs
    reticle.rs
    minimap.rs
    hotbar.rs
    dialogue.rs
    game_menu.rs
  diagnostics.rs

crates/telorgon/src/shell/
  lib.rs                 # protocol-neutral model/request exports only
  id.rs                  # output/surface/workspace/application stable IDs
  capability.rs          # host-granted shell capabilities/layer authority
  model/
    output.rs
    surface.rs
    workspace.rs
    application.rs
    notification.rs
    status.rs
    accessibility.rs
  request/
    result.rs
    input.rs
    surface.rs
    workspace.rs
    output.rs
    system.rs
  host.rs                # snapshot/request transport trait; no protocol types
  diagnostics.rs
  error.rs

crates/telorgon/src/shell_primitives/
  lib.rs
  prelude.rs
  ext.rs                 # ShellUiExt mount conveniences
  root.rs                # shell_root and authorized layer scopes
  output_view.rs
  layer.rs
  client_surface.rs
  surface_tree.rs
  placeholder.rs
  snapshot.rs
  reserved_area.rs
  exclusive_region.rs
  surface_input_region.rs
  drag_region.rs
  resize_region.rs
  output_edge.rs
  diagnostics.rs

crates/telorgon/src/shell_components/
  lib.rs
  prelude.rs
  chrome/
    frame.rs
    titlebar.rs
    controls.rs
    shadow.rs
    snap_preview.rs
  workspace/
    view.rs
    stack.rs
    tiling_region.rs
    floating_region.rs
    switcher.rs
    overview.rs
  panel/
    panel.rs
    auto_hide.rs
    taskbar.rs
    dock.rs
  launcher/
    launcher.rs
    application_grid.rs
    start_menu.rs
  status/
    area.rs
    clock.rs
    indicator.rs
    media.rs
    quick_settings.rs
    extension_slot.rs
  notification/
    host.rs
    center.rs
    system_dialog.rs
    osd.rs
  secure/
    lock_composition.rs
    system_modal.rs
  diagnostics.rs
```

The `telorgon-components-application` prelude initially exports Tier A only. Tier B/C modules are
available through explicit focused modules until their API and support level stabilize. This keeps
the default learning surface understandable without collapsing the packages.

## 12. Ordered implementation slices

Gate 8 implementation begins after the required Gate 7 runtime slices exist. The order is:

1. **Foundation behavior seam:** finish neutral focus, activation, gesture cancellation, semantics,
   overlay, scroll, and environment records with headless fixtures; do not add styled controls yet.
2. **Application baseline actions/choice/range:** create application primitive/component crates,
   preludes, density values, button/toggle/checkbox/radio/switch/slider/progress components, styles,
   semantic goldens, and gallery specimens.
3. **Text and overlay baseline:** create `TextController`, basic edit history, fields/area/search,
   overlay host, popup placement, dialog/sheet/tooltip/toast, and IME/clipboard capability seams that
   remain fake/neutral until the applicable Gate 9 adapters are implemented.
4. **Commands/navigation/collections:** add command specs, shortcuts, menus/toolbars, tabs/routes,
   selection models, scroll/list/virtual-list/listbox, then table/grid/tree behavior.
5. **Application structure and advanced catalogs:** add forms/adaptive scaffold, then Tier B and
   explicitly profiled Tier C tool/game components without widening the default prelude prematurely.
6. **Shell model seam:** create `telorgon-shell` IDs/snapshots/capabilities/request results plus trace
   host fixtures; migrate only protocol-neutral current compositor records.
7. **Shell primitives:** add authorized roots/layers, output/client-surface trees, geometry regions,
   input mapping, placeholder/protection behavior, and external-content trace tests.
8. **Shell components:** add chrome/workspaces/panels/launcher/status/notification/secure facilities
   in that order, each against a fake policy host before any real protocol adapter.
9. **Domain split completion:** split theme registries/galleries/docs, compile application-only and
   shell-only profiles, migrate first-party tools, then remove old builder control conveniences only
   after their compatibility ledger exit conditions pass.

Each slice lands behavior tests and source responsibility together. Constructors without semantics,
keyboard behavior, cancellation, styles, or diagnostics do not satisfy a slice.

## 13. Diagnostics and performance invariants

Per view/domain/component diagnostics include:

- live components by family/tier, controller/state bytes, style resolutions, and dirty reasons;
- activation armed/activated/cancelled/disabled counts by input source;
- focus moves/restores/failures, composite highlight/selection changes, and reveal requests;
- overlays opened/placed/repositioned/dismissed, active barriers, and focus restoration failures;
- text edits/history bytes/IME actions/redactions and scroll/virtual materialization/cache ranges;
- semantic nodes/actions/deltas, missing accessible names, duplicate relationships, and live-region
  coalescing;
- adaptive policy/slot moves and target-size/overlap violations;
- shell snapshot/request/result counts, stale/denied/unsupported requests, external-surface revisions,
  damaged regions, placeholders, protected redactions, and active layer tokens; and
- per-domain retained bytes, allocations, deadlines, scene deltas, and requested frame causes.

After warm-up:

- an idle control/facility causes zero binding, behavior, layout, semantics, scene, allocation,
  task-poll, or frame work;
- hover/press/focus paint feedback does not remount, remeasure, or restructure the control;
- one controlled value change visits only its dependent component/property/semantic paths;
- one slider update coalesces intermediate moves to the host cadence and never submits/render-loops;
- one caret blink or tooltip/menu deadline wakes only the owning live entry;
- one virtual-list key insertion visits the affected materialization/selection range, not all rows;
- moving an adaptive slot preserves its component/controller/semantic identities;
- unchanged render-target/video/client surfaces add no upload, copy, record, or submission work;
- one client-surface damage update patches only affected surface/scene records and damage; and
- an application-only build contains no shell models, external-surface import, or shell theme assets.

Standard behavior tests use exact work counts where deterministic. Machine timings and GPU/profile
claims remain controlled by Gate 6.

## 14. Required acceptance cases

### 14.1 Package and API boundaries

- compile application-only, shell-only, combined, embedded, and headless fixtures independently;
- reject application imports of shell packages and core/runtime imports of either domain;
- ensure preludes omit expert/backend/platform types and `lib.rs` contains no runtime implementation;
- compile-fail attempts to retain `Ui`/context references or mutate controlled values/controllers
  outside authorized transactions; and
- verify every public component is classified by tier and current implementation status.

### 14.2 Universal behavior

- pointer down/move-out/move-in/up, lost capture, cancellation, disable-during-press, unmount, long
  press, double press, secondary button, key repeat, Space, Enter, and semantic activation fixtures;
- focus-visible modality, Tab/reverse-Tab, directional navigation, re-entry/restoration, disabled
  composite items, RTL, removal/replacement, and no focus-to-stale-generation cases;
- target size/overlap at compact/standard/touch density and text scales; pointer-cancellation and
  non-drag alternatives; and
- reduced-motion, inactive view, scheduler deadline cancellation, and zero background-thread cases.

### 14.3 Application components

- controlled toggle/checkbox/radio/switch state and tri-state cycle policies;
- slider/range begin-update-commit-cancel, clamping, steps, reversed/RTL/orientation, two-thumb focus,
  semantic increment/decrement, and no duplicate update cases;
- text revision/edit/selection/composition, filter/transform mapping, undo/redo grouping boundaries,
  secure redaction, submission/newline, read-only/disabled, focus reveal, and controlled-reset cases;
- nested menu/submenu typeahead/focus/dismissal, popup placement/occlusion, modal inertness/focus loop,
  focus restoration, tooltip/toast nonfocus, and anchor removal;
- selection-versus-focus for listbox/table/grid/tree, stable-key reorder/removal, virtualization,
  offscreen reveal/semantics, unknown counts, tree/grid edit mode, and non-drag alternatives; and
- adaptive scaffold slot identity across width/input/text-scale changes plus game/viewport hit
  pass-through and no host-renderer interference.

### 14.4 Semantics and visual contracts

- semantic-tree goldens for every state/input mode, including name/value/state/action/relationships,
  collection metadata, modal/inert, validation/error, live region, and missing-name diagnostics;
- keyboard/directional behavior matrices cross-checked with semantic actions;
- application/shell theme-state visual goldens at density, scale, contrast, direction, disabled,
  focus-visible, selected/checked, invalid, busy, and reduced-motion variants; and
- analytic primitive/batching assertions showing ordinary controls do not allocate unnecessary path
  meshes or offscreen layers.

### 14.5 Shell behavior

- stale/unknown/denied/unsupported host requests never change authoritative visual/semantic state;
- output add/remove/scale/transform, multi-output identity, workspace reorder, surface parent/reorder,
  damage, input/opaque region, unavailable/protected content, and placeholder behavior;
- titlebar button, drag/resize precedence/cancellation, snap preview, panel reservation/auto-hide,
  lock/system-modal inertness, separate UI/client focus, and local coordinate input mapping;
- privileged layer-token and extension capability rejection; redaction of protected surface,
  notification, lock, and user data; and
- external-content trace proving no CPU readback, protocol call, native-handle ownership, independent
  submission, presentation, or premature synchronization release in a shell component package.

Gate 6's E0–E3 layers cover portable behavior, compile boundaries, semantic/scene traces, and visual
fixtures. Operational external surfaces, accessibility export, platform services, and real shell
hosts add the applicable E4–E9 evidence when Gate 9 adapters exist.

## 15. Gate 8 reference and specification audit

```text
Concern:
Define an understandable, accessible, cross-input control library and a separate protocol-neutral
shell library without duplicating the runtime or embedding platform/window policy.

Telorgon files/contracts affected:
APPLICATION_AND_SHELL_PRIMITIVES.md; PROJECT_SCOPE_AND_ARCHITECTURE.md sections 6–9;
AUTHORING_AND_COMPONENT_RUNTIME.md default behavior/controller boundaries; MIGRATION_PLAN.md Epoch E;
target foundation, application-domain, telorgon-shell, and shell-domain packages.

Reference revisions, paths, and symbols inspected:
Flutter 51fd9afadf309ba5337320bd3653f5345c156cb9 — material/button_style_button.dart
ButtonStyleButton and WidgetState resolution; checkbox.dart controlled/tristate callbacks;
widgets/radio_group.dart composite focus/navigation; material/slider.dart phased changes and keyboard;
material/tabs.dart controller/selection; material/menu_anchor.dart nested overlay/focus/dismissal;
widgets/editable_text.dart controller, selection, input actions, and semantics.
AndroidX Compose support 491d5b9a1de8225097e39684c3412f40f227a0f7 — foundation Clickable.kt;
selection/Toggleable.kt and Selectable.kt state/role/action separation; text/input/TextFieldState.kt
editing/selection/composition/undo; lazy/layout/LazyLayout.kt materialization/prefetch ownership.
Qt Declarative 3e2d6bd456a8e850bcf641de77d1d5d8bc8419ef — quicktemplates
QQuickAbstractButton press/key/touch/check/exclusive behavior; QQuickComboBox highlight versus
activation/typeahead; QQuickPopup close policy, input interception, placement, focus restoration.
Android platform base 1cdfff555f4a21f71ccc978290e2e212e2f8b168 — SystemUI model/interactor/view-model
separation; WindowManager Shell docs policy separation; windowdecor/WindowDecoration external task
surface/chrome/input regions; desktopmode/ShellModeVisualIndicator policy-driven snap feedback.

Official specifications/guidance checked:
WAI-ARIA 1.2 role/state/property ontology; WAI-ARIA Authoring Practices keyboard-interface guidance
and button, checkbox, radio, slider, tabs, menu/menubar, combobox, listbox, grid, tree, dialog, and
toolbar patterns; WCAG 2.2 focus, pointer cancellation, concurrent input, dragging alternative, and
target-size criteria.

Invariants extracted:
Controlled values and callbacks keep data ownership visible; semantic roles/actions belong with
behavior; focus differs from selection/highlight; composites require predictable directional focus;
disabled discovery is pattern-specific; modal content makes lower content inert and restores focus;
popup close/anchor/focus ordering needs an explicit controller; text content/selection/composition
move together; lazy collections preserve stable item identity; shell visuals consume models and emit
requests while policy validates and commits truth.

Failure/recovery cases extracted:
Activation after pointer cancel or disable; duplicate click after long/double press; focus sent to a
removed item; selection lost during navigation; controlled text resetting user edits; menu close and
anchor click reopening in one turn; closing overlay remaining interactive/semantic; popup outside
usable bounds; virtualized semantic index drift; drag as the only operation; stale shell request
optimistically moving a surface; resize region stealing titlebar/client input; protected surface data
appearing in dumps; lock visuals treated as the security boundary.

Approaches rejected and why:
Calling every visible element a primitive; one universal widgets crate/prelude; application
components subclassed by shell components; frame-local immediate widget rebuilding; generic two-way
binding; string command/event buses; component-owned platform services; global overlay/window
singletons; theme-defined semantics; native/platform control types in public APIs; protocol objects
or window policy in shell components; client accessibility inferred from pixels.

Telorgon-specific decision:
Shared focused foundations, sibling application/shell primitive and component packages, controlled
semantic values plus typed actions, specialized controllers, one retained overlay/focus mechanism,
constraint/capability-driven adaptation, mandatory semantic behavior, protocol-neutral shell
snapshots/requests with host authority, explicit catalog tiers, and per-concern source files.

Tests/diagnostics derived:
Section 14 plus exact activation/focus/selection/controller/overlay/virtualization/shell request
counts, semantic and visual goldens, compile dependency fixtures, target-size/drag alternatives,
protected-data redaction, external-surface traces, and application-only/shell-only profile builds.

Known gaps requiring Gate 9 implementation or prototypes:
Native accessibility role/action mappings and assistive focus; platform key/shortcut conventions;
IME/virtual keyboard, clipboard, drag/drop, haptics, cursor, URI/file/menu/window services; real
protocol surface/input/accessibility bridges; final theme token/style/motion syntax; rich editor and
docking algorithms; concrete vendor shell authorization/security integration.
```

Primary official references:

- [WAI-ARIA 1.2](https://www.w3.org/TR/wai-aria-1.2/)
- [WAI-ARIA Authoring Practices patterns](https://www.w3.org/WAI/ARIA/apg/patterns/)
- [Developing a keyboard interface](https://www.w3.org/WAI/ARIA/apg/practices/keyboard-interface/)
- [WCAG 2.2](https://www.w3.org/TR/WCAG22/)
- [Modal dialog pattern](https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/)
- [Menu and menubar pattern](https://www.w3.org/WAI/ARIA/apg/patterns/menubar/)
- [Tree view pattern](https://www.w3.org/WAI/ARIA/apg/patterns/treeview/)

The WAI-ARIA patterns inform Telorgon's neutral cross-platform behavior and semantic tests; they are
not copied as a web/DOM runtime or treated as a substitute for native accessibility validation.
Adjacent source was inspected read-only, and no source or distinctive test vector was copied.

## 16. Deferred, not undefined

Gate 8 deliberately leaves these to their named owners:

- native lifecycle/window/view ownership, services, key translation, cursor, clipboard, drag/drop,
  haptics, menus, notifications, accessibility export, IME, and virtual keyboard — implement
  [Gate 9](PLATFORM_INTEGRATION_CONTRACT.md);
- exact theme source syntax, style inheritance, motion tokens, and design-tool interchange — focused
  theme/motion work using the typed style contracts here;
- renderer/RHI/external-image native handle and synchronization implementation — Gates 3–5;
- display protocols, protocol serials, client validation, focus/window/workspace/security policy,
  app/service enumeration, and real shell authorization — the host/platform integration;
- rich document spans, multi-cursor, syntax, collaboration, advanced undo, and large-editor
  virtualization — later editor packages using `TextController`/`telorgon-text` foundations;
- final docking, graph, timeline, calendar, and vendor shell algorithms — Tier B/C focused work after
  Tier A behavior is qualified; and
- application business models, persistence, networking, game state, and engine rendering — the
  application/host.

These extensions preserve the two-domain dependency cut, controlled value/action ownership,
component identity, neutral semantics, explicit host authority, and no-hidden-render-work rules.

## 17. Gate completion criteria

Gate 8 is complete when:

1. every named type can be classified as foundation mechanism, domain primitive, component, or
   facility without a duplicate owner;
2. application and shell package/prelude dependencies are separate and acyclic;
3. controlled values, controllers, actions, state ownership, and customization rules are exact;
4. activation, focus, selection, composite navigation, cancellation, density/adaptation, semantics,
   overlays, text, and virtualization have baseline contracts;
5. Tier A and later catalog support claims cannot be confused;
6. shell models/requests/layers/surfaces/chrome preserve host protocol, policy, and security
   ownership;
7. target files, implementation slices, diagnostics, performance invariants, and Gate 6 acceptance
   cases are assigned;
8. reference revisions and official accessibility guidance are recorded without copying code; and
9. active documents link to this authority without claiming these planned packages already exist.

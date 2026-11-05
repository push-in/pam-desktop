//! UI Automation over the retained view tree.
//!
//! The window answers `WM_GETOBJECT` with a fragment root; every exposed
//! view is a fragment created on demand (providers hold a node id, never a
//! pointer into the scene). UI Automation is asked to call on the UI
//! thread (`ProviderOptions_UseComThreading`), where the scene lives.
//! Native text controls and web views bring their own providers and are
//! left out of this tree.

#![allow(
    non_upper_case_globals,
    reason = "UI Automation constants keep their SDK names, also in patterns"
)]

use std::rc::Rc;

use pam_desktop_ui::accessibility::{Accessibility, Checked, LiveRegion, Role};
use pam_desktop_ui::backend::{BackendEvent, NodeId, WindowId};
use pam_desktop_ui::geometry::Point;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, SAFEARRAY};
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::{
    Assertive, CLSID_AccPropServices, ExpandCollapseState, ExpandCollapseState_Collapsed,
    ExpandCollapseState_Expanded, IAccPropServices, IExpandCollapseProvider,
    IExpandCollapseProvider_Impl, IInvokeProvider, IInvokeProvider_Impl, IRangeValueProvider,
    IRangeValueProvider_Impl, IRawElementProviderFragment, IRawElementProviderFragment_Impl,
    IRawElementProviderFragmentRoot, IRawElementProviderFragmentRoot_Impl,
    IRawElementProviderSimple, IRawElementProviderSimple_Impl, ISelectionItemProvider,
    ISelectionItemProvider_Impl, IToggleProvider, IToggleProvider_Impl, Name_Property_GUID,
    NavigateDirection, NavigateDirection_FirstChild, NavigateDirection_LastChild,
    NavigateDirection_NextSibling, NavigateDirection_Parent, NavigateDirection_PreviousSibling,
    NotificationKind_Other, NotificationProcessing_All, NotificationProcessing_ImportantAll, Off,
    Polite, ProviderOptions, ProviderOptions_ServerSideProvider, ProviderOptions_UseComThreading,
    ToggleState, ToggleState_Indeterminate, ToggleState_Off, ToggleState_On,
    UIA_AutomationFocusChangedEventId, UIA_AutomationIdPropertyId, UIA_ButtonControlTypeId,
    UIA_CONTROLTYPE_ID, UIA_CheckBoxControlTypeId, UIA_ComboBoxControlTypeId,
    UIA_ControlTypePropertyId, UIA_E_ELEMENTNOTAVAILABLE, UIA_E_INVALIDOPERATION,
    UIA_EditControlTypeId, UIA_ExpandCollapseExpandCollapseStatePropertyId,
    UIA_ExpandCollapsePatternId, UIA_FrameworkIdPropertyId, UIA_GroupControlTypeId,
    UIA_HasKeyboardFocusPropertyId, UIA_HelpTextPropertyId, UIA_HyperlinkControlTypeId,
    UIA_ImageControlTypeId, UIA_InvokePatternId, UIA_IsContentElementPropertyId,
    UIA_IsControlElementPropertyId, UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId,
    UIA_IsOffscreenPropertyId, UIA_ItemStatusPropertyId, UIA_ListControlTypeId,
    UIA_ListItemControlTypeId, UIA_LiveSettingPropertyId, UIA_MenuBarControlTypeId,
    UIA_MenuControlTypeId, UIA_MenuItemControlTypeId, UIA_NamePropertyId, UIA_PATTERN_ID,
    UIA_PROPERTY_ID, UIA_ProgressBarControlTypeId, UIA_RadioButtonControlTypeId,
    UIA_RangeValuePatternId, UIA_RangeValueValuePropertyId, UIA_ScrollBarControlTypeId,
    UIA_SelectionItemIsSelectedPropertyId, UIA_SelectionItemPatternId, UIA_SliderControlTypeId,
    UIA_SpinnerControlTypeId, UIA_TabControlTypeId, UIA_TabItemControlTypeId,
    UIA_TableControlTypeId, UIA_TextControlTypeId, UIA_TogglePatternId,
    UIA_ToggleToggleStatePropertyId, UIA_ToolBarControlTypeId, UIA_WindowControlTypeId,
    UiaAppendRuntimeId, UiaClientsAreListening, UiaHostProviderFromHwnd, UiaRaiseAutomationEvent,
    UiaRaiseAutomationPropertyChangedEvent, UiaRaiseNotificationEvent, UiaRect,
    UiaReturnRawElementProvider, UiaRootObjectId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, OBJID_CLIENT, SPI_GETSCREENREADER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SystemParametersInfoW,
};
use windows::core::{
    BOOL, BSTR, Error, HRESULT, IUnknown, IUnknownImpl, Interface, Result, implement,
};

use crate::app::App;
use crate::scene::{Scene, View};

fn unavailable() -> Error {
    Error::from(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32))
}

/// "No element": success with a null result.
fn none() -> Error {
    Error::empty()
}

/// Neither the view nor anything inside it is exposed.
fn subtree_hidden(view: &View) -> bool {
    !view.layer.visible
        || view
            .accessibility
            .as_ref()
            .is_some_and(|accessibility| accessibility.hide_descendants)
}

/// Whether a view is an element of the tree (its children are in any case,
/// promoted to its parent when it is not).
fn exposed(view: &View) -> bool {
    // Native controls and web pages describe themselves through their own windows.
    view.input.is_none()
        && view.web.is_none()
        && !view
            .accessibility
            .as_ref()
            .is_some_and(|accessibility| accessibility.hidden)
}

fn collect_children(scene: &Scene, children: &[NodeId], out: &mut Vec<NodeId>, depth: usize) {
    for child in children {
        let Some(view) = scene.views.get(child) else {
            continue;
        };
        if subtree_hidden(view) || depth > 256 {
            continue;
        }
        if exposed(view) {
            out.push(*child);
        } else {
            collect_children(scene, &view.children, out, depth + 1);
        }
    }
}

/// Exposed children of an element (`None` is the window root).
fn children_of(scene: &Scene, window: WindowId, node: Option<NodeId>) -> Vec<NodeId> {
    let mut out = Vec::new();
    match node {
        Some(id) => {
            if let Some(view) = scene.views.get(&id) {
                collect_children(scene, &view.children, &mut out, 0);
            }
        }
        None => {
            if let Some(entry) = scene.windows.get(&window) {
                let layers: Vec<NodeId> = entry
                    .root
                    .iter()
                    .chain(entry.overlays.iter())
                    .copied()
                    .collect();
                collect_children(scene, &layers, &mut out, 0);
            }
        }
    }
    out
}

/// The exposed parent of a view: `Some(None)` is the window root, `None`
/// means the view is not in a window.
fn parent_of(scene: &Scene, id: NodeId) -> Option<Option<NodeId>> {
    let mut current = scene.views.get(&id)?;
    for _ in 0..4096 {
        match current.parent {
            Some(parent) => {
                let view = scene.views.get(&parent)?;
                if exposed(view) {
                    return Some(Some(parent));
                }
                current = view;
            }
            None => return current.layer_of.map(|_| None),
        }
    }
    None
}

/// The accessible name: the label, or the text the view shows.
fn name_of(view: &View) -> String {
    match view
        .accessibility
        .as_ref()
        .map(|accessibility| accessibility.label.as_str())
        .filter(|label| !label.is_empty())
    {
        Some(label) => label.to_owned(),
        None => view.plain_text().unwrap_or_default(),
    }
}

fn control_type(view: &View) -> UIA_CONTROLTYPE_ID {
    let Some(accessibility) = view.accessibility.as_ref() else {
        // A plain view that shows text is static text.
        return if view.text.is_some() {
            UIA_TextControlTypeId
        } else {
            UIA_GroupControlTypeId
        };
    };
    match accessibility.role {
        Role::Generic
        | Role::RadioGroup
        | Role::Summary
        | Role::Timer
        | Role::Alert
        | Role::Search => UIA_GroupControlTypeId,
        Role::Button | Role::ToggleButton => UIA_ButtonControlTypeId,
        Role::TextInput => UIA_EditControlTypeId,
        Role::Image => UIA_ImageControlTypeId,
        Role::Switch | Role::Checkbox => UIA_CheckBoxControlTypeId,
        Role::Slider => UIA_SliderControlTypeId,
        Role::ComboBox => UIA_ComboBoxControlTypeId,
        Role::Heading | Role::Text => UIA_TextControlTypeId,
        Role::Link => UIA_HyperlinkControlTypeId,
        Role::Menu => UIA_MenuControlTypeId,
        Role::MenuBar => UIA_MenuBarControlTypeId,
        Role::MenuItem => UIA_MenuItemControlTypeId,
        Role::ProgressBar => UIA_ProgressBarControlTypeId,
        Role::Radio => UIA_RadioButtonControlTypeId,
        Role::ScrollBar => UIA_ScrollBarControlTypeId,
        Role::SpinButton => UIA_SpinnerControlTypeId,
        Role::Tab => UIA_TabItemControlTypeId,
        Role::TabList => UIA_TabControlTypeId,
        Role::Toolbar => UIA_ToolBarControlTypeId,
        Role::Grid => UIA_TableControlTypeId,
        Role::List => UIA_ListControlTypeId,
        Role::ListItem => UIA_ListItemControlTypeId,
        Role::Dialog => UIA_WindowControlTypeId,
    }
}

/// Whether assistive technology should stop on the element (a bare layout
/// box only exists in the raw tree).
fn is_control(view: &View) -> bool {
    view.accessibility.is_some() || view.text.is_some() || view.focusable
}

/// Whether the item is selected, for elements that are selection items:
/// those with a selected state, and radio buttons (their checked state).
fn selection_of(accessibility: &Accessibility) -> Option<bool> {
    accessibility.selected.or_else(|| {
        (accessibility.role == Role::Radio).then(|| accessibility.checked == Some(Checked::Checked))
    })
}

/// Whether the view is part of the tree assistive technology walks:
/// exposed itself, in a window, and under no ancestor that hides it.
fn reachable(scene: &Scene, id: NodeId) -> bool {
    let Some(view) = scene.views.get(&id) else {
        return false;
    };
    if !exposed(view) || subtree_hidden(view) {
        return false;
    }
    let mut current = view;
    for _ in 0..4096 {
        match current.parent {
            Some(parent) => match scene.views.get(&parent) {
                Some(parent) if !subtree_hidden(parent) => current = parent,
                _ => return false,
            },
            None => return current.layer_of.is_some(),
        }
    }
    false
}

/// Whether the default action (a press) makes sense for the element.
fn invokable(view: &View) -> bool {
    view.focusable
        || view.accessibility.as_ref().is_some_and(|accessibility| {
            matches!(
                accessibility.role,
                Role::Button | Role::Link | Role::MenuItem | Role::Tab | Role::ListItem
            )
        })
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot
)]
struct RootProvider {
    hwnd: HWND,
    window: WindowId,
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IInvokeProvider,
    IToggleProvider,
    ISelectionItemProvider,
    IExpandCollapseProvider,
    IRangeValueProvider
)]
struct NodeProvider {
    hwnd: HWND,
    window: WindowId,
    node: NodeId,
}

fn node_fragment(hwnd: HWND, window: WindowId, node: NodeId) -> IRawElementProviderFragment {
    NodeProvider { hwnd, window, node }.into()
}

/// The root provider of a window, created once so its identity is stable.
fn root_provider(app: &App, hwnd: HWND, window: WindowId) -> Option<IRawElementProviderSimple> {
    app.with_state(|state| {
        let entry = state.scene.windows.get_mut(&window)?;
        Some(
            entry
                .automation
                .get_or_insert_with(|| RootProvider { hwnd, window }.into())
                .clone(),
        )
    })
    .flatten()
}

fn options() -> ProviderOptions {
    // Calls arrive on the thread that owns the scene.
    ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading
}

fn runtime_id(node: NodeId) -> Result<*mut SAFEARRAY> {
    let parts = [
        UiaAppendRuntimeId as i32,
        (node & 0xFFFF_FFFF) as u32 as i32,
        (node >> 32) as u32 as i32,
    ];
    // SAFETY: a three element vector of 32-bit integers, filled index by
    // index from live locals; ownership passes to the caller.
    unsafe {
        let array = SafeArrayCreateVector(VT_I4, 0, parts.len() as u32);
        if array.is_null() {
            return Err(Error::from(windows::Win32::Foundation::E_OUTOFMEMORY));
        }
        for (index, part) in parts.iter().enumerate() {
            let index = index as i32;
            SafeArrayPutElement(array, &index, std::ptr::from_ref(part).cast())?;
        }
        Ok(array)
    }
}

impl IRawElementProviderSimple_Impl for RootProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(options())
    }

    fn GetPatternProvider(&self, _pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        Err(none())
    }

    fn GetPropertyValue(&self, _property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        // The window's own provider (the host) answers for the root.
        Ok(VARIANT::default())
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        // SAFETY: `hwnd` is the window this provider was created for.
        unsafe { UiaHostProviderFromHwnd(self.hwnd) }
    }
}

impl IRawElementProviderFragment_Impl for RootProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let app = App::current().ok_or_else(unavailable)?;
        let children = app
            .with_state(|state| children_of(&state.scene, self.window, None))
            .ok_or_else(unavailable)?;
        let target = match direction {
            NavigateDirection_FirstChild => children.first(),
            NavigateDirection_LastChild => children.last(),
            _ => None,
        };
        target
            .map(|node| node_fragment(self.hwnd, self.window, *node))
            .ok_or_else(none)
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        // The host window supplies the root's runtime id.
        Err(none())
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        // The host window supplies the root's rectangle.
        Ok(UiaRect::default())
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Err(none())
    }

    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        let app = App::current().ok_or_else(unavailable)?;
        root_provider(&app, self.hwnd, self.window)
            .ok_or_else(unavailable)?
            .cast()
    }
}

impl IRawElementProviderFragmentRoot_Impl for RootProvider_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let app = App::current().ok_or_else(unavailable)?;
        let mut point = POINT {
            x: x.round() as i32,
            y: y.round() as i32,
        };
        // SAFETY: plain coordinate conversion on our window.
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut point);
        }
        let hit = app
            .with_state(|state| {
                let scale = state.scene.windows.get(&self.window)?.scale();
                let mut path = Vec::new();
                state.scene.hit_path(
                    self.window,
                    Point::new(point.x as f32 / scale, point.y as f32 / scale),
                    &mut path,
                );
                // The deepest exposed view on the path, unless an ancestor hides it.
                let hidden_from = path
                    .iter()
                    .position(|id| state.scene.views.get(id).is_none_or(subtree_hidden))
                    .unwrap_or(path.len());
                path[..hidden_from]
                    .iter()
                    .rev()
                    .copied()
                    .find(|id| state.scene.views.get(id).is_some_and(exposed))
            })
            .flatten();
        match hit {
            Some(node) => Ok(node_fragment(self.hwnd, self.window, node)),
            None => root_provider(&app, self.hwnd, self.window)
                .ok_or_else(unavailable)?
                .cast(),
        }
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let app = App::current().ok_or_else(unavailable)?;
        let focused = app
            .with_state(|state| {
                let focus = state.scene.focus?;
                let view = state.scene.views.get(&focus)?;
                (state.scene.window_of(focus) == Some(self.window) && exposed(view))
                    .then_some(focus)
            })
            .flatten();
        focused
            .map(|node| node_fragment(self.hwnd, self.window, node))
            .ok_or_else(none)
    }
}

impl NodeProvider {
    /// Reads from the view, or fails when it is gone.
    fn read<T>(&self, read: impl FnOnce(&Scene, &View) -> T) -> Result<T> {
        let app = App::current().ok_or_else(unavailable)?;
        app.with_state(|state| {
            state
                .scene
                .views
                .get(&self.node)
                .map(|view| read(&state.scene, view))
        })
        .flatten()
        .ok_or_else(unavailable)
    }

    fn described<T>(&self, read: impl FnOnce(&Accessibility) -> Option<T>) -> Result<T> {
        self.read(|_, view| view.accessibility.as_deref().and_then(read))?
            .ok_or_else(|| Error::from(HRESULT(UIA_E_INVALIDOPERATION as i32)))
    }

    /// The default action: the core presses the node.
    fn activate(&self) -> Result<()> {
        let app = App::current().ok_or_else(unavailable)?;
        let enabled = self.read(|_, view| {
            !view
                .accessibility
                .as_ref()
                .is_some_and(|accessibility| accessibility.disabled)
        })?;
        if !enabled {
            return Err(Error::from(HRESULT(
                windows::Win32::UI::Accessibility::UIA_E_ELEMENTNOTENABLED as i32,
            )));
        }
        app.emit(BackendEvent::AccessibilityAction {
            id: self.node,
            name: String::from("activate"),
        });
        app.pump();
        Ok(())
    }
}

impl IRawElementProviderSimple_Impl for NodeProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(options())
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        let supported = self.read(|_, view| {
            let described = view.accessibility.as_deref();
            match pattern {
                UIA_InvokePatternId => invokable(view),
                UIA_TogglePatternId => {
                    described.is_some_and(|accessibility| accessibility.checked.is_some())
                }
                UIA_SelectionItemPatternId => {
                    described.is_some_and(|accessibility| selection_of(accessibility).is_some())
                }
                UIA_ExpandCollapsePatternId => {
                    described.is_some_and(|accessibility| accessibility.expanded.is_some())
                }
                UIA_RangeValuePatternId => {
                    described.is_some_and(|accessibility| accessibility.range.is_some())
                }
                _ => false,
            }
        })?;
        if supported {
            Ok(self.to_interface::<IUnknown>())
        } else {
            Err(none())
        }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        self.read(|scene, view| {
            let described = view.accessibility.as_deref();
            match property {
                UIA_ControlTypePropertyId => VARIANT::from(control_type(view).0),
                UIA_NamePropertyId => VARIANT::from(BSTR::from(name_of(view))),
                UIA_HelpTextPropertyId => described
                    .filter(|accessibility| !accessibility.hint.is_empty())
                    .map_or_else(VARIANT::default, |accessibility| {
                        VARIANT::from(BSTR::from(accessibility.hint.as_str()))
                    }),
                UIA_AutomationIdPropertyId => described
                    .filter(|accessibility| !accessibility.identifier.is_empty())
                    .map_or_else(VARIANT::default, |accessibility| {
                        VARIANT::from(BSTR::from(accessibility.identifier.as_str()))
                    }),
                UIA_IsEnabledPropertyId => {
                    VARIANT::from(!described.is_some_and(|accessibility| accessibility.disabled))
                }
                UIA_ItemStatusPropertyId => match described {
                    Some(accessibility) if !accessibility.value_text.is_empty() => {
                        VARIANT::from(BSTR::from(accessibility.value_text.as_str()))
                    }
                    Some(accessibility) if accessibility.busy => VARIANT::from(BSTR::from("busy")),
                    _ => VARIANT::default(),
                },
                UIA_IsControlElementPropertyId | UIA_IsContentElementPropertyId => {
                    VARIANT::from(is_control(view))
                }
                UIA_IsKeyboardFocusablePropertyId => VARIANT::from(view.focusable),
                UIA_HasKeyboardFocusPropertyId => VARIANT::from(scene.focus == Some(self.node)),
                UIA_IsOffscreenPropertyId => VARIANT::from(!scene.is_shown(self.node)),
                UIA_LiveSettingPropertyId => VARIANT::from(
                    match described.map_or(LiveRegion::Off, |accessibility| accessibility.live) {
                        LiveRegion::Off => Off.0,
                        LiveRegion::Polite => Polite.0,
                        LiveRegion::Assertive => Assertive.0,
                    },
                ),
                UIA_FrameworkIdPropertyId => VARIANT::from(BSTR::from("PAM")),
                _ => VARIANT::default(),
            }
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        // Only the root is hosted in a window.
        Err(none())
    }
}

impl IRawElementProviderFragment_Impl for NodeProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let app = App::current().ok_or_else(unavailable)?;
        // `Some(None)` is the window root.
        let known = app
            .with_state(|state| state.scene.views.contains_key(&self.node))
            .ok_or_else(unavailable)?;
        if !known {
            return Err(unavailable());
        }
        let target: Option<Option<NodeId>> = app
            .with_state(|state| {
                let scene = &state.scene;
                let parent = parent_of(scene, self.node)?;
                match direction {
                    NavigateDirection_Parent => Some(parent),
                    NavigateDirection_FirstChild => {
                        children_of(scene, self.window, Some(self.node))
                            .first()
                            .map(|node| Some(*node))
                    }
                    NavigateDirection_LastChild => children_of(scene, self.window, Some(self.node))
                        .last()
                        .map(|node| Some(*node)),
                    NavigateDirection_NextSibling | NavigateDirection_PreviousSibling => {
                        let siblings = children_of(scene, self.window, parent);
                        let index = siblings.iter().position(|sibling| *sibling == self.node)?;
                        let neighbour = if direction == NavigateDirection_NextSibling {
                            index.checked_add(1)
                        } else {
                            index.checked_sub(1)
                        }?;
                        siblings.get(neighbour).map(|node| Some(*node))
                    }
                    _ => None,
                }
            })
            .ok_or_else(unavailable)?;
        match target {
            Some(Some(node)) => Ok(node_fragment(self.hwnd, self.window, node)),
            Some(None) => root_provider(&app, self.hwnd, self.window)
                .ok_or_else(unavailable)?
                .cast(),
            None => Err(none()),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        runtime_id(self.node)
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let app = App::current().ok_or_else(unavailable)?;
        let bounds = app
            .with_state(|state| {
                let (window, transform) = state.scene.world_transform(self.node)?;
                let scale = state.scene.windows.get(&window)?.scale();
                let rect = transform.bounds(state.scene.views.get(&self.node)?.bounds());
                Some((rect, scale))
            })
            .flatten()
            .ok_or_else(unavailable)?;
        let (rect, scale) = bounds;
        let mut origin = POINT {
            x: (rect.x * scale).round() as i32,
            y: (rect.y * scale).round() as i32,
        };
        // SAFETY: plain coordinate conversion on our window.
        unsafe {
            let _ = ClientToScreen(self.hwnd, &mut origin);
        }
        Ok(UiaRect {
            left: f64::from(origin.x),
            top: f64::from(origin.y),
            width: f64::from(rect.width * scale),
            height: f64::from(rect.height * scale),
        })
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Err(none())
    }

    fn SetFocus(&self) -> Result<()> {
        let app = App::current().ok_or_else(unavailable)?;
        crate::focus::set(&app, Some(self.node));
        app.pump();
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        let app = App::current().ok_or_else(unavailable)?;
        root_provider(&app, self.hwnd, self.window)
            .ok_or_else(unavailable)?
            .cast()
    }
}

impl IInvokeProvider_Impl for NodeProvider_Impl {
    fn Invoke(&self) -> Result<()> {
        self.activate()
    }
}

impl IToggleProvider_Impl for NodeProvider_Impl {
    fn Toggle(&self) -> Result<()> {
        self.activate()
    }

    fn ToggleState(&self) -> Result<ToggleState> {
        self.described(|accessibility| accessibility.checked.map(toggle_state))
    }
}

impl ISelectionItemProvider_Impl for NodeProvider_Impl {
    fn Select(&self) -> Result<()> {
        if self.described(selection_of)? {
            Ok(())
        } else {
            self.activate()
        }
    }

    fn AddToSelection(&self) -> Result<()> {
        self.Select()
    }

    fn RemoveFromSelection(&self) -> Result<()> {
        // Selection is single: an item is deselected by selecting another.
        Err(Error::from(HRESULT(UIA_E_INVALIDOPERATION as i32)))
    }

    fn IsSelected(&self) -> Result<BOOL> {
        self.described(selection_of).map(BOOL::from)
    }

    fn SelectionContainer(&self) -> Result<IRawElementProviderSimple> {
        Err(none())
    }
}

impl IExpandCollapseProvider_Impl for NodeProvider_Impl {
    fn Expand(&self) -> Result<()> {
        if self.described(|accessibility| accessibility.expanded)? {
            Ok(())
        } else {
            self.activate()
        }
    }

    fn Collapse(&self) -> Result<()> {
        if self.described(|accessibility| accessibility.expanded)? {
            self.activate()
        } else {
            Ok(())
        }
    }

    fn ExpandCollapseState(&self) -> Result<ExpandCollapseState> {
        self.described(|accessibility| accessibility.expanded)
            .map(|expanded| {
                if expanded {
                    ExpandCollapseState_Expanded
                } else {
                    ExpandCollapseState_Collapsed
                }
            })
    }
}

impl IRangeValueProvider_Impl for NodeProvider_Impl {
    fn SetValue(&self, _value: f64) -> Result<()> {
        // The value belongs to the application; it changes through its own controls.
        Err(Error::from(HRESULT(UIA_E_INVALIDOPERATION as i32)))
    }

    fn Value(&self) -> Result<f64> {
        self.described(|accessibility| accessibility.range)
            .map(|(_, _, now)| f64::from(now))
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        Ok(BOOL::from(true))
    }

    fn Maximum(&self) -> Result<f64> {
        self.described(|accessibility| accessibility.range)
            .map(|(_, maximum, _)| f64::from(maximum))
    }

    fn Minimum(&self) -> Result<f64> {
        self.described(|accessibility| accessibility.range)
            .map(|(minimum, _, _)| f64::from(minimum))
    }

    fn LargeChange(&self) -> Result<f64> {
        Ok(0.0)
    }

    fn SmallChange(&self) -> Result<f64> {
        Ok(0.0)
    }
}

/// `WM_GETOBJECT`: hands the window's fragment root to UI Automation.
pub fn get_object(
    app: &Rc<App>,
    hwnd: HWND,
    window: WindowId,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    // The object id travels in the low 32 bits.
    if lparam.0 as i32 != UiaRootObjectId {
        return None;
    }
    let root = root_provider(app, hwnd, window)?;
    // SAFETY: `hwnd` is the window receiving the message; no state borrow
    // is held (UI Automation may call the provider back at once).
    Some(unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &root) })
}

/// Releases what UI Automation holds for a destroyed window.
pub fn window_destroyed(hwnd: HWND) {
    // SAFETY: the documented way to disconnect every provider of a window.
    unsafe {
        UiaReturnRawElementProvider(
            hwnd,
            WPARAM(0),
            LPARAM(0),
            None::<&IRawElementProviderSimple>,
        );
    }
}

fn listening() -> bool {
    // SAFETY: plain query without preconditions.
    unsafe { UiaClientsAreListening() }.as_bool()
}

fn simple_provider(app: &App, id: NodeId) -> Option<IRawElementProviderSimple> {
    let (hwnd, window) = app
        .with_state(|state| {
            let window = state.scene.window_of(id)?;
            Some((state.scene.windows.get(&window)?.hwnd, window))
        })
        .flatten()?;
    Some(
        NodeProvider {
            hwnd,
            window,
            node: id,
        }
        .into(),
    )
}

/// Keyboard focus moved to a view that is not a native control.
pub fn focus_changed(app: &Rc<App>, id: NodeId) {
    if !listening() {
        return;
    }
    if let Some(provider) = simple_provider(app, id) {
        // SAFETY: raising an event with a live provider; no state borrow is held.
        unsafe {
            let _ = UiaRaiseAutomationEvent(&provider, UIA_AutomationFocusChangedEventId);
        }
    }
}

/// Names a native text control after its label (dynamic annotation: the
/// control keeps its own provider).
fn annotate_control(hwnd: HWND, name: &str) {
    // SAFETY: standard in-process activation and calls with a live window.
    unsafe {
        let Ok(services) = CoCreateInstance::<_, IAccPropServices>(
            &CLSID_AccPropServices,
            None,
            CLSCTX_INPROC_SERVER,
        ) else {
            return;
        };
        if name.is_empty() {
            let _ = services.ClearHwndProps(
                hwnd,
                OBJID_CLIENT.0 as u32,
                CHILDID_SELF,
                &[Name_Property_GUID],
            );
        } else {
            let _ = services.SetHwndPropStr(
                hwnd,
                OBJID_CLIENT.0 as u32,
                CHILDID_SELF,
                Name_Property_GUID,
                &windows::core::HSTRING::from(name),
            );
        }
    }
}

/// The text or the description of a view changed: assistive technology
/// learns the new name, and live regions speak it.
pub fn content_changed(app: &Rc<App>, id: NodeId) {
    let listening = listening();
    let change = app
        .with_state(|state| {
            let reachable = reachable(&state.scene, id);
            let view = state.scene.views.get_mut(&id)?;
            let control = view.input.as_ref().and_then(|input| input.hwnd);
            // Nobody to tell: the name is read when somebody asks.
            if !listening && control.is_none() {
                return None;
            }
            let name = name_of(view);
            let live = view
                .accessibility
                .as_ref()
                .map_or(LiveRegion::Off, |accessibility| accessibility.live);
            if name == view.announced {
                return None;
            }
            let previous = std::mem::replace(&mut view.announced, name.clone());
            // A view assistive technology cannot reach raises nothing.
            (reachable || control.is_some()).then_some((name, previous, control, live))
        })
        .flatten();
    let Some((name, previous, control, live)) = change else {
        return;
    };
    if let Some(control) = control {
        annotate_control(control, &name);
        return;
    }
    // The first name of a view is read when it is visited; only a change
    // (or a live region) is worth an event.
    if previous.is_empty() && live == LiveRegion::Off {
        return;
    }
    let Some(provider) = simple_provider(app, id) else {
        return;
    };
    // SAFETY: raising events with a live provider; no state borrow is held.
    unsafe {
        let _ = UiaRaiseAutomationPropertyChangedEvent(
            &provider,
            UIA_NamePropertyId,
            &VARIANT::from(BSTR::from(previous)),
            &VARIANT::from(BSTR::from(name.as_str())),
        );
        if live != LiveRegion::Off && !name.is_empty() {
            let processing = if live == LiveRegion::Assertive {
                NotificationProcessing_ImportantAll
            } else {
                NotificationProcessing_All
            };
            let _ = UiaRaiseNotificationEvent(
                &provider,
                NotificationKind_Other,
                processing,
                &BSTR::from(name),
                &BSTR::from("pam-live-region"),
            );
        }
    }
}

fn toggle_state(checked: Checked) -> ToggleState {
    match checked {
        Checked::Unchecked => ToggleState_Off,
        Checked::Checked => ToggleState_On,
        Checked::Mixed => ToggleState_Indeterminate,
    }
}

/// The description of a view was replaced: the states that changed are
/// announced (a toggled switch, a selected tab, an expanded section…).
pub fn state_changed(app: &Rc<App>, id: NodeId, previous: Option<&Accessibility>) {
    if !listening() {
        return;
    }
    let current = app
        .with_state(|state| {
            if reachable(&state.scene, id) {
                state
                    .scene
                    .views
                    .get(&id)
                    .map(|view| view.accessibility.as_deref().cloned())
            } else {
                None
            }
        })
        .flatten();
    let Some(current) = current else { return };
    let plain = Accessibility::default();
    let (before, now) = (
        previous.unwrap_or(&plain),
        current.as_ref().unwrap_or(&plain),
    );
    let mut changes: Vec<(UIA_PROPERTY_ID, VARIANT, VARIANT)> = Vec::new();
    if before.checked != now.checked
        && let (Some(old), Some(new)) = (before.checked, now.checked)
    {
        changes.push((
            UIA_ToggleToggleStatePropertyId,
            VARIANT::from(toggle_state(old).0),
            VARIANT::from(toggle_state(new).0),
        ));
    }
    if selection_of(before) != selection_of(now)
        && let (Some(old), Some(new)) = (selection_of(before), selection_of(now))
    {
        changes.push((
            UIA_SelectionItemIsSelectedPropertyId,
            VARIANT::from(old),
            VARIANT::from(new),
        ));
    }
    if before.expanded != now.expanded
        && let (Some(old), Some(new)) = (before.expanded, now.expanded)
    {
        let state = |expanded: bool| {
            if expanded {
                ExpandCollapseState_Expanded.0
            } else {
                ExpandCollapseState_Collapsed.0
            }
        };
        changes.push((
            UIA_ExpandCollapseExpandCollapseStatePropertyId,
            VARIANT::from(state(old)),
            VARIANT::from(state(new)),
        ));
    }
    if before.range != now.range
        && let (Some((_, _, old)), Some((_, _, new))) = (before.range, now.range)
    {
        changes.push((
            UIA_RangeValueValuePropertyId,
            VARIANT::from(f64::from(old)),
            VARIANT::from(f64::from(new)),
        ));
    }
    if before.disabled != now.disabled {
        changes.push((
            UIA_IsEnabledPropertyId,
            VARIANT::from(!before.disabled),
            VARIANT::from(!now.disabled),
        ));
    }
    if changes.is_empty() {
        return;
    }
    let Some(provider) = simple_provider(app, id) else {
        return;
    };
    for (property, old, new) in &changes {
        // SAFETY: raising an event with a live provider; no state borrow is held.
        unsafe {
            let _ = UiaRaiseAutomationPropertyChangedEvent(&provider, *property, old, new);
        }
    }
}

/// Whether a screen reader is running.
pub fn screen_reader_active() -> bool {
    let mut active = BOOL(0);
    // SAFETY: the out pointer refers to a live BOOL.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETSCREENREADER,
            0,
            Some(std::ptr::from_mut(&mut active).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_ok() && active.as_bool()
}

/// `Backend::announce`: hands `message` to assistive technology through
/// the main window. Answers whether a screen reader is there to speak it.
pub fn announce(app: &Rc<App>, message: &str, assertive: bool) -> bool {
    let window = pam_desktop_ui::backend::MAIN_WINDOW;
    let Some(hwnd) = app
        .with_state(|state| state.scene.windows.get(&window).map(|entry| entry.hwnd))
        .flatten()
    else {
        return false;
    };
    let Some(root) = root_provider(app, hwnd, window) else {
        return false;
    };
    let processing = if assertive {
        NotificationProcessing_ImportantAll
    } else {
        NotificationProcessing_All
    };
    // SAFETY: raising an event with a live provider; no state borrow is held.
    let raised = unsafe {
        UiaRaiseNotificationEvent(
            &root,
            NotificationKind_Other,
            processing,
            &BSTR::from(message),
            &BSTR::from("pam-announcement"),
        )
    };
    raised.is_ok() && screen_reader_active()
}

/// A native text control was created: it takes the name of its view.
pub fn control_created(app: &Rc<App>, id: NodeId, hwnd: HWND) {
    let name = app
        .with_state(|state| state.scene.views.get(&id).map(name_of))
        .flatten()
        .unwrap_or_default();
    if !name.is_empty() {
        annotate_control(hwnd, &name);
    }
}

/// A view was destroyed: providers still held by clients answer "not available".
pub fn view_destroyed(_app: &Rc<App>, _id: NodeId) {}

//! Desktop shell on AppKit: the menu bar, context menus, the status item,
//! file panels, alerts, notifications, the clipboard, window commands,
//! cursors and global shortcuts.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAppearance, NSAppearanceNameAqua,
    NSAppearanceNameDarkAqua, NSApplication, NSCursor, NSEvent, NSEventMask, NSEventModifierFlags,
    NSFloatingWindowLevel, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSMenuItemValidation,
    NSModalResponse, NSModalResponseOK, NSNormalWindowLevel, NSOpenPanel, NSPasteboard,
    NSPasteboardTypeString, NSSavePanel, NSSharingServicePicker, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength, NSView, NSWindow, NSWindowStyleMask, NSWorkspace,
    NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{
    NSArray, NSBundle, NSCopying, NSError, NSNotification, NSOperationQueue, NSProcessInfo, NSURL,
    NSUUID,
};
use objc2_uniform_type_identifiers::UTType;
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use pam_desktop_ui::backend::{AlertButtonStyle, AlertConfig, BackendEvent, Modifiers};
use pam_desktop_ui::geometry::{Point, Size};
use pam_desktop_ui::i18n;
use pam_desktop_ui::shell::{
    Accelerator, Cursor, FileDialogConfig, FileDialogKind, MenuNode, MenuRole, NotificationConfig,
    Shortcut, TrayConfig, WindowCommand,
};

use crate::events::{self, Events, MainThread};
use crate::util::{cg_point, cg_size, ns_string};
use crate::window::window_state;

/// The system cursor of a CSS cursor; `None` keeps the arrow.
#[expect(
    deprecated,
    reason = "the replacements of the resize cursors need macOS 15"
)]
pub fn native_cursor(cursor: Cursor) -> Option<Retained<NSCursor>> {
    Some(match cursor {
        Cursor::Default => return None,
        Cursor::Pointer => NSCursor::pointingHandCursor(),
        Cursor::Text => NSCursor::IBeamCursor(),
        Cursor::Grab => NSCursor::openHandCursor(),
        Cursor::Grabbing => NSCursor::closedHandCursor(),
        Cursor::NotAllowed => NSCursor::operationNotAllowedCursor(),
        Cursor::Crosshair => NSCursor::crosshairCursor(),
        // AppKit has no public busy cursor; the arrow stays.
        Cursor::Wait => return None,
        Cursor::Move => NSCursor::openHandCursor(),
        Cursor::ResizeHorizontal => NSCursor::resizeLeftRightCursor(),
        Cursor::ResizeVertical => NSCursor::resizeUpDownCursor(),
    })
}

/// Whether an appearance is one of the dark ones.
pub fn is_dark(appearance: &NSAppearance) -> bool {
    // SAFETY: the appearance names are immutable strings exported by AppKit.
    let (light, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    let names = NSArray::from_slice(&[light, dark]);
    appearance
        .bestMatchFromAppearancesWithNames(&names)
        .is_some_and(|name| &*name == dark)
}

/// Forces light or dark chrome, or follows the system.
pub fn set_appearance(mtm: MainThreadMarker, dark: Option<bool>) {
    let appearance = dark.and_then(|dark| {
        // SAFETY: the appearance names are immutable strings exported by AppKit.
        NSAppearance::appearanceNamed(unsafe {
            if dark {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            }
        })
    });
    NSApplication::sharedApplication(mtm).setAppearance(appearance.as_deref());
}

/// The system's own appearance, whatever the application forces.
pub fn system_is_dark(mtm: MainThreadMarker, forced: bool) -> bool {
    if forced {
        // The effective appearance is the forced one: ask the preference
        // the system keeps for "dark mode".
        let style = objc2_foundation::NSUserDefaults::standardUserDefaults()
            .stringForKey(&ns_string("AppleInterfaceStyle"));
        style.is_some_and(|style| style.to_string().eq_ignore_ascii_case("dark"))
    } else {
        is_dark(&NSApplication::sharedApplication(mtm).effectiveAppearance())
    }
}

pub fn reduced_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// The registration of an observer block with a notification center.
pub type Registration = Retained<ProtocolObject<dyn NSObjectProtocol>>;

/// Tells the core when the display options of the system change (reduce
/// motion among them), until `unwatch_display_options` takes the
/// registration back.
pub fn watch_display_options(events: &Rc<Events>) -> Registration {
    let events = Rc::clone(events);
    let changed =
        RcBlock::new(move |_: NonNull<NSNotification>| events.push(BackendEvent::MetricsChanged));
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: the block only runs on the main queue, where the events it
    // captures live; the name is an immutable string exported by AppKit.
    unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &changed,
        )
    }
}

pub fn unwatch_display_options(registration: &Registration) {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: the object is a registration this center returned.
    unsafe { center.removeObserver(AsRef::<AnyObject>::as_ref(&**registration)) };
}

/// Opens the share picker of the system over `view`, at the pointer when
/// it is inside the view and in the middle of the view otherwise.
pub fn share(
    mtm: MainThreadMarker,
    view: &NSView,
    request: &pam_desktop_ui::shell::ShareRequest,
) -> bool {
    let text = (!request.text.is_empty()).then(|| ns_string(&request.text));
    let files: Vec<Retained<NSURL>> = request
        .files
        .iter()
        .map(|file| NSURL::fileURLWithPath(&ns_string(&file.to_string_lossy())))
        .collect();
    let mut items: Vec<&AnyObject> = Vec::with_capacity(files.len() + 1);
    if let Some(text) = &text {
        items.push(text.as_ref());
    }
    items.extend(files.iter().map(|file| -> &AnyObject { file.as_ref() }));
    if items.is_empty() {
        return false;
    }
    let items = NSArray::from_slice(&items);
    // SAFETY: strings and file URLs write themselves to a pasteboard,
    // which is what the picker requires of its items.
    let picker = unsafe { NSSharingServicePicker::initWithItems(mtm.alloc(), &items) };
    let bounds = view.bounds();
    let pointer = view
        .window()
        .map(|window| view.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None));
    let inside = |point: &CGPoint| {
        point.x >= bounds.origin.x
            && point.y >= bounds.origin.y
            && point.x <= bounds.origin.x + bounds.size.width
            && point.y <= bounds.origin.y + bounds.size.height
    };
    let anchor = pointer.filter(inside).unwrap_or_else(|| {
        CGPoint::new(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
        )
    });
    picker.showRelativeToRect_ofView_preferredEdge(
        CGRect::new(anchor, CGSize::new(1.0, 1.0)),
        view,
        objc2_foundation::NSRectEdge::MinY,
    );
    true
}

pub fn application_name() -> String {
    NSProcessInfo::processInfo().processName().to_string()
}

fn modifier_mask(modifiers: Modifiers) -> NSEventModifierFlags {
    let mut mask = NSEventModifierFlags::empty();
    if modifiers.meta {
        mask |= NSEventModifierFlags::Command;
    }
    if modifiers.control {
        mask |= NSEventModifierFlags::Control;
    }
    if modifiers.alt {
        mask |= NSEventModifierFlags::Option;
    }
    if modifiers.shift {
        mask |= NSEventModifierFlags::Shift;
    }
    mask
}

/// The key equivalent string of a W3C key name.
fn key_equivalent(key: &str) -> String {
    let function = |code: u32| char::from_u32(code).map(String::from).unwrap_or_default();
    match key {
        "Enter" => String::from("\r"),
        "Tab" => String::from("\t"),
        "Escape" => String::from("\u{1b}"),
        "Backspace" => String::from("\u{8}"),
        "Delete" => String::from("\u{7f}"),
        "ArrowUp" => function(0xF700),
        "ArrowDown" => function(0xF701),
        "ArrowLeft" => function(0xF702),
        "ArrowRight" => function(0xF703),
        "Insert" => function(0xF727),
        "Home" => function(0xF729),
        "End" => function(0xF72B),
        "PageUp" => function(0xF72C),
        "PageDown" => function(0xF72D),
        other => match other
            .strip_prefix('F')
            .and_then(|number| number.parse::<u32>().ok())
        {
            Some(number @ 1..=24) => function(0xF704 + number - 1),
            _ => other.to_lowercase(),
        },
    }
}

/// The shortcut the platform gives a role when the application gives none.
fn role_accelerator(role: MenuRole) -> Option<Accelerator> {
    let command = Modifiers {
        meta: true,
        ..Modifiers::default()
    };
    let (modifiers, key) = match role {
        MenuRole::Quit => (command, "q"),
        MenuRole::Preferences => (command, ","),
        MenuRole::Undo => (command, "z"),
        MenuRole::Redo => (
            Modifiers {
                shift: true,
                ..command
            },
            "z",
        ),
        MenuRole::Cut => (command, "x"),
        MenuRole::Copy => (command, "c"),
        MenuRole::Paste => (command, "v"),
        MenuRole::SelectAll => (command, "a"),
        MenuRole::Minimize => (command, "m"),
        MenuRole::CloseWindow => (command, "w"),
        MenuRole::Hide => (command, "h"),
        MenuRole::HideOthers => (
            Modifiers {
                alt: true,
                ..command
            },
            "h",
        ),
        MenuRole::ToggleFullScreen => (
            Modifiers {
                control: true,
                ..command
            },
            "f",
        ),
        MenuRole::About | MenuRole::Zoom | MenuRole::ShowAll => return None,
    };
    Some(Accelerator {
        modifiers,
        key: key.to_owned(),
    })
}

/// The standard action of a role, sent down the responder chain.
fn role_selector(role: MenuRole) -> Option<Sel> {
    Some(match role {
        MenuRole::Quit => sel!(terminate:),
        MenuRole::About => sel!(orderFrontStandardAboutPanel:),
        // The application shows its own settings.
        MenuRole::Preferences => return None,
        MenuRole::Undo => sel!(undo:),
        MenuRole::Redo => sel!(redo:),
        MenuRole::Cut => sel!(cut:),
        MenuRole::Copy => sel!(copy:),
        MenuRole::Paste => sel!(paste:),
        MenuRole::SelectAll => sel!(selectAll:),
        MenuRole::Minimize => sel!(performMiniaturize:),
        MenuRole::Zoom => sel!(performZoom:),
        MenuRole::ToggleFullScreen => sel!(toggleFullScreen:),
        MenuRole::CloseWindow => sel!(performClose:),
        MenuRole::Hide => sel!(hide:),
        MenuRole::HideOthers => sel!(hideOtherApplications:),
        MenuRole::ShowAll => sel!(unhideAllApplications:),
    })
}

#[derive(Clone, Copy, PartialEq)]
enum MenuKind {
    /// Chosen items are reported as `MenuAction`.
    Bar,
    /// Like the bar; opening the menu also reports `TrayActivated`.
    Tray,
    /// The chosen item is kept for the caller of `popup_menu`.
    Popup,
}

struct MenuEntry {
    id: String,
    enabled: bool,
    checkable: bool,
}

struct MenuIvars {
    events: Rc<Events>,
    kind: MenuKind,
    /// Indexed by the tag of the menu items.
    entries: RefCell<Vec<MenuEntry>>,
    chosen: RefCell<Option<String>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements. Main thread only.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuIvars]
    /// The target of the items of one menu.
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(menuAction:))]
        fn menu_action(&self, sender: &NSMenuItem) {
            let ivars = self.ivars();
            let chosen = {
                let entries = ivars.entries.borrow();
                let Some(entry) = usize::try_from(sender.tag()).ok().and_then(|index| entries.get(index)) else { return };
                if entry.checkable {
                    // Check items flip at once; the application confirms
                    // with the next menu it sends.
                    sender.setState(if sender.state() == 0 { 1 } else { 0 });
                }
                entry.id.clone()
            };
            match ivars.kind {
                MenuKind::Popup => *ivars.chosen.borrow_mut() = Some(chosen),
                MenuKind::Bar | MenuKind::Tray => ivars.events.push(BackendEvent::MenuAction { id: chosen }),
            }
        }

        /// The status item itself was clicked (it has no menu).
        #[unsafe(method(statusItemClicked:))]
        fn status_item_clicked(&self, _sender: Option<&AnyObject>) {
            self.ivars().events.push(BackendEvent::TrayActivated);
        }
    }

    unsafe impl NSObjectProtocol for MenuTarget {}

    unsafe impl NSMenuItemValidation for MenuTarget {
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let entries = self.ivars().entries.borrow();
            usize::try_from(item.tag()).ok().and_then(|index| entries.get(index)).is_none_or(|entry| entry.enabled)
        }
    }

    unsafe impl NSMenuDelegate for MenuTarget {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &NSMenu) {
            let ivars = self.ivars();
            if ivars.kind == MenuKind::Tray {
                ivars.events.push_later(BackendEvent::TrayActivated);
            }
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker, events: &Rc<Events>, kind: MenuKind) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuIvars {
            events: Rc::clone(events),
            kind,
            entries: RefCell::new(Vec::new()),
            chosen: RefCell::new(None),
        });
        // SAFETY: `init` is the designated initializer of `NSObject`.
        unsafe { msg_send![super(this), init] }
    }

    /// Registers an item that reports `id` and returns its tag.
    fn register(&self, id: &str, enabled: bool, checkable: bool) -> isize {
        let mut entries = self.ivars().entries.borrow_mut();
        entries.push(MenuEntry {
            id: id.to_owned(),
            enabled,
            checkable,
        });
        (entries.len() - 1) as isize
    }

    fn take_chosen(&self) -> Option<String> {
        self.ivars().chosen.borrow_mut().take()
    }
}

fn menu_item(
    mtm: MainThreadMarker,
    label: &str,
    action: Option<Sel>,
    accelerator: Option<&Accelerator>,
) -> Retained<NSMenuItem> {
    let key = accelerator
        .map(|accelerator| key_equivalent(&accelerator.key))
        .unwrap_or_default();
    // SAFETY: the action is a selector taking the sender, or nil.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &ns_string(label),
            action,
            &ns_string(&key),
        )
    };
    if let Some(accelerator) = accelerator {
        item.setKeyEquivalentModifierMask(modifier_mask(accelerator.modifiers));
    }
    item
}

/// Adds one node to a native menu. Items without a platform role report
/// through `target`.
fn add_node(mtm: MainThreadMarker, menu: &NSMenu, node: &MenuNode, target: &MenuTarget) {
    match node {
        MenuNode::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
        MenuNode::Submenu {
            label,
            enabled,
            children,
        } => {
            let item = menu_item(mtm, label, None, None);
            item.setSubmenu(Some(&build_menu(mtm, label, children, target)));
            item.setEnabled(*enabled);
            menu.addItem(&item);
        }
        MenuNode::Item {
            id,
            label,
            accelerator,
            enabled,
            checked,
            role,
            ..
        } => {
            let default_accelerator = role.and_then(role_accelerator);
            let accelerator = accelerator.as_ref().or(default_accelerator.as_ref());
            // A role item without a label gets the platform's, translated.
            let named;
            let label = match role {
                Some(role) if label.trim().is_empty() => {
                    named = i18n::menu_role(*role, Some(&application_name()));
                    named.as_str()
                }
                _ => label.as_str(),
            };
            let item = match role.and_then(role_selector) {
                // Sent to the first responder that implements it; a
                // disabled role item simply has no action.
                Some(selector) => menu_item(mtm, label, enabled.then_some(selector), accelerator),
                None => {
                    let reported = match role {
                        Some(MenuRole::Preferences) => "role:preferences",
                        _ => id.as_str(),
                    };
                    let item = menu_item(mtm, label, Some(sel!(menuAction:)), accelerator);
                    item.setTag(target.register(reported, *enabled, checked.is_some()));
                    // SAFETY: the target implements `menuAction:` and is
                    // kept alive by whoever owns the menu.
                    unsafe { item.setTarget(Some(target)) };
                    item
                }
            };
            if let Some(checked) = checked {
                item.setState(isize::from(*checked));
            }
            menu.addItem(&item);
        }
    }
}

fn build_menu(
    mtm: MainThreadMarker,
    title: &str,
    nodes: &[MenuNode],
    target: &MenuTarget,
) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns_string(title));
    for node in nodes {
        add_node(mtm, &menu, node, target);
    }
    menu
}

fn role_item(label: &str, role: MenuRole) -> MenuNode {
    MenuNode::Item {
        id: String::new(),
        label: label.to_owned(),
        accelerator: None,
        enabled: true,
        checked: None,
        role: Some(role),
        destructive: false,
    }
}

/// The menu named after the application, which every Mac program has.
fn application_menu(name: &str) -> MenuNode {
    MenuNode::Submenu {
        label: name.to_owned(),
        enabled: true,
        children: vec![
            role_item(
                &i18n::menu_role(MenuRole::About, Some(name)),
                MenuRole::About,
            ),
            MenuNode::Separator,
            role_item(&i18n::menu_role(MenuRole::Hide, Some(name)), MenuRole::Hide),
            role_item(i18n::text(i18n::Text::HideOthers), MenuRole::HideOthers),
            role_item(i18n::text(i18n::Text::ShowAll), MenuRole::ShowAll),
            MenuNode::Separator,
            role_item(&i18n::menu_role(MenuRole::Quit, Some(name)), MenuRole::Quit),
        ],
    }
}

/// What the menu bar holds when the application defines none: without an
/// Edit menu, text fields would have no copy and paste shortcuts.
fn default_menus() -> Vec<MenuNode> {
    vec![
        MenuNode::Submenu {
            label: i18n::text(i18n::Text::Edit).to_owned(),
            enabled: true,
            children: vec![
                role_item(i18n::text(i18n::Text::Undo), MenuRole::Undo),
                role_item(i18n::text(i18n::Text::Redo), MenuRole::Redo),
                MenuNode::Separator,
                role_item(i18n::text(i18n::Text::Cut), MenuRole::Cut),
                role_item(i18n::text(i18n::Text::Copy), MenuRole::Copy),
                role_item(i18n::text(i18n::Text::Paste), MenuRole::Paste),
                role_item(i18n::text(i18n::Text::SelectAll), MenuRole::SelectAll),
            ],
        },
        MenuNode::Submenu {
            label: i18n::text(i18n::Text::Window).to_owned(),
            enabled: true,
            children: vec![
                role_item(i18n::text(i18n::Text::Minimize), MenuRole::Minimize),
                role_item(i18n::text(i18n::Text::Zoom), MenuRole::Zoom),
                role_item(
                    i18n::text(i18n::Text::EnterFullScreen),
                    MenuRole::ToggleFullScreen,
                ),
                MenuNode::Separator,
                role_item(i18n::text(i18n::Text::Close), MenuRole::CloseWindow),
            ],
        },
    ]
}

fn is_application_menu(node: &MenuNode) -> bool {
    let MenuNode::Submenu { children, .. } = node else {
        return false;
    };
    children.iter().any(|child| {
        matches!(
            child,
            MenuNode::Item {
                role: Some(
                    MenuRole::About
                        | MenuRole::Quit
                        | MenuRole::Preferences
                        | MenuRole::Hide
                        | MenuRole::HideOthers
                        | MenuRole::ShowAll
                ),
                ..
            }
        )
    })
}

struct Tray {
    item: Retained<NSStatusItem>,
    /// Kept alive: menu items and the button reference it weakly.
    _target: Retained<MenuTarget>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements. The only ivar is
    // a thread-safe handle: the system calls the delegate on any thread.
    #[unsafe(super(NSObject))]
    #[ivars = MainThread]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        /// Notifications also show while the application is in front.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        /// The user activated a notification.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            let id = response.notification().request().identifier().to_string();
            self.ivars()
                .run(move || events::post(BackendEvent::NotificationActivated { id }));
            completion.call(());
        }
    }
);

pub struct Shell {
    mtm: MainThreadMarker,
    events: Rc<Events>,
    /// Target of the menu bar's items.
    menu_target: Option<Retained<MenuTarget>>,
    tray: Option<Tray>,
    notifications: Option<Retained<NotificationDelegate>>,
    /// The monitor of key presses in other applications, while any global
    /// shortcut is registered.
    global_monitor: Option<Retained<AnyObject>>,
    global_shortcuts: Rc<RefCell<Vec<Shortcut>>>,
}

impl Shell {
    pub fn new(mtm: MainThreadMarker, events: Rc<Events>) -> Self {
        Self {
            mtm,
            events,
            menu_target: None,
            tray: None,
            notifications: None,
            global_monitor: None,
            global_shortcuts: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn set_menu_bar(&mut self, menu: &[MenuNode]) {
        let mtm = self.mtm;
        let name = application_name();
        let mut nodes: Vec<MenuNode> = if menu.is_empty() {
            default_menus()
        } else {
            menu.to_vec()
        };
        // The first menu of the bar is the application's.
        if !nodes.first().is_some_and(is_application_menu) {
            nodes.insert(0, application_menu(&name));
        }
        let target = MenuTarget::new(mtm, &self.events, MenuKind::Bar);
        let bar = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns_string(&name));
        // A bare item on the bar is shown by AppKit as a title.
        for node in &nodes {
            add_node(mtm, &bar, node, &target);
        }
        NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
        self.menu_target = Some(target);
    }

    /// Opens a context menu at `position` of `view`. The menu tracks the
    /// pointer in its own run loop, so it opens once the runtime is idle.
    pub fn popup_menu(
        &self,
        view: &NSView,
        position: Point,
        menu: &[MenuNode],
        done: Box<dyn FnOnce(Option<String>)>,
    ) {
        let target = MenuTarget::new(self.mtm, &self.events, MenuKind::Popup);
        let native = build_menu(self.mtm, "", menu, &target);
        let view = view.retain();
        let events = Rc::clone(&self.events);
        self.events.defer(move || {
            native.popUpMenuPositioningItem_atLocation_inView(
                None,
                cg_point(position),
                Some(&view),
            );
            // The chosen item's action is delivered by the run loop: read
            // the choice on its next turn.
            events.defer(move || done(target.take_chosen()));
        });
    }

    pub fn set_tray(&mut self, config: Option<&TrayConfig>) {
        let mtm = self.mtm;
        let Some(config) = config else {
            if let Some(tray) = self.tray.take() {
                NSStatusBar::systemStatusBar().removeStatusItem(&tray.item);
            }
            return;
        };
        let item = match self.tray.take() {
            Some(tray) => tray.item,
            None => NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength),
        };
        let target = MenuTarget::new(mtm, &self.events, MenuKind::Tray);
        if let Some(button) = item.button(mtm) {
            let image = match &config.icon {
                Some(path) => NSImage::initWithContentsOfFile(
                    NSImage::alloc(),
                    &ns_string(&path.to_string_lossy()),
                ),
                // A copy: resizing the shared image would resize the Dock's.
                None => NSApplication::sharedApplication(mtm)
                    .applicationIconImage()
                    .map(|icon| icon.copy()),
            };
            if let Some(image) = &image {
                // The height of the menu bar's icons.
                image.setSize(CGSize::new(18.0, 18.0));
            }
            button.setImage(image.as_deref());
            button.setToolTip(
                (!config.tooltip.is_empty())
                    .then(|| ns_string(&config.tooltip))
                    .as_deref(),
            );
            // SAFETY: the target implements `statusItemClicked:` and is
            // kept alive by the tray.
            unsafe {
                button.setTarget(Some(&target));
                button.setAction(Some(sel!(statusItemClicked:)));
            }
        }
        if config.menu.is_empty() {
            item.setMenu(None);
        } else {
            let menu = build_menu(mtm, "", &config.menu, &target);
            menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
            item.setMenu(Some(&menu));
        }
        self.tray = Some(Tray {
            item,
            _target: target,
        });
    }

    pub fn pick_files(
        &self,
        parent: Option<&NSWindow>,
        config: &FileDialogConfig,
        done: Box<dyn FnOnce(Vec<PathBuf>)>,
    ) {
        let mtm = self.mtm;
        let (panel, open): (Retained<NSSavePanel>, Option<Retained<NSOpenPanel>>) =
            if config.kind == FileDialogKind::Save {
                (NSSavePanel::savePanel(mtm), None)
            } else {
                let open = NSOpenPanel::openPanel(mtm);
                let folders = config.kind == FileDialogKind::SelectFolder;
                open.setCanChooseFiles(!folders);
                open.setCanChooseDirectories(folders);
                open.setAllowsMultipleSelection(config.kind == FileDialogKind::OpenMultiple);
                (Retained::into_super(open.clone()), Some(open))
            };
        panel.setCanCreateDirectories(true);
        if !config.title.is_empty() {
            // Sheets have no title bar: the message line shows the title.
            panel.setTitle(Some(&ns_string(&config.title)));
            panel.setMessage(Some(&ns_string(&config.title)));
        }
        if !config.default_name.is_empty() {
            panel.setNameFieldStringValue(&ns_string(&config.default_name));
        }
        if let Some(directory) = &config.directory {
            panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&ns_string(
                &directory.to_string_lossy(),
            ))));
        }
        let extensions: Vec<&str> = config
            .filters
            .iter()
            .flat_map(|filter| filter.extensions.iter().map(String::as_str))
            .collect();
        if !extensions.is_empty()
            && !extensions.contains(&"*")
            && config.kind != FileDialogKind::SelectFolder
        {
            let types: Vec<Retained<UTType>> = extensions
                .iter()
                .filter_map(|extension| {
                    UTType::typeWithFilenameExtension(&ns_string(extension.trim_start_matches('.')))
                })
                .collect();
            if !types.is_empty() {
                panel.setAllowedContentTypes(&NSArray::from_retained_slice(&types));
            }
        }
        let done = Cell::new(Some(done));
        let chosen = panel.clone();
        let handler = RcBlock::new(move |response: NSModalResponse| {
            let Some(done) = done.take() else { return };
            let urls: Vec<Retained<NSURL>> = if response != NSModalResponseOK {
                Vec::new()
            } else {
                match &open {
                    Some(open) => open.URLs().to_vec(),
                    None => chosen.URL().into_iter().collect(),
                }
            };
            done(
                urls.iter()
                    .filter_map(|url| url.path())
                    .map(|path| PathBuf::from(path.to_string()))
                    .collect(),
            );
        });
        match parent {
            Some(window) => panel.beginSheetModalForWindow_completionHandler(window, &handler),
            None => panel.beginWithCompletionHandler(&handler),
        }
    }

    pub fn show_alert(
        &self,
        parent: Option<&NSWindow>,
        config: &AlertConfig,
        done: Box<dyn FnOnce(Option<usize>)>,
    ) {
        let alert = NSAlert::new(self.mtm);
        alert.setMessageText(&ns_string(&config.title));
        alert.setInformativeText(&ns_string(&config.message));
        let count = config.buttons.len().max(1);
        if config.buttons.is_empty() {
            alert.addButtonWithTitle(&ns_string(i18n::text(i18n::Text::Ok)));
        }
        // The first button that is not Cancel answers to Return; AppKit
        // gives Return to the first one added.
        let default = config
            .buttons
            .iter()
            .position(|button| button.style != AlertButtonStyle::Cancel)
            .unwrap_or(0);
        for (index, button) in config.buttons.iter().enumerate() {
            let native = alert.addButtonWithTitle(&ns_string(&button.label));
            match button.style {
                AlertButtonStyle::Cancel => native.setKeyEquivalent(&ns_string("\u{1b}")),
                AlertButtonStyle::Destructive => {
                    if native.respondsToSelector(sel!(setHasDestructiveAction:)) {
                        native.setHasDestructiveAction(true);
                    }
                    native.setKeyEquivalent(&ns_string(if index == default { "\r" } else { "" }));
                }
                AlertButtonStyle::Default => {
                    native.setKeyEquivalent(&ns_string(if index == default { "\r" } else { "" }))
                }
            }
        }
        let answer = move |response: NSModalResponse| {
            usize::try_from(response - NSAlertFirstButtonReturn)
                .ok()
                .filter(|index| *index < count)
        };
        match parent {
            Some(window) => {
                let done = Cell::new(Some(done));
                let handler = RcBlock::new(move |response: NSModalResponse| {
                    if let Some(done) = done.take() {
                        done(answer(response));
                    }
                });
                alert.beginSheetModalForWindow_completionHandler(window, Some(&handler));
            }
            // Without a window the alert is application modal: it runs its
            // own loop, so it opens once the runtime is idle.
            None => self.events.defer(move || done(answer(alert.runModal()))),
        }
    }

    pub fn clipboard_set_text(&self, text: &str) {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        // SAFETY: the type name is an immutable string exported by AppKit.
        pasteboard.setString_forType(&ns_string(text), unsafe { NSPasteboardTypeString });
    }

    pub fn clipboard_text(&self, done: Box<dyn FnOnce(Option<String>)>) {
        // SAFETY: the type name is an immutable string exported by AppKit.
        let text = NSPasteboard::generalPasteboard()
            .stringForType(unsafe { NSPasteboardTypeString })
            .map(|text| text.to_string());
        self.events.defer(move || done(text));
    }

    /// Posts a notification through the notification center, asking the
    /// user for permission the first time. Without a bundle identifier
    /// (a bare executable) the system offers no notifications at all.
    pub fn notify(&mut self, config: &NotificationConfig) {
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            return;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        if self.notifications.is_none() {
            let this = NotificationDelegate::alloc().set_ivars(self.events.main().clone());
            // SAFETY: `init` is the designated initializer of `NSObject`.
            let delegate: Retained<NotificationDelegate> = unsafe { msg_send![super(this), init] };
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            self.notifications = Some(delegate);
        }
        let (id, title, body) = (config.id.clone(), config.title.clone(), config.body.clone());
        // The answer arrives on a queue of the system: the block only
        // touches what it owns.
        let handler = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            if !granted.as_bool() {
                return;
            }
            let content = UNMutableNotificationContent::new();
            content.setTitle(&ns_string(&title));
            content.setBody(&ns_string(&body));
            content.setSound(Some(&UNNotificationSound::defaultSound()));
            let identifier = if id.is_empty() {
                NSUUID::new().UUIDString()
            } else {
                ns_string(&id)
            };
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &identifier,
                &content,
                None,
            );
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, None);
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &handler,
        );
    }

    /// Shortcuts of the focused window are matched by the core. Global
    /// ones are watched while other applications are in front, which the
    /// system only allows to applications trusted for accessibility.
    pub fn set_shortcuts(&mut self, shortcuts: &[Shortcut]) {
        let global: Vec<Shortcut> = shortcuts
            .iter()
            .filter(|shortcut| shortcut.global)
            .cloned()
            .collect();
        let wanted = !global.is_empty();
        *self.global_shortcuts.borrow_mut() = global;
        if !wanted {
            if let Some(monitor) = self.global_monitor.take() {
                // SAFETY: the monitor was returned by `addGlobalMonitor…`.
                unsafe { NSEvent::removeMonitor(&monitor) };
            }
        } else if self.global_monitor.is_none() {
            let (registered, events) = (Rc::clone(&self.global_shortcuts), Rc::clone(&self.events));
            let handler = RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| {
                // SAFETY: AppKit passes a valid event for the duration of the call.
                let event = unsafe { event.as_ref() };
                let (key, modifiers) = (
                    events::key_name(event),
                    events::modifiers(event.modifierFlags()),
                );
                let fired = registered
                    .borrow()
                    .iter()
                    .find(|shortcut| shortcut.accelerator.matches(&key, modifiers))
                    .map(|shortcut| shortcut.id.clone());
                if let Some(id) = fired {
                    events.push(BackendEvent::Shortcut { id });
                }
            });
            self.global_monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                NSEventMask::KeyDown,
                &handler,
            );
        }
    }

    /// Releases what outlives the windows: the status item and the monitor.
    pub fn shutdown(&mut self) {
        self.set_tray(None);
        self.set_shortcuts(&[]);
        if self.notifications.take().is_some()
            && NSBundle::mainBundle().bundleIdentifier().is_some()
        {
            UNUserNotificationCenter::currentNotificationCenter().setDelegate(None);
        }
    }
}

pub fn window_command(mtm: MainThreadMarker, window: &NSWindow, command: &WindowCommand) {
    let state = window_state(window);
    match command {
        WindowCommand::SetTitle(title) => window.setTitle(&ns_string(title)),
        WindowCommand::SetSize(size) => window.setContentSize(cg_size(*size)),
        WindowCommand::SetMinSize(size) => {
            window.setContentMinSize(cg_size(size.unwrap_or(Size::ZERO)))
        }
        WindowCommand::SetResizable(resizable) => {
            let mut style = window.styleMask();
            style.set(NSWindowStyleMask::Resizable, *resizable);
            window.setStyleMask(style);
        }
        WindowCommand::Minimize => window.miniaturize(None),
        WindowCommand::Maximize => {
            if !state.maximized && !state.full_screen {
                window.zoom(None);
            }
        }
        WindowCommand::Restore => {
            if state.full_screen {
                window.toggleFullScreen(None);
            } else if state.minimized {
                window.deminiaturize(None);
            } else if state.maximized {
                window.zoom(None);
            } else {
                window.makeKeyAndOrderFront(None);
            }
        }
        WindowCommand::SetFullScreen(enabled) => {
            if state.full_screen != *enabled {
                window.toggleFullScreen(None);
            }
        }
        WindowCommand::Focus => {
            #[expect(deprecated, reason = "the replacement needs macOS 14")]
            NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
            window.makeKeyAndOrderFront(None);
        }
        WindowCommand::Center => window.center(),
        WindowCommand::SetAlwaysOnTop(enabled) => window.setLevel(if *enabled {
            NSFloatingWindowLevel
        } else {
            NSNormalWindowLevel
        }),
        WindowCommand::Show => window.makeKeyAndOrderFront(None),
        WindowCommand::Hide => window.orderOut(None),
        WindowCommand::Close => window.close(),
        // The Dock icon shows the progress as its badge.
        WindowCommand::SetProgress(progress) => {
            let badge = progress.map(|progress| {
                ns_string(&format!(
                    "{}%",
                    (progress.clamp(0.0, 1.0) * 100.0).round() as u32
                ))
            });
            NSApplication::sharedApplication(mtm)
                .dockTile()
                .setBadgeLabel(badge.as_deref());
        }
    }
}

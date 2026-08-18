use std::collections::HashMap;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use pam_desktop_protocol::{
    Bootstrap, Effect, EffectKind, MenuConfig, MenuItemConfig, MenuItemKind, ShellConfig,
    ShortcutState, TrayCloseBehavior,
};
use serde_json::Value;
use winit::event_loop::EventLoopProxy;

use crate::host_event::{HostEvent, ShellEvent};

pub struct NativeShell {
    hotkey_manager: Option<GlobalHotKeyManager>,
    hotkeys: Vec<HotKey>,
    shortcut_ids: HashMap<u32, String>,
    tray: Option<PlatformTray>,
    close_behavior: TrayCloseBehavior,
}

type PreparedHotkeys = (
    Option<GlobalHotKeyManager>,
    Vec<HotKey>,
    HashMap<u32, String>,
);

impl NativeShell {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            hotkey_manager: None,
            hotkeys: Vec::new(),
            shortcut_ids: HashMap::new(),
            tray: None,
            close_behavior: TrayCloseBehavior::Exit,
        }
    }

    pub fn install_event_handlers(proxy: &EventLoopProxy<HostEvent>) {
        let shortcut_proxy = proxy.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            let state = match event.state {
                HotKeyState::Pressed => ShortcutState::Pressed,
                HotKeyState::Released => ShortcutState::Released,
            };
            let _ = shortcut_proxy.send_event(HostEvent::Shell(ShellEvent::Shortcut {
                native_id: event.id,
                state,
            }));
        }));
        install_platform_event_handlers(proxy);
    }

    pub fn prepare(
        bootstrap: &Bootstrap,
        event_proxy: EventLoopProxy<HostEvent>,
    ) -> Result<Self, String> {
        let (hotkey_manager, hotkeys, shortcut_ids) =
            prepare_hotkeys(&bootstrap.shell, &event_proxy)?;
        let tray = bootstrap
            .shell
            .tray
            .as_ref()
            .map(|tray| {
                let menu = bootstrap
                    .shell
                    .menus
                    .iter()
                    .find(|menu| menu.id == tray.menu_id)
                    .expect("the tray menu reference was validated");
                PlatformTray::prepare(
                    &bootstrap.manifest.identifier,
                    &bootstrap.manifest.name,
                    tray.tooltip.clone(),
                    menu,
                    event_proxy,
                )
            })
            .transpose()?;
        Ok(Self {
            hotkey_manager,
            hotkeys,
            shortcut_ids,
            tray,
            close_behavior: bootstrap
                .shell
                .tray
                .as_ref()
                .map_or(TrayCloseBehavior::Exit, |tray| tray.close_behavior),
        })
    }

    #[must_use]
    pub const fn close_behavior(&self) -> TrayCloseBehavior {
        self.close_behavior
    }

    pub fn dispatch(&self, event: ShellEvent) -> Option<(&'static str, Value)> {
        match event {
            ShellEvent::MenuSelected(id) => {
                Some(("pam.menu.selected", serde_json::json!({"id": id})))
            }
            ShellEvent::TrayActivated { button } => {
                Some(("pam.tray.activated", serde_json::json!({"button": button})))
            }
            ShellEvent::Shortcut { native_id, state } => {
                self.shortcut_ids.get(&native_id).map(|id| {
                    (
                        "pam.shortcut.changed",
                        serde_json::json!({"id": id, "state": state}),
                    )
                })
            }
        }
    }

    pub fn apply_effect(&mut self, effect: &Effect) -> Result<bool, String> {
        let Some(tray) = &mut self.tray else {
            return match effect.kind {
                EffectKind::SetMenuItemEnabled
                | EffectKind::SetMenuItemChecked
                | EffectKind::SetTrayVisible => {
                    Err("native shell effects require a configured tray".to_owned())
                }
                _ => Ok(false),
            };
        };
        match effect.kind {
            EffectKind::SetMenuItemEnabled => {
                let id = effect
                    .payload
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "menu enabled effect is missing its item id".to_owned())?;
                let enabled = effect
                    .payload
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| "menu enabled effect is missing its state".to_owned())?;
                tray.set_menu_enabled(id, enabled)?;
                Ok(true)
            }
            EffectKind::SetMenuItemChecked => {
                let id = effect
                    .payload
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "menu checked effect is missing its item id".to_owned())?;
                let checked = effect
                    .payload
                    .get("checked")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| "menu checked effect is missing its state".to_owned())?;
                tray.set_menu_checked(id, checked)?;
                Ok(true)
            }
            EffectKind::SetTrayVisible => {
                let visible = effect
                    .payload
                    .get("visible")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| "tray visibility effect is missing its state".to_owned())?;
                tray.set_visible(visible)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

impl Drop for NativeShell {
    fn drop(&mut self) {
        if let Some(manager) = &self.hotkey_manager
            && let Err(error) = manager.unregister_all(&self.hotkeys)
        {
            eprintln!("pam-desktop: cannot unregister global shortcuts: {error}");
        }
    }
}

fn prepare_hotkeys(
    shell: &ShellConfig,
    _event_proxy: &EventLoopProxy<HostEvent>,
) -> Result<PreparedHotkeys, String> {
    if shell.shortcuts.is_empty() {
        return Ok((None, Vec::new(), HashMap::new()));
    }
    let manager = match GlobalHotKeyManager::new() {
        Ok(manager) => manager,
        Err(error) => {
            eprintln!("pam-desktop: global shortcuts are unavailable: {error}");
            return Ok((None, Vec::new(), HashMap::new()));
        }
    };
    let mut hotkeys = Vec::with_capacity(shell.shortcuts.len());
    let mut ids = HashMap::with_capacity(shell.shortcuts.len());
    for shortcut in &shell.shortcuts {
        let hotkey = shortcut.accelerator.parse::<HotKey>().map_err(|error| {
            format!(
                "global shortcut {:?} has an unsupported accelerator {:?}: {error}",
                shortcut.id, shortcut.accelerator
            )
        })?;
        match manager.register(hotkey) {
            Ok(()) => {
                ids.insert(hotkey.id(), shortcut.id.clone());
                hotkeys.push(hotkey);
            }
            Err(error) => {
                eprintln!(
                    "pam-desktop: global shortcut {:?} ({}) is unavailable: {error}",
                    shortcut.id, shortcut.accelerator
                );
            }
        }
    }
    Ok((Some(manager), hotkeys, ids))
}

#[cfg(target_os = "linux")]
mod platform {
    use ksni::blocking::{Handle, TrayMethods as _};
    use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
    use ksni::{MenuItem, Status, ToolTip, Tray};

    use super::{EventLoopProxy, HostEvent, MenuConfig, MenuItemConfig, MenuItemKind, ShellEvent};

    pub struct PlatformTray {
        handle: Handle<LinuxTray>,
    }

    impl PlatformTray {
        pub fn prepare(
            identifier: &str,
            name: &str,
            tooltip: String,
            menu: &MenuConfig,
            event_proxy: EventLoopProxy<HostEvent>,
        ) -> Result<Self, String> {
            let tray = LinuxTray {
                identifier: identifier.to_owned(),
                name: name.to_owned(),
                tooltip,
                menu: menu.clone(),
                visible: true,
                event_proxy,
            };
            let handle = tray
                .assume_sni_available(true)
                .spawn()
                .map_err(|error| format!("cannot create Linux status notifier tray: {error}"))?;
            Ok(Self { handle })
        }

        pub fn set_menu_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
            self.handle
                .update(|tray| set_item_enabled(&mut tray.menu.items, id, enabled))
                .flatten()
                .ok_or_else(|| format!("menu item {id:?} was not found"))
        }

        pub fn set_menu_checked(&self, id: &str, checked: bool) -> Result<(), String> {
            self.handle
                .update(|tray| set_item_checked(&mut tray.menu.items, id, checked))
                .flatten()
                .ok_or_else(|| format!("checkbox menu item {id:?} was not found"))
        }

        pub fn set_visible(&self, visible: bool) -> Result<(), String> {
            self.handle
                .update(|tray| tray.visible = visible)
                .ok_or_else(|| "Linux tray service is unavailable".to_owned())
        }
    }

    impl Drop for PlatformTray {
        fn drop(&mut self) {
            self.handle.shutdown().wait();
        }
    }

    struct LinuxTray {
        identifier: String,
        name: String,
        tooltip: String,
        menu: MenuConfig,
        visible: bool,
        event_proxy: EventLoopProxy<HostEvent>,
    }

    impl Tray for LinuxTray {
        fn id(&self) -> String {
            self.identifier.clone()
        }

        fn title(&self) -> String {
            self.name.clone()
        }

        fn icon_name(&self) -> String {
            self.identifier.clone()
        }

        fn status(&self) -> Status {
            if self.visible {
                Status::Active
            } else {
                Status::Passive
            }
        }

        fn tool_tip(&self) -> ToolTip {
            ToolTip {
                icon_name: self.identifier.clone(),
                title: self.name.clone(),
                description: self.tooltip.clone(),
                ..ToolTip::default()
            }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self
                .event_proxy
                .send_event(HostEvent::Shell(ShellEvent::TrayActivated { button: 1 }));
        }

        fn secondary_activate(&mut self, _x: i32, _y: i32) {
            let _ = self
                .event_proxy
                .send_event(HostEvent::Shell(ShellEvent::TrayActivated { button: 2 }));
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            self.menu.items.iter().map(linux_menu_item).collect()
        }
    }

    fn linux_menu_item(item: &MenuItemConfig) -> MenuItem<LinuxTray> {
        match item.kind {
            MenuItemKind::Command => {
                let id = item.id.clone();
                StandardItem {
                    label: item.label.clone(),
                    enabled: item.enabled,
                    shortcut: shortcut(item.accelerator.as_deref()),
                    activate: Box::new(move |tray: &mut LinuxTray| {
                        let _ = tray
                            .event_proxy
                            .send_event(HostEvent::Shell(ShellEvent::MenuSelected(id.clone())));
                    }),
                    ..StandardItem::default()
                }
                .into()
            }
            MenuItemKind::Checkbox => {
                let id = item.id.clone();
                CheckmarkItem {
                    label: item.label.clone(),
                    enabled: item.enabled,
                    checked: item.checked,
                    shortcut: shortcut(item.accelerator.as_deref()),
                    activate: Box::new(move |tray: &mut LinuxTray| {
                        let next = !find_checked(&tray.menu.items, &id).unwrap_or(false);
                        let _ = set_item_checked(&mut tray.menu.items, &id, next);
                        let _ = tray
                            .event_proxy
                            .send_event(HostEvent::Shell(ShellEvent::MenuSelected(id.clone())));
                    }),
                    ..CheckmarkItem::default()
                }
                .into()
            }
            MenuItemKind::Separator => MenuItem::Separator,
            MenuItemKind::Submenu => SubMenu {
                label: item.label.clone(),
                enabled: item.enabled,
                submenu: item.items.iter().map(linux_menu_item).collect(),
                ..SubMenu::default()
            }
            .into(),
        }
    }

    fn shortcut(accelerator: Option<&str>) -> Vec<Vec<String>> {
        accelerator.map_or_else(Vec::new, |accelerator| {
            vec![
                accelerator
                    .split('+')
                    .map(|token| match token.to_ascii_lowercase().as_str() {
                        "ctrl" | "control" | "cmdorctrl" => "Control".to_owned(),
                        "cmd" | "command" | "super" => "Super".to_owned(),
                        "alt" => "Alt".to_owned(),
                        "shift" => "Shift".to_owned(),
                        _ => token.trim_start_matches("Key").to_owned(),
                    })
                    .collect(),
            ]
        })
    }

    fn set_item_enabled(items: &mut [MenuItemConfig], id: &str, enabled: bool) -> Option<()> {
        for item in items {
            if item.id == id {
                item.enabled = enabled;
                return Some(());
            }
            if set_item_enabled(&mut item.items, id, enabled).is_some() {
                return Some(());
            }
        }
        None
    }

    fn set_item_checked(items: &mut [MenuItemConfig], id: &str, checked: bool) -> Option<()> {
        for item in items {
            if item.id == id && item.kind == MenuItemKind::Checkbox {
                item.checked = checked;
                return Some(());
            }
            if set_item_checked(&mut item.items, id, checked).is_some() {
                return Some(());
            }
        }
        None
    }

    fn find_checked(items: &[MenuItemConfig], id: &str) -> Option<bool> {
        for item in items {
            if item.id == id && item.kind == MenuItemKind::Checkbox {
                return Some(item.checked);
            }
            if let Some(checked) = find_checked(&item.items, id) {
                return Some(checked);
            }
        }
        None
    }

    pub fn install_platform_event_handlers(_proxy: &EventLoopProxy<HostEvent>) {}
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform {
    use tray_icon::menu::accelerator::Accelerator;
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    use super::{
        EventLoopProxy, HashMap, HostEvent, MenuConfig, MenuItemConfig, MenuItemKind, ShellEvent,
    };

    enum ItemHandle {
        Command(MenuItem),
        Checkbox(CheckMenuItem),
        Submenu(Submenu),
    }

    impl ItemHandle {
        fn set_enabled(&self, enabled: bool) {
            match self {
                Self::Command(item) => item.set_enabled(enabled),
                Self::Checkbox(item) => item.set_enabled(enabled),
                Self::Submenu(item) => item.set_enabled(enabled),
            }
        }

        fn set_checked(&self, checked: bool) -> Result<(), String> {
            match self {
                Self::Checkbox(item) => {
                    item.set_checked(checked);
                    Ok(())
                }
                Self::Command(_) | Self::Submenu(_) => {
                    Err("only checkbox menu items accept checked state".to_owned())
                }
            }
        }
    }

    pub struct PlatformTray {
        tray: TrayIcon,
        _menu: Menu,
        handles: HashMap<String, ItemHandle>,
        #[cfg(target_os = "macos")]
        _application_menu: Menu,
    }

    impl PlatformTray {
        pub fn prepare(
            identifier: &str,
            _name: &str,
            tooltip: String,
            config: &MenuConfig,
            _event_proxy: EventLoopProxy<HostEvent>,
        ) -> Result<Self, String> {
            let (menu, handles) = build_menu(config)?;
            let tray = TrayIconBuilder::new()
                .with_id(identifier)
                .with_tooltip(tooltip)
                .with_icon(default_icon()?)
                .with_menu(Box::new(menu.clone()))
                .with_menu_on_left_click(false)
                .build()
                .map_err(|error| format!("cannot create native tray icon: {error}"))?;
            #[cfg(target_os = "macos")]
            let application_menu = {
                let (application_menu, _) = build_menu(config)?;
                application_menu.init_for_nsapp();
                application_menu
            };
            Ok(Self {
                tray,
                _menu: menu,
                handles,
                #[cfg(target_os = "macos")]
                _application_menu: application_menu,
            })
        }

        pub fn set_menu_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
            self.handles
                .get(id)
                .ok_or_else(|| format!("menu item {id:?} was not found"))?
                .set_enabled(enabled);
            Ok(())
        }

        pub fn set_menu_checked(&self, id: &str, checked: bool) -> Result<(), String> {
            self.handles
                .get(id)
                .ok_or_else(|| format!("menu item {id:?} was not found"))?
                .set_checked(checked)
        }

        pub fn set_visible(&self, visible: bool) -> Result<(), String> {
            self.tray
                .set_visible(visible)
                .map_err(|error| format!("cannot change tray visibility: {error}"))
        }
    }

    fn build_menu(config: &MenuConfig) -> Result<(Menu, HashMap<String, ItemHandle>), String> {
        let menu = Menu::with_id(&config.id);
        let mut handles = HashMap::new();
        for item in &config.items {
            append_to_menu(&menu, item, &mut handles)?;
        }
        Ok((menu, handles))
    }

    fn append_to_menu(
        menu: &Menu,
        config: &MenuItemConfig,
        handles: &mut HashMap<String, ItemHandle>,
    ) -> Result<(), String> {
        let item = build_item(config, handles)?;
        match &item {
            BuiltItem::Command(value) => menu.append(value),
            BuiltItem::Checkbox(value) => menu.append(value),
            BuiltItem::Separator(value) => menu.append(value),
            BuiltItem::Submenu(value) => menu.append(value),
        }
        .map_err(|error| format!("cannot append native menu item: {error}"))?;
        Ok(())
    }

    fn append_to_submenu(
        menu: &Submenu,
        config: &MenuItemConfig,
        handles: &mut HashMap<String, ItemHandle>,
    ) -> Result<(), String> {
        let item = build_item(config, handles)?;
        match &item {
            BuiltItem::Command(value) => menu.append(value),
            BuiltItem::Checkbox(value) => menu.append(value),
            BuiltItem::Separator(value) => menu.append(value),
            BuiltItem::Submenu(value) => menu.append(value),
        }
        .map_err(|error| format!("cannot append native submenu item: {error}"))?;
        Ok(())
    }

    enum BuiltItem {
        Command(MenuItem),
        Checkbox(CheckMenuItem),
        Separator(PredefinedMenuItem),
        Submenu(Submenu),
    }

    fn build_item(
        config: &MenuItemConfig,
        handles: &mut HashMap<String, ItemHandle>,
    ) -> Result<BuiltItem, String> {
        let accelerator = config
            .accelerator
            .as_deref()
            .map(str::parse::<Accelerator>)
            .transpose()
            .map_err(|error| format!("cannot parse menu accelerator: {error}"))?;
        match config.kind {
            MenuItemKind::Command => {
                let item =
                    MenuItem::with_id(&config.id, &config.label, config.enabled, accelerator);
                handles.insert(config.id.clone(), ItemHandle::Command(item.clone()));
                Ok(BuiltItem::Command(item))
            }
            MenuItemKind::Checkbox => {
                let item = CheckMenuItem::with_id(
                    &config.id,
                    &config.label,
                    config.enabled,
                    config.checked,
                    accelerator,
                );
                handles.insert(config.id.clone(), ItemHandle::Checkbox(item.clone()));
                Ok(BuiltItem::Checkbox(item))
            }
            MenuItemKind::Separator => Ok(BuiltItem::Separator(PredefinedMenuItem::separator())),
            MenuItemKind::Submenu => {
                let submenu = Submenu::with_id(&config.id, &config.label, config.enabled);
                for child in &config.items {
                    append_to_submenu(&submenu, child, handles)?;
                }
                handles.insert(config.id.clone(), ItemHandle::Submenu(submenu.clone()));
                Ok(BuiltItem::Submenu(submenu))
            }
        }
    }

    fn default_icon() -> Result<Icon, String> {
        const SIZE: u32 = 32;
        let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let dx = i64::from(x) - 16;
                let dy = i64::from(y) - 16;
                let inside = dx * dx + dy * dy <= 14 * 14;
                rgba.extend_from_slice(if inside {
                    &[112, 76, 255, 255]
                } else {
                    &[0, 0, 0, 0]
                });
            }
        }
        Icon::from_rgba(rgba, SIZE, SIZE)
            .map_err(|error| format!("cannot create tray icon pixels: {error}"))
    }

    pub fn install_platform_event_handlers(proxy: &EventLoopProxy<HostEvent>) {
        let menu_proxy = proxy.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = menu_proxy.send_event(HostEvent::Shell(ShellEvent::MenuSelected(
                event.id.as_ref().to_owned(),
            )));
        }));
        let tray_proxy = proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            let button = match event {
                TrayIconEvent::Click {
                    button,
                    button_state: MouseButtonState::Up,
                    ..
                }
                | TrayIconEvent::DoubleClick { button, .. } => match button {
                    MouseButton::Left => 1,
                    MouseButton::Right => 2,
                    MouseButton::Middle => 3,
                },
                _ => return,
            };
            let _ = tray_proxy.send_event(HostEvent::Shell(ShellEvent::TrayActivated { button }));
        }));
    }
}

use platform::{PlatformTray, install_platform_event_handlers};

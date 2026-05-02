// prism-runtime/src/tray/mod.rs
//
// System tray integration for OpenRender Runtime.
// Provides a tray icon with double-click to show/hide window and a
// configurable right-click menu with built-in Exit and Reload options.

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Public types for tray configuration
// ---------------------------------------------------------------------------

/// Configuration for the system tray.
#[derive(Debug, Clone)]
pub struct TrayConfig {
    /// Whether the system tray is enabled.
    pub enabled: bool,
    /// Optional path to a custom tray icon (.png, 32-bit RGBA).
    /// If `None`, uses a built-in OpenRender icon.
    pub icon_path: Option<PathBuf>,
    /// Optional inline RGBA icon data (bytes, width, height).
    /// Takes priority over `icon_path` when set.
    pub icon_rgba: Option<(Vec<u8>, u32, u32)>,
    /// Tooltip text shown on hover.
    pub tooltip: String,
    /// Menu entries shown on right-click.
    pub menu: TrayMenu,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            icon_path: None,
            icon_rgba: None,
            tooltip: "OpenRender".to_string(),
            menu: TrayMenu::default(),
        }
    }
}

/// The tray menu definition.
#[derive(Debug, Clone)]
pub struct TrayMenu {
    /// User-defined menu entries (displayed before the built-in entries).
    pub items: Vec<TrayMenuEntry>,
    /// Optional CSS class for the menu container (for future CSS-rendered menus).
    pub class: Option<String>,
}

impl Default for TrayMenu {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            class: None,
        }
    }
}

/// A single entry in the tray menu.
#[derive(Debug, Clone)]
pub enum TrayMenuEntry {
    /// A single menu item.
    Item(TrayMenuItem),
    /// A group of items displayed together (e.g., side-by-side in CSS-rendered menus).
    ItemStack(TrayItemStack),
    /// A submenu containing nested entries.
    Submenu(TraySubmenu),
    /// A visual separator line.
    Separator,
}

/// A submenu with a label and nested menu entries.
#[derive(Debug, Clone)]
pub struct TraySubmenu {
    /// Display label for the submenu.
    pub label: String,
    /// Whether this submenu is enabled.
    pub enabled: bool,
    /// Nested menu entries.
    pub items: Vec<TrayMenuEntry>,
}

impl TraySubmenu {
    /// Create a new submenu with the given label and items.
    pub fn new(label: impl Into<String>, items: Vec<TrayMenuEntry>) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            items,
        }
    }
}

/// A single tray menu item.
#[derive(Debug, Clone)]
pub struct TrayMenuItem {
    /// Unique identifier for this item.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Optional icon name or path.
    pub icon: Option<String>,
    /// CSS class(es) for styling (for future CSS-rendered menus).
    pub class: Option<String>,
    /// What happens when this item is clicked.
    pub action: TrayMenuAction,
    /// Whether this item is enabled (clickable).
    pub enabled: bool,
}

impl TrayMenuItem {
    /// Create a new menu item with a custom action.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        let id_str = id.into();
        Self {
            id: id_str.clone(),
            label: label.into(),
            icon: None,
            class: None,
            action: TrayMenuAction::Custom(id_str),
            enabled: true,
        }
    }

    /// Set the CSS class for styling.
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = Some(class.into());
        self
    }

    /// Set an icon for this item.
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the action for this item.
    pub fn with_action(mut self, action: TrayMenuAction) -> Self {
        self.action = action;
        self
    }

    /// Mark this item as disabled.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// A group of menu items displayed together (side-by-side or stacked).
///
/// In native menus, items are flattened sequentially.
/// In CSS-rendered menus, the `direction` determines layout (e.g., the
/// built-in Reload+Exit pair renders side-by-side as a horizontal stack).
#[derive(Debug, Clone)]
pub struct TrayItemStack {
    /// The items in this group.
    pub items: Vec<TrayMenuItem>,
    /// CSS class(es) for the stack container.
    pub class: Option<String>,
    /// Layout direction.
    pub direction: StackDirection,
}

impl TrayItemStack {
    /// Create a horizontal item stack.
    pub fn horizontal(items: Vec<TrayMenuItem>) -> Self {
        Self {
            items,
            class: None,
            direction: StackDirection::Horizontal,
        }
    }

    /// Create a vertical item stack.
    pub fn vertical(items: Vec<TrayMenuItem>) -> Self {
        Self {
            items,
            class: None,
            direction: StackDirection::Vertical,
        }
    }

    /// Set the CSS class for styling.
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = Some(class.into());
        self
    }
}

/// Layout direction for item stacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackDirection {
    Horizontal,
    Vertical,
}

/// The action triggered when a tray menu item is clicked.
#[derive(Debug, Clone)]
pub enum TrayMenuAction {
    /// Exit the application.
    Exit,
    /// Reload the current scene.
    Reload,
    /// Show/hide the main window.
    ToggleWindow,
    /// Custom action identified by a string ID (fires a JS event).
    Custom(String),
}

// ---------------------------------------------------------------------------
// Events emitted by the tray
// ---------------------------------------------------------------------------

/// Events emitted by the system tray.
#[derive(Debug, Clone)]
pub enum TrayEvent {
    /// User double-clicked the tray icon (show window).
    ShowWindow,
    /// User selected Exit from the menu.
    Exit,
    /// User selected Reload from the menu.
    Reload,
    /// User selected Show/Hide from the menu.
    ToggleWindow,
    /// User selected a custom menu item.
    CustomAction(String),
}

// ---------------------------------------------------------------------------
// System tray management
// ---------------------------------------------------------------------------

/// Manages the system tray icon and menu.
pub struct SystemTray {
    /// The tray_icon handle — kept alive so the tray icon stays visible.
    tray_icon: Option<tray_icon::TrayIcon>,
    /// Maps MenuId → TrayMenuAction for lookup on menu click.
    menu_actions: Vec<(tray_icon::menu::MenuId, TrayMenuAction)>,
}

impl SystemTray {
    /// Create a new system tray from the given configuration.
    pub fn new(config: &TrayConfig) -> Self {
        if !config.enabled {
            return Self {
                tray_icon: None,
                menu_actions: Vec::new(),
            };
        }

        match Self::build(config) {
            Ok(tray) => tray,
            Err(e) => {
                log::error!("Failed to create system tray: {}", e);
                Self {
                    tray_icon: None,
                    menu_actions: Vec::new(),
                }
            }
        }
    }

    fn build(config: &TrayConfig) -> Result<Self, Box<dyn std::error::Error>> {
        use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
        use tray_icon::TrayIconBuilder;

        let menu = Menu::new();
        let mut menu_actions: Vec<(tray_icon::menu::MenuId, TrayMenuAction)> = Vec::new();

        // Add user-defined items first.
        Self::append_entries(&menu, &config.menu.items, &mut menu_actions)?;

        // Separator before built-in items (if user items exist).
        if !config.menu.items.is_empty() {
            menu.append(&PredefinedMenuItem::separator())?;
        }

        // Built-in: Reload and Exit.
        let reload_item = MenuItem::new("Reload", true, None);
        menu_actions.push((reload_item.id().clone(), TrayMenuAction::Reload));
        menu.append(&reload_item)?;

        let exit_item = MenuItem::new("Exit", true, None);
        menu_actions.push((exit_item.id().clone(), TrayMenuAction::Exit));
        menu.append(&exit_item)?;

        // Load icon — try custom path first, then fallback to a generated icon.
        let icon = if let Some((rgba, w, h)) = config.icon_rgba.clone() {
            tray_icon::Icon::from_rgba(rgba, w, h)?
        } else if let Some(ref icon_path) = config.icon_path {
            load_icon_from_file(icon_path)?
        } else {
            create_default_icon()
        };

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(&config.tooltip)
            .with_icon(icon)
            .build()?;

        Ok(Self {
            tray_icon: Some(tray_icon),
            menu_actions,
        })
    }

    /// Recursively append menu entries (including submenus) to a menu-like container.
    fn append_entries(
        menu: &tray_icon::menu::Menu,
        entries: &[TrayMenuEntry],
        actions: &mut Vec<(tray_icon::menu::MenuId, TrayMenuAction)>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use tray_icon::menu::{MenuItem, PredefinedMenuItem, Submenu};

        for entry in entries {
            match entry {
                TrayMenuEntry::Item(item) => {
                    let mi = MenuItem::new(&item.label, item.enabled, None);
                    actions.push((mi.id().clone(), item.action.clone()));
                    menu.append(&mi)?;
                }
                TrayMenuEntry::ItemStack(stack) => {
                    for item in &stack.items {
                        let mi = MenuItem::new(&item.label, item.enabled, None);
                        actions.push((mi.id().clone(), item.action.clone()));
                        menu.append(&mi)?;
                    }
                }
                TrayMenuEntry::Submenu(sub) => {
                    let submenu = Submenu::new(&sub.label, sub.enabled);
                    Self::append_entries_to_submenu(&submenu, &sub.items, actions)?;
                    menu.append(&submenu)?;
                }
                TrayMenuEntry::Separator => {
                    menu.append(&PredefinedMenuItem::separator())?;
                }
            }
        }
        Ok(())
    }

    /// Recursively append menu entries to a Submenu.
    fn append_entries_to_submenu(
        submenu: &tray_icon::menu::Submenu,
        entries: &[TrayMenuEntry],
        actions: &mut Vec<(tray_icon::menu::MenuId, TrayMenuAction)>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use tray_icon::menu::{MenuItem, PredefinedMenuItem, Submenu};

        for entry in entries {
            match entry {
                TrayMenuEntry::Item(item) => {
                    let mi = MenuItem::new(&item.label, item.enabled, None);
                    actions.push((mi.id().clone(), item.action.clone()));
                    submenu.append(&mi)?;
                }
                TrayMenuEntry::ItemStack(stack) => {
                    for item in &stack.items {
                        let mi = MenuItem::new(&item.label, item.enabled, None);
                        actions.push((mi.id().clone(), item.action.clone()));
                        submenu.append(&mi)?;
                    }
                }
                TrayMenuEntry::Submenu(sub) => {
                    let nested = Submenu::new(&sub.label, sub.enabled);
                    Self::append_entries_to_submenu(&nested, &sub.items, actions)?;
                    submenu.append(&nested)?;
                }
                TrayMenuEntry::Separator => {
                    submenu.append(&PredefinedMenuItem::separator())?;
                }
            }
        }
        Ok(())
    }

    /// Update the tray menu with new entries. Preserves built-in Reload/Exit items.
    pub fn update_menu(&mut self, items: &[TrayMenuEntry]) {
        use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};

        let Some(ref tray) = self.tray_icon else { return };

        let menu = Menu::new();
        let mut new_actions: Vec<(tray_icon::menu::MenuId, TrayMenuAction)> = Vec::new();

        if let Err(e) = Self::append_entries(&menu, items, &mut new_actions) {
            log::error!("Failed to build updated tray menu: {}", e);
            return;
        }

        if !items.is_empty() {
            let _ = menu.append(&PredefinedMenuItem::separator());
        }

        let reload_item = MenuItem::new("Reload", true, None);
        new_actions.push((reload_item.id().clone(), TrayMenuAction::Reload));
        let _ = menu.append(&reload_item);

        let exit_item = MenuItem::new("Exit", true, None);
        new_actions.push((exit_item.id().clone(), TrayMenuAction::Exit));
        let _ = menu.append(&exit_item);

        tray.set_menu(Some(Box::new(menu)));
        self.menu_actions = new_actions;
        log::info!("Tray menu updated");
    }

    /// Poll for tray events. Should be called once per frame.
    pub fn poll_events(&self) -> Vec<TrayEvent> {
        let mut events = Vec::new();

        if self.tray_icon.is_none() {
            return events;
        }

        // Check for tray icon events (double-click, etc.).
        while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
            if matches!(event, tray_icon::TrayIconEvent::DoubleClick { .. }) {
                events.push(TrayEvent::ShowWindow);
            }
        }

        // Check for menu events.
        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            for (id, action) in &self.menu_actions {
                if *id == event.id {
                    match action {
                        TrayMenuAction::Exit => events.push(TrayEvent::Exit),
                        TrayMenuAction::Reload => events.push(TrayEvent::Reload),
                        TrayMenuAction::ToggleWindow => events.push(TrayEvent::ToggleWindow),
                        TrayMenuAction::Custom(s) => events.push(TrayEvent::CustomAction(s.clone())),
                    }
                    break;
                }
            }
        }

        events
    }

    /// Check if the tray is active.
    pub fn is_active(&self) -> bool {
        self.tray_icon.is_some()
    }
}

/// Load an icon from a PNG/image file and convert to RGBA for tray-icon.
fn load_icon_from_file(path: &std::path::Path) -> Result<tray_icon::Icon, Box<dyn std::error::Error>> {
    let img = image::open(path)?.into_rgba8();
    let (w, h) = img.dimensions();
    let rgba = img.into_raw();
    Ok(tray_icon::Icon::from_rgba(rgba, w, h)?)
}

/// Create a simple 32×32 OpenRender-branded icon (indigo gradient square).
fn create_default_icon() -> tray_icon::Icon {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let fx = x as f32 / SIZE as f32;
            let fy = y as f32 / SIZE as f32;
            // Indigo gradient: #4f46e5 → #6366f1
            let r = (79.0 + (99.0 - 79.0) * fx) as u8;
            let g = (70.0 + (102.0 - 70.0) * fy) as u8;
            let b = (229.0 + (241.0 - 229.0) * (fx + fy) / 2.0) as u8;
            // Rounded corners: fade alpha near corners.
            let dx = (fx - 0.5).abs() * 2.0;
            let dy = (fy - 0.5).abs() * 2.0;
            let dist = (dx * dx + dy * dy).sqrt();
            let a = if dist > 0.9 { ((1.0 - (dist - 0.9) / 0.1).max(0.0) * 255.0) as u8 } else { 255 };
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = a;
        }
    }
    tray_icon::Icon::from_rgba(rgba, SIZE, SIZE).expect("Failed to create default tray icon")
}


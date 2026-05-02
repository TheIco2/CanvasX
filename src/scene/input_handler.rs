// prism-runtime/src/scene/input_handler.rs
//
// Input event processing — mouse hit-testing, keyboard input routing,
// focus management, and event dispatch. This makes OpenRender documents
// actually interactive and usable as app windows.

use std::collections::HashMap;
use crate::prd::document::PrdDocument;
use crate::prd::node::{NodeId, NodeKind, EventAction};
use crate::prd::input::{InputKind, InteractionState, FocusState};
use crate::prd::style::{CursorStyle, Overflow};
use crate::prd::value::Rect;

/// Mouse button identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// A raw input event from the platform layer.
#[derive(Debug, Clone)]
pub enum RawInputEvent {
    /// Mouse moved to a position in viewport coordinates.
    MouseMove { x: f32, y: f32 },
    /// Mouse button pressed.
    MouseDown { x: f32, y: f32, button: MouseButton },
    /// Mouse button released.
    MouseUp { x: f32, y: f32, button: MouseButton },
    /// Mouse wheel scrolled.
    MouseWheel { x: f32, y: f32, delta_x: f32, delta_y: f32 },
    /// Keyboard key pressed.
    KeyDown { key: KeyCode, modifiers: Modifiers },
    /// Keyboard key released.
    KeyUp { key: KeyCode, modifiers: Modifiers },
    /// Text input (from keyboard composition).
    TextInput { text: String },
}

/// Simplified key codes for interactive elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
    A, C, V, X, Z, // For clipboard shortcuts
    Other(u32),
}

/// Modifier key state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Events emitted by the input handler after processing raw input.
/// These are the high-level events that the host application cares about.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// A button or clickable element was clicked.
    Click { node_id: NodeId },
    /// A text input or textarea value changed.
    ValueChanged { node_id: NodeId, value: String },
    /// A checkbox was toggled.
    CheckChanged { node_id: NodeId, checked: bool },
    /// A slider value changed.
    SliderChanged { node_id: NodeId, value: f64 },
    /// A dropdown selection changed.
    SelectionChanged { node_id: NodeId, value: String },
    /// A color picker value changed.
    ColorChanged { node_id: NodeId, value: crate::prd::value::Color },
    /// A tab was selected.
    TabSelected { node_id: NodeId, tab_id: String },
    /// A link was activated.
    LinkActivated { node_id: NodeId, target: crate::prd::input::LinkTarget },
    /// A compiled event action should be dispatched.
    Action(EventAction),
    /// Navigation request (from Navigate events or links).
    NavigateRequest { scene_id: String },
    /// Open an external URL.
    OpenExternal { url: String },
    /// An IPC command should be sent.
    IpcCommand { ns: String, cmd: String, args: Option<serde_json::Value> },
}

/// The input handler manages focus, hover, and dispatches events.
pub struct InputHandler {
    /// Per-node interaction state.
    pub states: HashMap<NodeId, InteractionState>,
    /// Currently focused node (receives keyboard input).
    pub focused: Option<NodeId>,
    /// Currently hovered node.
    pub hovered: Option<NodeId>,
    /// Last known mouse position.
    pub mouse_pos: (f32, f32),
    /// Pending events from the current frame.
    pending_events: Vec<UiEvent>,
    /// The current cursor style hint.
    pub cursor: CursorIcon,
    /// Set when a scroll event changed a node's scroll_y (caller should invalidate layout).
    pub scroll_dirty: bool,
    /// Set when a ToggleClass action modified a node's class list (caller should re-apply CSS).
    pub class_dirty: bool,
    /// Set when hover state changed (caller should invalidate paint but not layout).
    pub hover_dirty: bool,
}

/// Cursor icon hints for the platform layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorIcon {
    Default,
    Pointer,
    Text,
    Move,
    NotAllowed,
    ResizeNS,
    ResizeEW,
}

impl Default for CursorIcon {
    fn default() -> Self {
        CursorIcon::Default
    }
}

impl InputHandler {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            focused: None,
            hovered: None,
            mouse_pos: (0.0, 0.0),
            pending_events: Vec::new(),
            cursor: CursorIcon::Default,
            scroll_dirty: false,
            class_dirty: false,
            hover_dirty: false,
        }
    }

    /// Process a raw input event against the document. Returns high-level UI events.
    pub fn process_event(
        &mut self,
        doc: &mut PrdDocument,
        event: RawInputEvent,
    ) -> Vec<UiEvent> {
        self.pending_events.clear();

        match event {
            RawInputEvent::MouseMove { x, y } => {
                self.mouse_pos = (x, y);
                self.handle_mouse_move(doc, x, y);
            }
            RawInputEvent::MouseDown { x, y, button } => {
                // Use tracked position when coordinates are zero (common for
                // AppHost consumers that don't have direct InputHandler access).
                let (mx, my) = if x == 0.0 && y == 0.0 { self.mouse_pos } else { (x, y) };
                self.mouse_pos = (mx, my);
                if button == MouseButton::Left {
                    self.handle_mouse_down(doc, mx, my);
                }
            }
            RawInputEvent::MouseUp { x, y, button } => {
                let (mx, my) = if x == 0.0 && y == 0.0 { self.mouse_pos } else { (x, y) };
                self.mouse_pos = (mx, my);
                if button == MouseButton::Left {
                    self.handle_mouse_up(doc, mx, my);
                }
            }
            RawInputEvent::MouseWheel { x, y, delta_x, delta_y } => {
                self.handle_scroll(doc, x, y, delta_x, delta_y);
            }
            RawInputEvent::KeyDown { key, modifiers } => {
                self.handle_key_down(doc, key, modifiers);
            }
            RawInputEvent::TextInput { text } => {
                self.handle_text_input(doc, &text);
            }
            _ => {}
        }

        std::mem::take(&mut self.pending_events)
    }

    /// Drain all pending events.
    pub fn take_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.pending_events)
    }

    // --- Hit testing ---

    /// Find the deepest node at (x, y) that is interactive.
    fn hit_test(&self, doc: &PrdDocument, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_node(doc, doc.root, x, y)
    }

    fn hit_test_node(&self, doc: &PrdDocument, node_id: NodeId, x: f32, y: f32) -> Option<NodeId> {
        let node = doc.get_node(node_id)?;

        // Skip nodes that are invisible or have pointer-events disabled.
        if matches!(node.style.display, crate::prd::style::Display::None) {
            return None;
        }
        if matches!(node.style.visibility, crate::prd::style::Visibility::Hidden
                                          | crate::prd::style::Visibility::Collapse) {
            return None;
        }
        if matches!(node.style.pointer_events, crate::prd::style::PointerEvents::None) {
            return None;
        }

        // Check if point is in this node's rect.
        let rect = &node.layout.rect;
        if !point_in_rect(x, y, rect) {
            let tag = node.tag.as_deref().unwrap_or("-");
            let cls = node.classes.first().map(|s| s.as_str()).unwrap_or("");
            log::trace!(
                "[hit] MISS node {node_id} <{tag}> .{cls}  rect=({:.1},{:.1} {:.1}×{:.1})  mouse=({x:.1},{y:.1})",
                rect.x, rect.y, rect.width, rect.height,
            );
            return None;
        }

        // Check children in reverse order (front to back).
        for &child_id in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_node(doc, child_id, x, y) {
                return Some(hit);
            }
        }

        // If this node is interactive or has event bindings, return it.
        if is_interactive(&node.kind) || !node.events.is_empty() {
            let tag = node.tag.as_deref().unwrap_or("-");
            let cls = node.classes.first().map(|s| s.as_str()).unwrap_or("");
            log::trace!(
                "[hit] HIT  node {node_id} <{tag}> .{cls}  rect=({:.1},{:.1} {:.1}×{:.1})",
                rect.x, rect.y, rect.width, rect.height,
            );
            return Some(node_id);
        }

        // For non-interactive leaf/container nodes, still return them so
        // click events can bubble up to a parent with event handlers.
        // Only skip truly transparent nodes (root container with no content).
        if node_id != doc.root {
            return Some(node_id);
        }

        None
    }

    // --- Mouse handlers ---

    fn handle_mouse_move(&mut self, doc: &mut PrdDocument, x: f32, y: f32) {
        let hit = self.hit_test(doc, x, y);

        // Update hover state.
        if hit != self.hovered {
            // Un-hover previous node and all its ancestors.
            if let Some(prev) = self.hovered {
                if let Some(state) = self.states.get_mut(&prev) {
                    state.hovered = false;
                    state.focus = if state.focus == FocusState::Hovered {
                        FocusState::None
                    } else {
                        state.focus
                    };
                }
                // Clear hovered flag on the node and all ancestors.
                let mut clear_id = Some(prev);
                while let Some(nid) = clear_id {
                    if let Some(node) = doc.get_node_mut(nid) {
                        node.hovered = false;
                    }
                    clear_id = doc.find_parent(nid);
                }
            }

            // Hover new node and all its ancestors.
            if let Some(node_id) = hit {
                let state = self.states.entry(node_id).or_default();
                state.hovered = true;
                if state.focus == FocusState::None {
                    state.focus = FocusState::Hovered;
                }
                // Set hovered flag on the node and all ancestors.
                let mut hover_id = Some(node_id);
                while let Some(nid) = hover_id {
                    if let Some(node) = doc.get_node_mut(nid) {
                        node.hovered = true;
                    }
                    hover_id = doc.find_parent(nid);
                }
            }

            self.hovered = hit;
            self.hover_dirty = true;
        }

        // Update cursor.
        // Check :hover style overrides first (e.g. `cursor: pointer` on :hover).
        // Walk up ancestors to find hover cursor since hit test returns the deepest node
        // but CSS cursor on :hover may be set on a parent.
        self.cursor = if let Some(node_id) = hit {
            resolve_hover_cursor(doc, node_id)
        } else {
            CursorIcon::Default
        };
    }

    fn handle_mouse_down(&mut self, doc: &mut PrdDocument, x: f32, y: f32) {
        let hit = self.hit_test(doc, x, y);

        // Update focus.
        if hit != self.focused {
            // Blur previous — clear focused flag on node and ancestors.
            if let Some(prev) = self.focused {
                if let Some(state) = self.states.get_mut(&prev) {
                    state.focus = FocusState::None;
                }
                let mut clear_id = Some(prev);
                while let Some(nid) = clear_id {
                    if let Some(node) = doc.get_node_mut(nid) {
                        node.focused = false;
                    }
                    clear_id = doc.find_parent(nid);
                }
            }
            self.focused = hit;
            // Set focused flag on new node and ancestors.
            if let Some(node_id) = hit {
                let mut focus_id = Some(node_id);
                while let Some(nid) = focus_id {
                    if let Some(node) = doc.get_node_mut(nid) {
                        node.focused = true;
                    }
                    focus_id = doc.find_parent(nid);
                }
            }
        }

        // Set pressed / :active state on node and ancestors.
        if let Some(node_id) = hit {
            let state = self.states.entry(node_id).or_default();
            state.pressed = true;
            state.focus = FocusState::Active;
            let mut active_id = Some(node_id);
            while let Some(nid) = active_id {
                if let Some(node) = doc.get_node_mut(nid) {
                    node.active = true;
                }
                active_id = doc.find_parent(nid);
            }
        }
    }

    fn handle_mouse_up(&mut self, doc: &mut PrdDocument, x: f32, y: f32) {
        let hit = self.hit_test(doc, x, y);

        // Find all pressed nodes and release them.
        let pressed_nodes: Vec<NodeId> = self.states.iter()
            .filter(|(_, s)| s.pressed)
            .map(|(&id, _)| id)
            .collect();

        for node_id in pressed_nodes {
            if let Some(state) = self.states.get_mut(&node_id) {
                state.pressed = false;
                state.focus = if Some(node_id) == self.focused {
                    FocusState::Focused
                } else if state.hovered {
                    FocusState::Hovered
                } else {
                    FocusState::None
                };
            }

            // Clear :active flag on the node and all ancestors.
            let mut clear_id = Some(node_id);
            while let Some(nid) = clear_id {
                if let Some(node) = doc.get_node_mut(nid) {
                    node.active = false;
                }
                clear_id = doc.find_parent(nid);
            }

            // If released on the same node it was pressed on, it's a click.
            if hit == Some(node_id) {
                self.dispatch_click(doc, node_id);
            }
        }
    }

    fn handle_scroll(&mut self, doc: &mut PrdDocument, _x: f32, _y: f32, _dx: f32, dy: f32) {
        let (mx, my) = self.mouse_pos;
        // Find the nearest scroll container ancestor at the cursor.
        if let Some(node_id) = self.find_scroll_container(doc, mx, my) {
            // Compute content height from children's layout rects.
            let content_height = if let Some(node) = doc.get_node(node_id) {
                let container_top = node.layout.content_rect.y;
                let container_height = node.layout.content_rect.height;
                let mut max_bottom: f32 = 0.0;
                for &child_id in &node.children {
                    if let Some(child) = doc.get_node(child_id) {
                        // Children's rect.y already includes the previous scroll offset,
                        // so add it back to get the unscrolled bottom.
                        let child_bottom = child.layout.rect.y + child.layout.rect.height
                            - container_top + node.layout.scroll_y;
                        max_bottom = max_bottom.max(child_bottom);
                    }
                }
                // Scrollable distance is content that exceeds the container.
                (max_bottom - container_height).max(0.0)
            } else {
                0.0
            };

            if let Some(node) = doc.nodes.get_mut(node_id as usize) {
                let old_scroll = node.layout.scroll_y;
                node.layout.scroll_y = (node.layout.scroll_y - dy).clamp(0.0, content_height);
                if (node.layout.scroll_y - old_scroll).abs() > 0.01 {
                    self.scroll_dirty = true;
                }
            }
        }
    }

    fn find_scroll_container(&self, doc: &PrdDocument, x: f32, y: f32) -> Option<NodeId> {
        self.find_scroll_container_node(doc, doc.root, x, y)
    }

    fn find_scroll_container_node(
        &self,
        doc: &PrdDocument,
        node_id: NodeId,
        x: f32,
        y: f32,
    ) -> Option<NodeId> {
        let node = doc.get_node(node_id)?;
        let rect = &node.layout.rect;
        if !point_in_rect(x, y, rect) {
            return None;
        }

        // Check children first (deepest match wins).
        for &child_id in node.children.iter().rev() {
            if let Some(found) = self.find_scroll_container_node(doc, child_id, x, y) {
                return Some(found);
            }
        }

        // A node is a scroll container if it has overflow: scroll or is a ScrollContainer kind.
        match &node.kind {
            NodeKind::ScrollContainer { .. } => Some(node_id),
            _ if matches!(node.style.overflow, Overflow::Scroll) => Some(node_id),
            _ => None,
        }
    }

    // --- Click dispatch ---

    fn dispatch_click(&mut self, doc: &mut PrdDocument, node_id: NodeId) {
        let node = match doc.get_node(node_id) {
            Some(n) => n.clone(),
            None => return,
        };

        // Track whether event bindings were found on the clicked node.
        let mut found_bindings = false;

        // Collect actions to execute (clone to avoid holding node borrow).
        let mut toggle_classes: Vec<(NodeId, String, String)> = Vec::new();

        // Dispatch compiled event bindings.
        for binding in &node.events {
            if binding.event == "click" {
                found_bindings = true;
                match &binding.action {
                    EventAction::ToggleClass { target, class, target_html_id } => {
                        toggle_classes.push((*target, class.clone(), target_html_id.clone()));
                    }
                    EventAction::IpcCommand { ns, cmd, args } => {
                        self.pending_events.push(UiEvent::IpcCommand {
                            ns: ns.clone(),
                            cmd: cmd.clone(),
                            args: args.clone(),
                        });
                    }
                    EventAction::Navigate { scene_id } => {
                        self.pending_events.push(UiEvent::NavigateRequest {
                            scene_id: scene_id.clone(),
                        });
                    }
                    other => {
                        self.pending_events.push(UiEvent::Action(other.clone()));
                    }
                }
            }
        }

        // Execute ToggleClass actions on the document.
        for (target, class, target_html_id) in toggle_classes {
            let resolved = if !target_html_id.is_empty() {
                Self::find_node_by_html_id(doc, &target_html_id).unwrap_or(node_id)
            } else if target == 0 {
                node_id
            } else {
                target
            };
            if let Some(target_node) = doc.nodes.get_mut(resolved as usize) {
                if let Some(pos) = target_node.classes.iter().position(|c| c == &class) {
                    target_node.classes.remove(pos);
                } else {
                    target_node.classes.push(class);
                }
                self.class_dirty = true;
            }
        }

        // Event bubbling: if the clicked node had no click bindings,
        // walk up the parent chain to find an ancestor with handlers.
        if !found_bindings {
            self.bubble_click_to_parent(doc, node_id);
        }

        // Handle built-in interactive node types.
        self.handle_interactive_click(doc, node_id);

        // Always emit a generic click event.
        self.pending_events.push(UiEvent::Click { node_id });
    }

    /// Walk up from `child_id` through parent nodes, dispatching click event
    /// bindings on the first ancestor that has them (event bubbling).
    fn bubble_click_to_parent(&mut self, doc: &mut PrdDocument, child_id: NodeId) {
        // Find parent that contains child_id.
        let mut current = child_id;
        while let Some(parent_id) = self.find_parent(doc, doc.root, current) {
            if let Some(parent) = doc.get_node(parent_id) {
                let mut found = false;
                let mut toggle_classes: Vec<(NodeId, String, String)> = Vec::new();
                for binding in &parent.events {
                    if binding.event == "click" {
                        found = true;
                        match &binding.action {
                            EventAction::ToggleClass { target, class, target_html_id } => {
                                toggle_classes.push((*target, class.clone(), target_html_id.clone()));
                            }
                            EventAction::IpcCommand { ns, cmd, args } => {
                                self.pending_events.push(UiEvent::IpcCommand {
                                    ns: ns.clone(),
                                    cmd: cmd.clone(),
                                    args: args.clone(),
                                });
                            }
                            EventAction::Navigate { scene_id } => {
                                self.pending_events.push(UiEvent::NavigateRequest {
                                    scene_id: scene_id.clone(),
                                });
                            }
                            other => {
                                self.pending_events.push(UiEvent::Action(other.clone()));
                            }
                        }
                    }
                }
                // Execute ToggleClass actions on the document.
                for (target, class, target_html_id) in toggle_classes {
                    let resolved = if !target_html_id.is_empty() {
                        Self::find_node_by_html_id(doc, &target_html_id).unwrap_or(parent_id)
                    } else if target == 0 {
                        parent_id
                    } else {
                        target
                    };
                    if let Some(target_node) = doc.nodes.get_mut(resolved as usize) {
                        if let Some(pos) = target_node.classes.iter().position(|c| c == &class) {
                            target_node.classes.remove(pos);
                        } else {
                            target_node.classes.push(class);
                        }
                        self.class_dirty = true;
                    }
                }
                if found {
                    return; // Stop bubbling once we find a handler.
                }
            }
            current = parent_id;
        }
    }

    /// Find the parent of `target` by searching from `node_id` downward.
    fn find_parent(&self, doc: &PrdDocument, node_id: NodeId, target: NodeId) -> Option<NodeId> {
        let node = doc.get_node(node_id)?;
        for &child_id in &node.children {
            if child_id == target {
                return Some(node_id);
            }
            if let Some(found) = self.find_parent(doc, child_id, target) {
                return Some(found);
            }
        }
        None
    }

    /// Find a node by its HTML `id` attribute.
    fn find_node_by_html_id(doc: &PrdDocument, html_id: &str) -> Option<NodeId> {
        for (i, node) in doc.nodes.iter().enumerate() {
            if node.html_id.as_deref() == Some(html_id) {
                return Some(i as NodeId);
            }
        }
        None
    }

    fn handle_interactive_click(&mut self, doc: &mut PrdDocument, node_id: NodeId) {
        // We need to work with a clone since we can't borrow mutably while reading.
        let node = match doc.get_node(node_id) {
            Some(n) => n.clone(),
            None => return,
        };

        match &node.kind {
            NodeKind::Input(InputKind::Checkbox { checked, label, disabled, style }) => {
                if !disabled {
                    let new_checked = !checked;
                    let label = label.clone();
                    let disabled = *disabled;
                    let style = *style;
                    if let Some(n) = doc.get_node_mut(node_id) {
                        n.kind = NodeKind::Input(InputKind::Checkbox {
                            checked: new_checked,
                            label,
                            disabled,
                            style,
                        });
                    }
                    self.pending_events.push(UiEvent::CheckChanged {
                        node_id,
                        checked: new_checked,
                    });
                }
            }
            NodeKind::Input(InputKind::Dropdown { open, options, selected, placeholder, disabled }) => {
                if !disabled {
                    let new_open = !open;
                    let options = options.clone();
                    let selected = selected.clone();
                    let placeholder = placeholder.clone();
                    let disabled = *disabled;
                    if let Some(n) = doc.get_node_mut(node_id) {
                        n.kind = NodeKind::Input(InputKind::Dropdown {
                            open: new_open,
                            options,
                            selected,
                            placeholder,
                            disabled,
                        });
                    }
                }
            }
            NodeKind::Input(InputKind::Link { href: target, .. }) => {
                match target {
                    crate::prd::input::LinkTarget::Scene(scene_id) => {
                        self.pending_events.push(UiEvent::NavigateRequest {
                            scene_id: scene_id.clone(),
                        });
                    }
                    crate::prd::input::LinkTarget::External(url) => {
                        self.pending_events.push(UiEvent::OpenExternal {
                            url: url.clone(),
                        });
                    }
                    crate::prd::input::LinkTarget::Ipc { ns, cmd, args } => {
                        self.pending_events.push(UiEvent::IpcCommand {
                            ns: ns.clone(),
                            cmd: cmd.clone(),
                            args: args.clone(),
                        });
                    }
                }
            }
            NodeKind::Input(InputKind::TabBar { tabs, active_tab }) => {
                // Tab clicks are handled by checking mouse position against tab rects.
                // For now, cycle to next tab on click.
                if let Some(idx) = tabs.iter().position(|t| t.id == *active_tab) {
                    let next = (idx + 1) % tabs.len();
                    let new_tab = tabs[next].id.clone();
                    let tabs = tabs.clone();
                    if let Some(n) = doc.get_node_mut(node_id) {
                        n.kind = NodeKind::Input(InputKind::TabBar {
                            tabs,
                            active_tab: new_tab.clone(),
                        });
                    }
                    self.pending_events.push(UiEvent::TabSelected {
                        node_id,
                        tab_id: new_tab,
                    });
                }
            }
            _ => {}
        }
    }

    // --- Keyboard handlers ---

    fn handle_key_down(&mut self, doc: &mut PrdDocument, key: KeyCode, modifiers: Modifiers) {
        let focused = match self.focused {
            Some(id) => id,
            None => {
                // Tab to focus first interactive element.
                if key == KeyCode::Tab {
                    self.focus_next(doc, true);
                }
                return;
            }
        };

        let node = match doc.get_node(focused) {
            Some(n) => n.clone(),
            None => return,
        };

        match &node.kind {
            NodeKind::Input(InputKind::TextInput { value, placeholder, max_length, read_only, input_type }) => {
                if *read_only {
                    return;
                }
                let state = self.states.entry(focused).or_default();
                let mut new_value = value.clone();
                let cursor = state.cursor_pos.min(new_value.len());

                match key {
                    KeyCode::Backspace => {
                        if cursor > 0 {
                            new_value.remove(cursor - 1);
                            state.cursor_pos = cursor - 1;
                        }
                    }
                    KeyCode::Delete => {
                        if cursor < new_value.len() {
                            new_value.remove(cursor);
                        }
                    }
                    KeyCode::Left => {
                        state.cursor_pos = cursor.saturating_sub(1);
                    }
                    KeyCode::Right => {
                        state.cursor_pos = (cursor + 1).min(new_value.len());
                    }
                    KeyCode::Home => {
                        state.cursor_pos = 0;
                    }
                    KeyCode::End => {
                        state.cursor_pos = new_value.len();
                    }
                    KeyCode::Tab => {
                        self.focus_next(doc, !modifiers.shift);
                        return;
                    }
                    KeyCode::Enter => {
                        // Submit / blur.
                        self.pending_events.push(UiEvent::ValueChanged {
                            node_id: focused,
                            value: new_value,
                        });
                        return;
                    }
                    _ => return,
                }

                // Update the node.
                let placeholder = placeholder.clone();
                let max_length = *max_length;
                let read_only = *read_only;
                let input_type = *input_type;
                if let Some(n) = doc.get_node_mut(focused) {
                    n.kind = NodeKind::Input(InputKind::TextInput {
                        value: new_value.clone(),
                        placeholder,
                        max_length,
                        read_only,
                        input_type,
                    });
                }
                self.pending_events.push(UiEvent::ValueChanged {
                    node_id: focused,
                    value: new_value,
                });
            }
            NodeKind::Input(InputKind::Slider { value, min, max, step, disabled, show_value }) => {
                if *disabled {
                    return;
                }
                let new_value = match key {
                    KeyCode::Left | KeyCode::Down => (value - step).max(*min),
                    KeyCode::Right | KeyCode::Up => (value + step).min(*max),
                    KeyCode::Home => *min,
                    KeyCode::End => *max,
                    KeyCode::Tab => {
                        self.focus_next(doc, !modifiers.shift);
                        return;
                    }
                    _ => return,
                };
                let min = *min;
                let max = *max;
                let step = *step;
                let disabled = *disabled;
                let show_value = *show_value;
                if let Some(n) = doc.get_node_mut(focused) {
                    n.kind = NodeKind::Input(InputKind::Slider {
                        value: new_value,
                        min, max, step, disabled, show_value,
                    });
                }
                self.pending_events.push(UiEvent::SliderChanged {
                    node_id: focused,
                    value: new_value,
                });
            }
            NodeKind::Input(InputKind::Checkbox { .. }) => {
                if key == KeyCode::Space || key == KeyCode::Enter {
                    self.handle_interactive_click(doc, focused);
                } else if key == KeyCode::Tab {
                    self.focus_next(doc, !modifiers.shift);
                }
            }
            _ => {
                if key == KeyCode::Tab {
                    self.focus_next(doc, !modifiers.shift);
                }
            }
        }
    }

    fn handle_text_input(&mut self, doc: &mut PrdDocument, text: &str) {
        let focused = match self.focused {
            Some(id) => id,
            None => return,
        };

        let node = match doc.get_node(focused) {
            Some(n) => n.clone(),
            None => return,
        };

        match &node.kind {
            NodeKind::Input(InputKind::TextInput { value, placeholder, max_length, read_only, input_type }) => {
                if *read_only {
                    return;
                }
                let state = self.states.entry(focused).or_default();
                let cursor = state.cursor_pos.min(value.len());
                let mut new_value = value.clone();

                for ch in text.chars() {
                    if *max_length > 0 && new_value.len() >= *max_length as usize {
                        break;
                    }
                    new_value.insert(cursor + new_value.len() - value.len(), ch);
                    state.cursor_pos += 1;
                }

                let placeholder = placeholder.clone();
                let max_length = *max_length;
                let read_only = *read_only;
                let input_type = *input_type;
                if let Some(n) = doc.get_node_mut(focused) {
                    n.kind = NodeKind::Input(InputKind::TextInput {
                        value: new_value.clone(),
                        placeholder,
                        max_length,
                        read_only,
                        input_type,
                    });
                }
                self.pending_events.push(UiEvent::ValueChanged {
                    node_id: focused,
                    value: new_value,
                });
            }
            _ => {}
        }
    }

    // --- Focus navigation ---

    fn focus_next(&mut self, doc: &PrdDocument, forward: bool) {
        let interactive = collect_interactive_nodes(doc);
        if interactive.is_empty() {
            return;
        }

        let current_idx = self.focused.and_then(|id| interactive.iter().position(|&nid| nid == id));

        let next_idx = match current_idx {
            Some(idx) => {
                if forward {
                    (idx + 1) % interactive.len()
                } else {
                    (idx + interactive.len() - 1) % interactive.len()
                }
            }
            None => {
                if forward { 0 } else { interactive.len() - 1 }
            }
        };

        // Blur current.
        if let Some(prev) = self.focused {
            if let Some(state) = self.states.get_mut(&prev) {
                state.focus = FocusState::None;
            }
        }

        // Focus new.
        let new_focus = interactive[next_idx];
        self.focused = Some(new_focus);
        let state = self.states.entry(new_focus).or_default();
        state.focus = FocusState::Focused;
    }
}

// --- Helpers ---

fn point_in_rect(x: f32, y: f32, rect: &Rect) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

fn is_interactive(kind: &NodeKind) -> bool {
    matches!(kind,
        NodeKind::Input(_)
    )
}

fn cursor_for_node(kind: &NodeKind) -> CursorIcon {
    match kind {
        NodeKind::Input(InputKind::TextInput { .. }) |
        NodeKind::Input(InputKind::TextArea { .. }) => CursorIcon::Text,
        NodeKind::Input(InputKind::Button { disabled, .. }) => {
            if *disabled { CursorIcon::NotAllowed } else { CursorIcon::Pointer }
        }
        NodeKind::Input(InputKind::Link { .. }) => CursorIcon::Pointer,
        NodeKind::Input(InputKind::Slider { disabled, .. }) => {
            if *disabled { CursorIcon::NotAllowed } else { CursorIcon::ResizeEW }
        }
        NodeKind::Input(InputKind::Checkbox { disabled, .. }) |
        NodeKind::Input(InputKind::Dropdown { disabled, .. }) => {
            if *disabled { CursorIcon::NotAllowed } else { CursorIcon::Pointer }
        }
        _ => CursorIcon::Default,
    }
}

/// Resolve cursor icon: use CSS `cursor` property if set, else fall back to node kind.
fn css_cursor_or_fallback(css: &CursorStyle, kind: &NodeKind) -> CursorIcon {
    match css {
        CursorStyle::Auto => cursor_for_node(kind),
        CursorStyle::Default => CursorIcon::Default,
        CursorStyle::Pointer => CursorIcon::Pointer,
        CursorStyle::Text => CursorIcon::Text,
        CursorStyle::Move => CursorIcon::Move,
        CursorStyle::NotAllowed => CursorIcon::NotAllowed,
        CursorStyle::NsResize => CursorIcon::ResizeNS,
        CursorStyle::EwResize => CursorIcon::ResizeEW,
        CursorStyle::Grab | CursorStyle::Grabbing | CursorStyle::CrossHair
        | CursorStyle::ColResize | CursorStyle::RowResize => cursor_for_node(kind),
    }
}

/// Resolve a raw CSS cursor value string (from hover_style) to a CursorIcon.
fn css_cursor_from_str(value: &str, kind: &NodeKind) -> CursorIcon {
    match value.trim() {
        "pointer" => CursorIcon::Pointer,
        "default" => CursorIcon::Default,
        "text" => CursorIcon::Text,
        "move" => CursorIcon::Move,
        "not-allowed" => CursorIcon::NotAllowed,
        "ns-resize" | "n-resize" | "s-resize" => CursorIcon::ResizeNS,
        "ew-resize" | "e-resize" | "w-resize" => CursorIcon::ResizeEW,
        "grab" | "grabbing" | "crosshair" | "col-resize" | "row-resize" => cursor_for_node(kind),
        "auto" => cursor_for_node(kind),
        _ => CursorIcon::Default,
    }
}

/// Resolve cursor for a hit node, walking up ancestors to find hover cursor.
///
/// CSS `cursor` is an inherited property and `:hover` rules may set it on a
/// parent while the hit test returns a deeper child. We walk up the ancestor
/// chain checking `hover_style` for a cursor override, then fall back to the
/// compiled `style.cursor` (which already has inheritance baked in).
fn resolve_hover_cursor(doc: &PrdDocument, start: NodeId) -> CursorIcon {
    let mut nid = Some(start);
    while let Some(id) = nid {
        if let Some(node) = doc.get_node(id) {
            if node.hovered {
                if let Some(cursor_val) = node.hover_style.iter()
                    .find(|(p, _)| p == "cursor")
                    .map(|(_, v)| v.as_str())
                {
                    return css_cursor_from_str(cursor_val, &node.kind);
                }
            }
            // Check compiled style for a non-Auto cursor.
            if !matches!(node.style.cursor, CursorStyle::Auto) {
                return css_cursor_or_fallback(&node.style.cursor, &node.kind);
            }
        }
        nid = doc.find_parent(id);
    }
    // Nothing found — use node-kind heuristic on the hit node.
    doc.get_node(start)
        .map(|n| cursor_for_node(&n.kind))
        .unwrap_or(CursorIcon::Default)
}

/// Collect all interactive node IDs in document order (depth-first).
fn collect_interactive_nodes(doc: &PrdDocument) -> Vec<NodeId> {
    let mut result = Vec::new();
    collect_interactive_recursive(doc, doc.root, &mut result);
    result
}

fn collect_interactive_recursive(doc: &PrdDocument, node_id: NodeId, out: &mut Vec<NodeId>) {
    let node = match doc.get_node(node_id) {
        Some(n) => n,
        None => return,
    };

    if is_interactive(&node.kind) {
        out.push(node_id);
    }

    for &child_id in &node.children {
        collect_interactive_recursive(doc, child_id, out);
    }
}


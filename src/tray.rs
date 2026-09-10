use std::sync::{Mutex, OnceLock};

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::Strings;

/// `MenuEvent::set_event_handler` runs its callback directly from `muda`'s
/// own message handling, on whatever thread that happens to be — not
/// necessarily GPUI's main thread, and GPUI's `AsyncApp` is `!Send` so it
/// can't be touched from here directly. Forwarding through this channel
/// lets a `cx.spawn` foreground task (`app.rs`) `.await` events and wake the
/// window from a context where `AsyncApp` is actually usable.
///
/// `set_event_handler` can only be set once per process (`muda` holds it in
/// a `OnceCell`; later calls are ignored), so the sending/receiving channel
/// is a `static` that outlives `TrayHandle` being rebuilt (e.g. on a
/// language switch in `apply_language`).
static FORWARDED_EVENTS: OnceLock<Mutex<Option<UnboundedReceiver<MenuEvent>>>> = OnceLock::new();

/// Fixed id for the tray menu's Quit item, so the event handler below can
/// recognize it without needing a `TrayHandle` (which doesn't exist yet when
/// `set_event_handler` is registered — see `ensure_event_forwarding`).
const QUIT_MENU_ID: &str = "issen-tray-quit";

/// Returns the receiver only the first time this is called (later calls,
/// e.g. from `apply_language` rebuilding the tray, see `None` — the
/// original receiver is still being awaited by the `cx.spawn` task started
/// at startup, so it must not be handed out twice).
fn ensure_event_forwarding() -> Option<UnboundedReceiver<MenuEvent>> {
    FORWARDED_EVENTS
        .get_or_init(|| {
            let (tx, rx) = unbounded::<MenuEvent>();
            // Quit can't tolerate ever being missed, so it's special-cased right
            // here in the handler — which demonstrably always runs, since `muda`
            // calls it directly from its own message handling — instead of going
            // through the `TrayAction` channel like every other tray action.
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if event.id == MenuId::new(QUIT_MENU_ID) {
                    std::process::exit(0);
                }
                let _ = tx.unbounded_send(event);
            }));
            Mutex::new(Some(rx))
        })
        .lock()
        .ok()
        .and_then(|mut rx| rx.take())
}

/// `TrayIconEvent::set_event_handler` has its own separate process-wide
/// `OnceCell` (a `tray-icon`-side mechanism distinct from `muda`'s), so it
/// needs a second channel following the same pattern as `FORWARDED_EVENTS`.
static FORWARDED_TRAY_ICON_EVENTS: OnceLock<Mutex<Option<UnboundedReceiver<TrayIconEvent>>>> =
    OnceLock::new();

fn ensure_tray_icon_event_forwarding() -> Option<UnboundedReceiver<TrayIconEvent>> {
    FORWARDED_TRAY_ICON_EVENTS
        .get_or_init(|| {
            let (tx, rx) = unbounded::<TrayIconEvent>();
            TrayIconEvent::set_event_handler(Some(move |event| {
                let _ = tx.unbounded_send(event);
            }));
            Mutex::new(Some(rx))
        })
        .lock()
        .ok()
        .and_then(|mut rx| rx.take())
}

pub enum TrayAction {
    Open,
    Settings,
    Reindex,
    About,
    // No `Quit` variant: it's handled directly inside the `MenuEvent` handler
    // (see `ensure_event_forwarding`'s doc comment) rather than through this
    // channel, since it can't tolerate the poll being starved while hidden.
}

pub struct TrayHandle {
    icon: TrayIcon,
    open_id: MenuId,
    settings_id: MenuId,
    reindex_id: MenuId,
    /// Kept alongside its id (not just the id) so `set_scanning` can swap
    /// the label and disable clicks while a scan is running (`MenuItem` is
    /// a shared handle to the native item, so this value stays live and
    /// mutable even after `menu.append` has taken it).
    reindex_item: MenuItem,
    about_id: MenuId,
}

/// Takes the `MenuEvent` receiver. Only returns `Some` the first time this
/// is called process-wide — call once at startup and hand the receiver to a
/// `cx.spawn` foreground task that awaits it for the app's whole lifetime
/// (see `app.rs`). A `TrayHandle` rebuild (e.g. `apply_language`) does not
/// call this again; matching a received event against the *current*
/// `TrayHandle`'s ids is done via `match_menu_action` instead, so the one
/// long-lived task keeps working across rebuilds.
pub fn take_menu_event_receiver() -> Option<UnboundedReceiver<MenuEvent>> {
    ensure_event_forwarding()
}

/// Same as `take_menu_event_receiver`, for tray icon clicks.
pub fn take_tray_icon_event_receiver() -> Option<UnboundedReceiver<TrayIconEvent>> {
    ensure_tray_icon_event_forwarding()
}

impl TrayHandle {
    /// Returns `None` if creating the tray icon fails, which can happen at
    /// times other than startup (e.g. `explorer.exe` restarting). Callers
    /// may panic on a startup failure, but a rebuild triggered by something
    /// like a language switch should fall back to keeping the existing
    /// tray icon instead.
    pub fn new(strings: &Strings) -> Option<Self> {
        let menu = Menu::new();
        let open_item = MenuItem::new(strings.tray_open, true, None);
        let settings_item = MenuItem::new(strings.tray_settings, true, None);
        let reindex_item = MenuItem::new(strings.tray_reindex, true, None);
        let about_item = MenuItem::new(strings.tray_about, true, None);
        let quit_item = MenuItem::with_id(QUIT_MENU_ID, strings.tray_quit, true, None);

        let open_id = open_item.id().clone();
        let settings_id = settings_item.id().clone();
        let reindex_id = reindex_item.id().clone();
        let about_id = about_item.id().clone();

        let _ = menu.append(&open_item);
        let _ = menu.append(&settings_item);
        let _ = menu.append(&reindex_item);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&about_item);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&quit_item);

        let tray_icon = TrayIconBuilder::new()
            .with_icon(app_icon()?)
            .with_tooltip("Issen")
            .with_menu(Box::new(menu))
            // Left-click-opens-menu is `tray-icon`'s default (`true`); leaving it
            // on means the first click of a double-click (WM_LBUTTONUP) always
            // opens the menu, which conflicts with double-click opening the main
            // window. The menu still opens on right-click (`menu_on_right_click`
            // stays at its default `true`).
            .with_menu_on_left_click(false)
            .build()
            .ok()?;

        Some(Self {
            icon: tray_icon,
            open_id,
            settings_id,
            reindex_id,
            reindex_item,
            about_id,
        })
    }

    /// Called when a background scan starts/finishes (`app.rs`'s
    /// `start_scan`/`poll_scan`). Progress is surfaced in two places — the
    /// menu item's label swap-and-disable (which also doubles as
    /// preventing a second scan from being triggered) and the tray
    /// tooltip — so it's clear a rescan is actually in progress rather
    /// than just requested. `set_text`/`set_enabled`/`set_tooltip` can fail
    /// at the OS level, but that failure is ignored since it only affects a
    /// display hint, not functionality.
    pub fn set_scanning(&self, strings: &Strings, scanning: bool) {
        if scanning {
            self.reindex_item.set_text(strings.tray_reindex_scanning);
            self.reindex_item.set_enabled(false);
            let _ = self.icon.set_tooltip(Some(strings.tray_tooltip_scanning));
        } else {
            self.reindex_item.set_text(strings.tray_reindex);
            self.reindex_item.set_enabled(true);
            let _ = self.icon.set_tooltip(Some("Issen"));
        }
    }

    /// Matches a `MenuEvent` (received via `take_menu_event_receiver`'s
    /// channel) against this handle's current menu item ids.
    pub fn match_menu_action(&self, event: &MenuEvent) -> Option<TrayAction> {
        if event.id == self.open_id {
            Some(TrayAction::Open)
        } else if event.id == self.settings_id {
            Some(TrayAction::Settings)
        } else if event.id == self.reindex_id {
            Some(TrayAction::Reindex)
        } else if event.id == self.about_id {
            Some(TrayAction::About)
        } else {
            None
        }
    }

    /// Matches a `TrayIconEvent` (received via
    /// `take_tray_icon_event_receiver`'s channel) — a left double-click
    /// opens the main window, same as the tray menu's "Open" item.
    pub fn match_tray_icon_action(event: &TrayIconEvent) -> Option<TrayAction> {
        if let TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } = event
        {
            Some(TrayAction::Open)
        } else {
            None
        }
    }
}

/// Loads Issen's app icon from the exe's own embedded Win32 resource
/// (id 1 — see `build.rs`, which embeds `assets/icon.ico` there via
/// `winresource`) rather than decoding a bundled image file at runtime.
fn app_icon() -> Option<Icon> {
    Icon::from_resource(1, Some((32, 32))).ok()
}

use std::sync::mpsc::{channel, Receiver};
use std::sync::{Mutex, OnceLock};

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::Strings;

/// `MenuEvent::receiver()` is a global channel, but a hidden window receives
/// no OS input events, so nothing would ever call `try_recv()` again and
/// `logic()` would never re-run on a click (the same problem documented on
/// `hotkey.rs::spawn`: a hidden window gets no OS input events, so without
/// this, `update()` never gets called). `MenuEvent::set_event_handler` lets
/// us call `ctx.request_repaint()` as soon as an event arrives, which wakes
/// egui's event loop.
///
/// `set_event_handler` can only be set once per process (`muda` holds it in
/// a `OnceCell`; later calls are ignored), so the receiving channel is a
/// `static` that outlives `TrayHandle` being rebuilt (e.g. on a language
/// switch in `apply_language`).
///
/// `ctx.request_repaint()` above is not actually reliable while the main
/// window is hidden: it schedules a wakeup via eframe's invisible-window
/// repaint machinery, but that wakeup can be silently dropped (confirmed by
/// instrumentation — a `Quit` click's `MenuEvent` arrived here immediately,
/// yet `logic()` didn't run again for several seconds in one run and never
/// did in another, leaving `try_recv_menu_action`'s poll starved forever).
/// Quit can't tolerate that, so it's special-cased right here in the
/// handler — which demonstrably always runs, since `muda` calls it directly
/// from its own message handling rather than through egui's repaint/poll
/// cycle — instead of going through the `TrayAction` channel like every
/// other tray action.
static FORWARDED_EVENTS: OnceLock<Mutex<Receiver<MenuEvent>>> = OnceLock::new();

/// Fixed id for the tray menu's Quit item, so the event handler below can
/// recognize it without needing a `TrayHandle` (which doesn't exist yet when
/// `set_event_handler` is registered — see `ensure_event_forwarding`).
const QUIT_MENU_ID: &str = "issen-tray-quit";

fn ensure_event_forwarding(ctx: &egui::Context) {
    FORWARDED_EVENTS.get_or_init(|| {
        let (tx, rx) = channel::<MenuEvent>();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == MenuId::new(QUIT_MENU_ID) {
                std::process::exit(0);
            }
            let _ = tx.send(event);
            ctx.request_repaint();
        }));
        Mutex::new(rx)
    });
}

/// `TrayIconEvent::set_event_handler` has its own separate process-wide
/// `OnceCell` (a `tray-icon`-side mechanism distinct from `muda`'s), so it
/// needs a second channel following the same pattern as `FORWARDED_EVENTS`
/// for the same two reasons: a hidden window needs `ctx.request_repaint()`
/// to wake up, and the channel needs to outlive `TrayHandle` rebuilds.
static FORWARDED_TRAY_ICON_EVENTS: OnceLock<Mutex<Receiver<TrayIconEvent>>> = OnceLock::new();

fn ensure_tray_icon_event_forwarding(ctx: &egui::Context) {
    FORWARDED_TRAY_ICON_EVENTS.get_or_init(|| {
        let (tx, rx) = channel::<TrayIconEvent>();
        let ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            let _ = tx.send(event);
            ctx.request_repaint();
        }));
        Mutex::new(rx)
    });
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

impl TrayHandle {
    /// Returns `None` if creating the tray icon fails, which can happen at
    /// times other than startup (e.g. `explorer.exe` restarting). Callers
    /// may panic on a startup failure, but a rebuild triggered by something
    /// like a language switch should fall back to keeping the existing
    /// tray icon instead.
    pub fn new(ctx: &egui::Context, strings: &Strings) -> Option<Self> {
        ensure_event_forwarding(ctx);
        ensure_tray_icon_event_forwarding(ctx);

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
            .with_icon(placeholder_icon()?)
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

    pub fn try_recv_action(&self) -> Option<TrayAction> {
        self.try_recv_menu_action()
            .or_else(Self::try_recv_tray_icon_action)
    }

    fn try_recv_menu_action(&self) -> Option<TrayAction> {
        let rx = FORWARDED_EVENTS.get()?;
        let rx = rx.lock().ok()?;
        let event = rx.try_recv().ok()?;
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

    /// Hovering the icon fires frequent `Move`/`Enter` events, so reading
    /// only one event per frame (as `try_recv_menu_action` does) would let
    /// a `DoubleClick` event queue up behind them and get processed several
    /// frames late. Instead, this drains everything currently queued each
    /// frame and returns a hit if a `DoubleClick` was among them.
    fn try_recv_tray_icon_action() -> Option<TrayAction> {
        let rx = FORWARDED_TRAY_ICON_EVENTS.get()?;
        let rx = rx.lock().ok()?;
        let mut action = None;
        while let Ok(event) = rx.try_recv() {
            if let TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } = event
            {
                action = Some(TrayAction::Open);
            }
        }
        action
    }
}

/// Placeholder icon: a solid 32x32 square in the accent color (#F2A93B).
/// Swap for a proper icon resource eventually.
fn placeholder_icon() -> Option<Icon> {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for _ in 0..(SIZE * SIZE) {
        rgba.extend_from_slice(&[0xF2, 0xA9, 0x3B, 0xFF]);
    }
    Icon::from_rgba(rgba, SIZE, SIZE).ok()
}

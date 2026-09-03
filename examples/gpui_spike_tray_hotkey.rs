//! GPUI移行のPhase 0実機スパイク: グローバルホットキー・トレイアイコンとの
//! 共存確認。
//!
//! Issenの現行実装(`src/hotkey.rs`, `src/tray.rs`)は、専用の別OSスレッド
//! (ホットキー)やmuda/tray-icon crateのグローバルイベントハンドラ(トレイ)
//! から`egui::Context::request_repaint()`を呼んでeguiのイベントループを
//! 起こす設計だった。`tray.rs`のコメントにある通り、**隠しウィンドウの間は
//! この`request_repaint()`が信頼できず**(observed: 呼ばれてもlogic()が
//! 何秒も再実行されない)、Quitだけはそれを迂回して直接`process::exit(0)`
//! する特別扱いになっていた。
//!
//! GPUI移行後は「ウィンドウは常にOS-visible(位置をオフスクリーンに退避する
//! だけ)」という同じモデルを踏襲する予定なので、そもそも「隠しウィンドウ」
//! という状態が存在しない。このスパイクでは、
//!   1. ホットキー用の専用スレッド(生のWin32 RegisterHotKey+GetMessageW、
//!      hotkey.rsとほぼ同じ実装)とGPUIのメインループが問題なく共存するか
//!   2. tray-icon crateのグローバルイベントハンドラとの共存
//!   3. 別スレッドからのチャンネル通知を`window.request_animation_frame()`
//!      によるポーリングでGPUI側に伝え、確実にウィンドウが反応するか
//!      (隠しウィンドウがないので、tray.rsが抱えていたrepaint取りこぼし
//!      問題が起きないことを期待している)
//! を確認する。
//!
//! 操作方法: Alt+Space(グローバル、フォーカス不問)でオフスクリーン退避を
//! トグル。トレイアイコン(黄色い32x32のプレースホルダー)を左クリックで
//! 同じトグル、右クリックでQuitメニュー。
//!
//! 実行: `cargo run --example gpui_spike_tray_hotkey`(Windows専用)。

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, OnceLock};
use std::thread;

use gpui::{App, Bounds, Context, Render, Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div, prelude::*, px, rgba, size, white};
use gpui_platform::application;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, MouseButton as TrayMouseButton, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_NOREPEAT, RegisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, GetWindowRect, MSG, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WM_HOTKEY,
};

const OFFSCREEN_X: i32 = -10000;
const OFFSCREEN_Y: i32 = -10000;
const ONSCREEN_X: i32 = 200;
const ONSCREEN_Y: i32 = 200;
const HOTKEY_ID: i32 = 1;
const VK_SPACE: u32 = 0x20;
const QUIT_MENU_ID: &str = "issen-spike-quit";

// `tray.rs`と同じ理由(muda/tray-iconのイベントハンドラはprocess-wideの
// OnceCellで1回しか登録できない)で、これらはグローバルなチャンネル。
static MENU_EVENTS: OnceLock<Mutex<Receiver<MenuEvent>>> = OnceLock::new();
static TRAY_ICON_EVENTS: OnceLock<Mutex<Receiver<TrayIconEvent>>> = OnceLock::new();

fn spawn_hotkey_listener() -> Receiver<()> {
    let (tx, rx) = channel::<()>();
    thread::spawn(move || unsafe {
        if RegisterHotKey(None, HOTKEY_ID, MOD_ALT | MOD_NOREPEAT, VK_SPACE).is_err() {
            eprintln!("[spike] RegisterHotKey(Alt+Space) 失敗");
        }
        loop {
            let mut msg = MSG::default();
            if GetMessageW(&mut msg, None, 0, 0).0 == 0 {
                break;
            }
            if msg.message == WM_HOTKEY {
                let _ = tx.send(());
            }
        }
    });
    rx
}

fn setup_tray() -> Option<()> {
    let (menu_tx, menu_rx) = channel::<MenuEvent>();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = menu_tx.send(event);
    }));
    MENU_EVENTS.set(Mutex::new(menu_rx)).ok();

    let (icon_tx, icon_rx) = channel::<TrayIconEvent>();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = icon_tx.send(event);
    }));
    TRAY_ICON_EVENTS.set(Mutex::new(icon_rx)).ok();

    let menu = Menu::new();
    let quit_item = MenuItem::with_id(QUIT_MENU_ID, "Quit", true, None);
    let _ = menu.append(&quit_item);

    const SIZE: u32 = 32;
    let mut rgba_bytes = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for _ in 0..(SIZE * SIZE) {
        rgba_bytes.extend_from_slice(&[0xF2, 0xA9, 0x3B, 0xFF]);
    }
    let icon = Icon::from_rgba(rgba_bytes, SIZE, SIZE).ok()?;

    let _tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("Issen GPUI spike")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .ok()?;
    // tray-iconはビルダーが返す`TrayIcon`をdropすると消えるので、リークして
    // プロセス終了までトレイアイコンを維持する(本番実装ではIssenApp内で
    // 保持する — このスパイクではシンプルさ優先)。
    std::mem::forget(_tray);
    Some(())
}

fn hwnd(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

struct SpikeWindow {
    is_offscreen: bool,
    hotkey_rx: Receiver<()>,
}

impl SpikeWindow {
    fn do_toggle(&mut self, window: &mut Window) {
        let Some(hwnd) = hwnd(window) else {
            return;
        };
        self.is_offscreen = !self.is_offscreen;
        let (x, y) = if self.is_offscreen {
            (OFFSCREEN_X, OFFSCREEN_Y)
        } else {
            (ONSCREEN_X, ONSCREEN_Y)
        };
        unsafe {
            let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        let mut rect = RECT::default();
        let ok = unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok();
        println!("[spike] toggle -> is_offscreen={} os_rect_ok={} rect=({},{})", self.is_offscreen, ok, rect.left, rect.top);
    }

    /// 各種イベントソース(ホットキー・トレイメニュー・トレイアイコン
    /// クリック)をポーリングする。隠しウィンドウが存在しない設計なので、
    /// `tray.rs`が抱えていた「repaintが取りこぼされてポーリング自体が
    /// 止まる」問題が起きないことを期待している — このポーリングが確実に
    /// 毎フレーム回り続けること自体がその検証になる。
    fn poll_events(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hotkey_rx.try_recv().is_ok() {
            println!("[spike] hotkey pressed");
            self.do_toggle(window);
        }
        if let Some(rx) = MENU_EVENTS.get() {
            if let Ok(rx) = rx.lock() {
                if let Ok(event) = rx.try_recv() {
                    if event.id == MenuId::new(QUIT_MENU_ID) {
                        println!("[spike] quit menu clicked");
                        cx.quit();
                    }
                }
            }
        }
        if let Some(rx) = TRAY_ICON_EVENTS.get() {
            if let Ok(rx) = rx.lock() {
                while let Ok(event) = rx.try_recv() {
                    if let TrayIconEvent::Click { button: TrayMouseButton::Left, .. } = event {
                        println!("[spike] tray icon left click");
                        self.do_toggle(window);
                    }
                }
            }
        }
    }
}

impl Render for SpikeWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.poll_events(window, cx);
        window.request_animation_frame();

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(rgba(0x1e1e1eee))
            .rounded_xl()
            .text_color(white())
            .child("Alt+Space or tray icon click to toggle offscreen")
            .child(format!("is_offscreen: {}", self.is_offscreen))
            .child(format!("window.bounds(): {:?}", window.bounds()))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, _| this.do_toggle(window)))
    }
}

fn run_spike() {
    if setup_tray().is_none() {
        eprintln!("[spike] トレイアイコンの作成に失敗(以降トレイなしで継続)");
    }
    let hotkey_rx = spawn_hotkey_listener();

    application().run(move |cx: &mut App| {
        let bounds = Bounds {
            origin: gpui::point(px(ONSCREEN_X as f32), px(ONSCREEN_Y as f32)),
            size: size(px(480.), px(160.)),
        };

        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: None,
                    focus: true,
                    show: true,
                    kind: WindowKind::PopUp,
                    is_movable: false,
                    window_background: WindowBackgroundAppearance::Transparent,
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| SpikeWindow {
                        is_offscreen: false,
                        hotkey_rx,
                    })
                },
            )
            .unwrap();

        let _ = window;
        cx.activate(true);
    });
}

fn main() {
    run_spike();
}

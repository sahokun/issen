//! GPUI移行(egui/eframe → GPUI)のPhase 0実機スパイク。
//!
//! `docs/../plans`のGPUI移行計画のPhase 0検証項目のうち、最優先の項目を
//! まとめて確認する:
//!   1. raw-window-handle経由でHWNDが取得できるか
//!   2. HWNDを直接`SetWindowPos`で動かすオフスクリーン退避パターンが働くか
//!   3. show/hide切り替え(実際はオフスクリーン⇔実位置move)でちらつきが
//!      起きないか
//!   4. 起動直後の初回合成でDWMの白いプレースホルダーが挟まるか
//!   5. `WS_EX_TOOLWINDOW`相当(タスクバー非表示・Alt+Tab除外)が効くか
//!
//! 表示位置とオフスクリーン位置は1.5秒おきに自動でトグルし続ける(2, 3の確認)。
//! オフスクリーンに退避した瞬間キーボードフォーカスが失われて手動操作を受け
//! 付けなくなるため、フォーカスに依存しない自動トグルにしている。
//!
//! 操作方法(ウィンドウにフォーカスした状態で):
//!   - Space: 即座に手動でもトグル可能
//!   - T: `WS_EX_TOOLWINDOW`スタイルのオン/オフをトグル(5の確認、タスクバー目視)
//!   - Q: 終了
//!
//! 起動直後、実位置に表示されるまでのフレームを画面録画しておけば4を確認できる。
//! ターミナルにはHWND値・トグル後の`window.bounds()`をprintlnするので、
//! `WM_MOVE`経由でGPUI内部のbounds()キャッシュが追従しているかもここで見える
//! (これが古い値のままならGPUI内部状態とのズレがある = 要注意)。
//!
//! 実行: `cargo run --example gpui_spike`(Windows専用。WSL側ではrustcバージョン
//! 不足のためビルド不可 — Windows側のPowerShellで実行すること)。

use gpui::{
    App, Bounds, Context, FocusHandle, Focusable, KeyBinding, Render, Window, WindowBackgroundAppearance,
    WindowBounds, WindowKind, WindowOptions, actions, div, prelude::*, px, rgba, size, white,
};
use gpui_platform::application;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SetWindowLongPtrW, SetWindowPos, WS_EX_TOOLWINDOW,
};

actions!(gpui_spike, [ToggleOffscreen, ToggleToolWindow, Quit]);

// `app.rs::OFFSCREEN_POSITION`相当。通常のモニタレイアウトでは絶対に到達しない
// 座標に置くことで「オフスクリーンに退避済みか」を判定できる(`window_is_offscreen`)。
const OFFSCREEN_X: i32 = -10000;
const OFFSCREEN_Y: i32 = -10000;
const ONSCREEN_X: i32 = 200;
const ONSCREEN_Y: i32 = 200;

struct SpikeWindow {
    focus_handle: FocusHandle,
    is_offscreen: bool,
    is_tool_window: bool,
    /// キー入力はオフスクリーンに退避した瞬間フォーカスが失われて届かなくなる
    /// (画面外のウィンドウをクリックし直せないため)。フォーカス状態に依存せず
    /// ちらつきを繰り返し確認できるよう、`render`から毎フレーム経過時間を見て
    /// 自動的にトグルする。
    last_auto_toggle: std::time::Instant,
}

const AUTO_TOGGLE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

impl SpikeWindow {
    /// `app.rs::window_hwnd`と同じパターン。GPUIの`Window`が`HasWindowHandle`を
    /// 公開実装している(`gpui/src/window.rs:6819`で確認済み)ことに依存する。
    /// 注意: `Window`には同名のinherentメソッド`window_handle()`(`AnyWindowHandle`
    /// というGPUI独自の型を返す、ウィンドウ識別子でありHWNDではない)があるため、
    /// トレイトメソッドを呼ぶには`HasWindowHandle::window_handle(window)`のように
    /// 完全修飾で呼ぶ必要がある。
    fn hwnd(window: &Window) -> Option<HWND> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        match handle.as_raw() {
            RawWindowHandle::Win32(handle) => {
                Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void))
            }
            _ => None,
        }
    }

    /// 実処理本体。キー操作(`toggle_offscreen`)とタイマー自動トグル
    /// (`render`)の両方から呼ぶ。
    fn do_toggle_offscreen(&mut self, window: &mut Window) {
        let Some(hwnd) = Self::hwnd(window) else {
            println!("[spike] HWND取得失敗 — HasWindowHandle実装が公開されていないか、raw handleがWin32でない");
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
        // GetWindowRectで実際にOS側の位置がSetWindowPos通りに動いたかを確認しつつ、
        // window.bounds()(GPUI内部のoriginキャッシュ、WM_MOVE経由で更新されるはず)
        // と突き合わせて内部状態が追従しているかを見る。
        let mut rect = RECT::default();
        let os_rect_ok = unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok();
        println!(
            "[spike] toggle_offscreen -> is_offscreen={} os_rect_ok={} os_rect=({},{}) gpui_bounds={:?}",
            self.is_offscreen,
            os_rect_ok,
            rect.left,
            rect.top,
            window.bounds(),
        );
    }

    fn toggle_offscreen(&mut self, _: &ToggleOffscreen, window: &mut Window, cx: &mut Context<Self>) {
        self.do_toggle_offscreen(window);
        cx.notify();
    }

    fn toggle_tool_window(&mut self, _: &ToggleToolWindow, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hwnd) = Self::hwnd(window) else {
            return;
        };
        self.is_tool_window = !self.is_tool_window;
        unsafe {
            let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let new_style = if self.is_tool_window {
                current | (WS_EX_TOOLWINDOW.0 as isize)
            } else {
                current & !(WS_EX_TOOLWINDOW.0 as isize)
            };
            let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
        }
        println!("[spike] toggle_tool_window -> is_tool_window={}(タスクバー/Alt+Tabを目視確認)", self.is_tool_window);
        cx.notify();
    }

    fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }
}

impl Focusable for SpikeWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SpikeWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // フォーカスの有無に関わらず、一定間隔でオフスクリーン⇔実位置を
        // 自動的にトグルし続ける(項目3のちらつき確認をキー入力なしで行える
        // ようにするため)。`request_animation_frame`で毎フレーム呼ばれ続ける。
        if self.last_auto_toggle.elapsed() >= AUTO_TOGGLE_INTERVAL {
            self.do_toggle_offscreen(window);
            self.last_auto_toggle = std::time::Instant::now();
        }
        window.request_animation_frame();

        div()
            .track_focus(&self.focus_handle)
            .key_context("gpui_spike")
            .on_action(cx.listener(Self::toggle_offscreen))
            .on_action(cx.listener(Self::toggle_tool_window))
            .on_action(cx.listener(Self::quit))
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(rgba(0x1e1e1eee))
            .rounded_xl()
            .text_color(white())
            .child("GPUI spike — Space: offscreen toggle, T: tool-window toggle, Q: quit")
            .child(format!("is_offscreen: {}", self.is_offscreen))
            .child(format!("is_tool_window: {}", self.is_tool_window))
            .child(format!("window.bounds(): {:?}", window.bounds()))
    }
}

fn run_spike() {
    application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("space", ToggleOffscreen, Some("gpui_spike")),
            KeyBinding::new("t", ToggleToolWindow, Some("gpui_spike")),
            KeyBinding::new("q", Quit, Some("gpui_spike")),
        ]);

        // 項目4(初回合成フリッカー)確認のため、起動直後からいきなり実位置に
        // 表示する(Issenの本番と同じくOS-level Show/Hideは使わず常時visible)。
        // ここでDWMの白いプレースホルダーが一瞬でも見えるかを画面録画で確認する。
        let bounds = Bounds {
            origin: gpui::point(px(ONSCREEN_X as f32), px(ONSCREEN_Y as f32)),
            size: size(px(480.), px(160.)),
        };

        cx.open_window(
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
            |window, cx| {
                cx.new(|cx| {
                    let focus_handle = cx.focus_handle();
                    focus_handle.focus(window, cx);
                    SpikeWindow {
                        focus_handle,
                        is_offscreen: false,
                        is_tool_window: false,
                        last_auto_toggle: std::time::Instant::now(),
                    }
                })
            },
        )
        .unwrap();

        cx.activate(true);
    });
}

fn main() {
    run_spike();
}

//! Shell icons are extracted off the UI thread and cached (including misses).
//! Keep the plugin ABI unchanged: an icon is derived from a result's action.
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{Context, RenderImage};
use windows::core::PCWSTR;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY,
};

use crate::search::Action;

const CACHE_CAPACITY: usize = 256;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum IconSource {
    File(PathBuf),
    App(String),
}

impl IconSource {
    pub fn for_action(action: &Action) -> Option<Self> {
        match action {
            Action::Launch { path, .. } => Some(Self::File(path.clone())),
            Action::LaunchUwp { aumid } => Some(Self::App(aumid.clone())),
            _ => None,
        }
    }

    fn parsing_name(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        match self {
            Self::File(path) => path.as_os_str().encode_wide().chain(Some(0)).collect(),
            Self::App(aumid) => format!("shell:AppsFolder\\{aumid}")
                .encode_utf16()
                .chain(Some(0))
                .collect(),
        }
    }
}

/// Segoe MDL2 glyphs provide stable, monochrome icons for non-file actions
/// and a placeholder while shell extraction is pending or unavailable.
pub fn fallback(action: &Action) -> &'static str {
    match action {
        Action::OpenUri(uri) if uri.starts_with("ms-settings:") => "\u{e713}",
        Action::OpenUri(_) => "\u{e774}",
        Action::CopyToClipboard(_) => "\u{e8c8}",
        Action::LaunchUwp { .. } => "\u{e71d}",
        Action::Launch { .. } => "\u{e8a5}",
    }
}

#[derive(Default)]
pub struct IconCache {
    images: HashMap<IconSource, Option<Arc<RenderImage>>>,
    order: VecDeque<IconSource>,
    loading: bool,
}

impl IconCache {
    pub fn get(&self, source: &IconSource) -> Option<Arc<RenderImage>> {
        self.images.get(source).and_then(Clone::clone)
    }

    pub fn request(&mut self, sources: Vec<IconSource>, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let mut seen = HashSet::new();
        let sources: Vec<_> = sources
            .into_iter()
            .filter(|source| !self.images.contains_key(source) && seen.insert(source.clone()))
            .collect();
        if sources.is_empty() {
            return;
        }
        self.loading = true;
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    sources
                        .into_iter()
                        .map(|source| {
                            let image = load_shell_icon(&source);
                            (source, image)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |cache, cx| {
                for (source, image) in loaded {
                    while cache.images.len() >= CACHE_CAPACITY {
                        if let Some(oldest) = cache.order.pop_front() {
                            cache.images.remove(&oldest);
                        }
                    }
                    cache.order.push_back(source.clone());
                    cache.images.insert(source, image);
                }
                cache.loading = false;
                cx.notify();
            });
        })
        .detach();
    }
}

fn load_shell_icon(source: &IconSource) -> Option<Arc<RenderImage>> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().ok()?;
        let result = (|| {
            let name = source.parsing_name();
            let factory: IShellItemImageFactory =
                SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None).ok()?;
            let bitmap = factory
                .GetImage(SIZE { cx: 32, cy: 32 }, SIIGBF_ICONONLY)
                .ok()?;
            let image = bitmap_image(bitmap);
            let _ = DeleteObject(bitmap.into());
            image
        })();
        CoUninitialize();
        result
    }
}

/// GPUI consumes BGRA, matching a top-down 32-bit Windows DIB.
unsafe fn bitmap_image(bitmap: HBITMAP) -> Option<Arc<RenderImage>> {
    let mut object = BITMAP::default();
    if GetObjectW(
        bitmap.into(),
        std::mem::size_of::<BITMAP>() as i32,
        Some((&mut object as *mut BITMAP).cast()),
    ) == 0
    {
        return None;
    }
    let (width, height) = (object.bmWidth, object.bmHeight);
    if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
        return None;
    }
    let dc = CreateCompatibleDC(None);
    if dc.0.is_null() {
        return None;
    }
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0; width as usize * height as usize * 4];
    let rows = GetDIBits(
        dc,
        bitmap,
        0,
        height as u32,
        Some(pixels.as_mut_ptr().cast()),
        &mut info,
        DIB_RGB_COLORS,
    );
    let _ = DeleteDC(dc);
    if rows != height {
        return None;
    }
    let buffer = image::RgbaImage::from_raw(width as u32, height as u32, pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_real_windows_executable_icon() {
        let windows = PathBuf::from(std::env::var_os("WINDIR").unwrap());
        let icon = load_shell_icon(&IconSource::File(windows.join("System32\\cmd.exe")))
            .expect("Windows shell should supply cmd.exe's icon");
        assert!(icon.as_bytes(0).unwrap().chunks_exact(4).any(|p| p[3] != 0));
    }

    #[test]
    fn unavailable_shell_item_falls_back_without_panicking() {
        assert!(load_shell_icon(&IconSource::File(PathBuf::from(
            "C:\\issen-nonexistent-icon-test\\missing.exe"
        )))
        .is_none());
    }

    #[test]
    fn extracts_packaged_app_icons_when_apps_are_installed() {
        let apps = crate::search::uwp::scan();
        // Minimal Windows installations need not have Store apps.
        if !apps.is_empty() {
            assert!(apps
                .into_iter()
                .any(|app| { load_shell_icon(&IconSource::App(app.aumid)).is_some() }));
        }
    }
}

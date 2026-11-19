//! Copies terminal text to the Windows clipboard. The WebView's
//! `navigator.clipboard` refuses to write while the window does not have
//! focus, which drops an OSC 52 copy from tmux that arrives after the user
//! has already clicked into another app, and it waits only a few milliseconds
//! for another app to release the clipboard.

/// Attempts to open the clipboard, which another app may be holding briefly
/// (clipboard managers, IMEs, remote desktop), and the pause between them.
#[cfg(windows)]
const OPEN_ATTEMPTS: u32 = 20;
#[cfg(windows)]
const OPEN_RETRY: std::time::Duration = std::time::Duration::from_millis(25);

/// Replaces the clipboard's contents with `text`. `owner` is the app's window.
#[cfg(windows)]
pub fn write_text(owner: windows::Win32::Foundation::HWND, text: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{GlobalFree, HANDLE};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    // SAFETY: the clipboard is opened and closed on this thread; the memory is
    // allocated for `wide`'s length, and is freed here unless the clipboard
    // took ownership of it.
    unsafe {
        let mut attempt = 1;
        while let Err(e) = OpenClipboard(Some(owner)) {
            if attempt == OPEN_ATTEMPTS {
                return Err(format!(
                    "another app is using the clipboard ({})",
                    e.message()
                ));
            }
            attempt += 1;
            std::thread::sleep(OPEN_RETRY);
        }
        let result = (|| {
            EmptyClipboard().map_err(|e| format!("cannot empty the clipboard: {}", e.message()))?;
            let size = wide.len() * size_of::<u16>();
            let memory = GlobalAlloc(GMEM_MOVEABLE, size)
                .map_err(|e| format!("cannot allocate the clipboard text: {}", e.message()))?;
            let target = GlobalLock(memory) as *mut u16;
            if target.is_null() {
                let _ = GlobalFree(Some(memory));
                return Err("cannot lock the clipboard text".to_string());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
            // Reports an error once the memory is unlocked, which it now is.
            let _ = GlobalUnlock(memory);
            SetClipboardData(CF_UNICODETEXT.0.into(), Some(HANDLE(memory.0)))
                .map(|_| ())
                .map_err(|e| {
                    let _ = GlobalFree(Some(memory));
                    format!("cannot set the clipboard text: {}", e.message())
                })
        })();
        let _ = CloseClipboard();
        result
    }
}

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

    use super::*;

    fn read_text() -> String {
        // SAFETY: the clipboard is opened and closed on this thread, and the
        // text handle is only read while locked.
        unsafe {
            OpenClipboard(None).unwrap();
            let handle = GetClipboardData(CF_UNICODETEXT.0.into()).unwrap();
            let memory = windows::Win32::Foundation::HGLOBAL(handle.0);
            let text = GlobalLock(memory) as *const u16;
            let len = (0..).take_while(|&i| *text.add(i) != 0).count();
            let read = String::from_utf16(std::slice::from_raw_parts(text, len)).unwrap();
            let _ = GlobalUnlock(memory);
            CloseClipboard().unwrap();
            read
        }
    }

    // Replaces whatever the user has on the clipboard, so it runs only when asked.
    #[test]
    #[ignore]
    fn writes_unicode_text_and_waits_for_a_busy_clipboard() {
        // SAFETY: returns the desktop window's handle; nothing else.
        let owner = unsafe { GetDesktopWindow() };
        let text = "ghi chú\r\n\u{1F600} tab\there";
        write_text(owner, text).unwrap();
        assert_eq!(read_text(), text);

        // Another app holds the clipboard for a while, then lets go.
        let (opened, held) = std::sync::mpsc::channel();
        // SAFETY: the clipboard is opened and closed on the holder's thread.
        let holder = std::thread::spawn(move || unsafe {
            OpenClipboard(None::<HWND>).unwrap();
            opened.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(200));
            CloseClipboard().unwrap();
        });
        held.recv().unwrap();
        write_text(owner, "after the wait").unwrap();
        holder.join().unwrap();
        assert_eq!(read_text(), "after the wait");
    }
}

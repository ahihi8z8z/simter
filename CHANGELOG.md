# Changelog

All notable changes to Simter are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Open remote files.** Double-click a file, press Enter, or choose **Open** in its right-click
  menu to download a read-only copy to a temporary folder and open it with its Windows app.
  Changes to the copy are not uploaded. Programs and scripts are never run: they open with the
  app you pick in **Open with**.

### Changed

- Hosts no longer move when you connect. Recently opened hosts are still listed first, but the
  order is updated only when Simter starts and on **Refresh**.

### Fixed

- Copying from the terminal sometimes left the clipboard unchanged: text selected in tmux/Byobu
  was lost when you clicked into another app before it arrived from the host, and a copy gave up
  when another app was briefly using the clipboard. Simter now copies whether or not its window
  has focus, waits up to half a second for the clipboard, and says so when a copy fails.
- After **Reconnect** from inside tmux/Byobu with mouse mode on, moving the mouse no longer types
  text such as `35;18;5M` into the new shell. Simter now turns off the modes the old connection
  left on (mouse reporting, the full-screen view, bracketed paste) before connecting again.

## [0.2.0] - 2026-10-27

Faster file browsing, file actions in right-click menus, real transfer progress, and a way to
reconnect after the PC wakes from sleep.

### Added

- **Right-click menus in each area.** Each part of the window has its own menu:
  - **Hosts:** Connect, Open config, Refresh.
  - **Files:** Upload files…, Upload folder…, Refresh, Download…, Delete…
  - **Terminal:** Copy, Paste, Reconnect.
  - **Tabs:** Reconnect, Close.

  The WebView menu (Back, Refresh, Save as, Print…) no longer appears. Text boxes still have
  Cut/Copy/Paste. While tmux/Byobu mouse mode is on, a right-click in the terminal goes to tmux;
  use **Shift+right-click** to open Simter's menu.
- **Reconnect** from the terminal's or the tab's right-click menu. It closes the tab's ssh and
  file connections and opens new ones in the same tab, even when the old ones still look alive
  after the PC slept. Byobu/tmux sessions on the host keep running, so you can attach to them
  again as usual.
- **Delete remote files and folders.** Delete always asks first, names the item, and warns that
  it cannot be undone. A folder is deleted with everything in it. A link is deleted without
  touching what it points to.
- **Transfer progress:** a progress bar and percentage, the amount copied out of the total size,
  the current speed, and for a folder how many files are done out of how many.

### Changed

- **Faster file browsing.** Each tab keeps one connection open for browsing, so opening a folder
  no longer starts a new SSH login. If that connection drops, the next action opens a new one.
- **File actions are in the right-click menu.** Click an item to select it, then right-click it
  for **Download…** and **Delete…**; right-click anywhere else in the list for **Upload
  files…**, **Upload folder…** and **Refresh**. The context-menu key and Shift+F10 also work.
- Nothing is selected when a folder opens, so a right-click always has somewhere to upload.

### Removed

- The Upload files, Upload folder and Download buttons in the file pane (use the right-click
  menu).

### Fixed

- Text selected inside tmux/Byobu with mouse mode on now reaches the Windows clipboard, as it
  does in Windows Terminal. The host can write to your clipboard this way but can never read it.
- Closing a tab or reconnecting while ssh was still starting no longer leaves an ssh process
  running in the background.

## [0.1.0] - 2026-10-21

First release.

### Added

- Host list from the SSH config, including `Include`d files, with search, keyboard selection,
  recent hosts first, and the effective `user@hostname:port` from `ssh -G`.
- One tab per host with an embedded terminal running the Windows OpenSSH `ssh <alias>`, with
  connection state and Reconnect. Function keys, including Byobu's F2–F9, reach the host.
- SFTP file pane beside the terminal: browse, upload and download files and folders, with
  Overwrite/Skip/Cancel when the target exists.
- **Open config** at the host's line in VS Code or Notepad, Refresh, and a choice of SSH config
  file.
- Password, passphrase and host-key prompts in a dialog, with an option to save a password in
  Windows Credential Manager.
- Resizable, collapsible side bar; the layout is saved.

[0.2.0]: https://github.com/ahihi8z8z/simter/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ahihi8z8z/simter/releases/tag/v0.1.0

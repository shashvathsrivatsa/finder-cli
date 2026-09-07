mod app;
mod column;
mod entry;
mod grouped;
mod rename;
mod ui;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering}};
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, DisableFocusChange, EnableFocusChange, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::{App, ClipboardEntry, ClipboardOp, ConvertState, PaneInfo, CLIPBOARD_FLASH_MS, PAGE_JUMP, convert_formats_for, save_favorites, unique_output_path};
use rename::{RenameMode, RenameState};
use ui::render;

fn copy_dest(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext  = path.extension().and_then(|s| s.to_str());
    let make = |n: u32| {
        let suffix = if n == 1 { "copy".to_string() } else { format!("copy {}", n) };
        if let Some(e) = ext { format!("{} {}.{}", stem, suffix, e) } else { format!("{} {}", stem, suffix) }
    };
    (1u32..).map(|n| parent.join(make(n))).find(|p| !p.exists()).unwrap()
}

fn copy_dir(src: &Path, dst: &Path, progress: &Arc<AtomicUsize>) -> io::Result<()> {
    std::fs::create_dir(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let ft = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ft.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            std::os::unix::fs::symlink(&target, &dst_path)?;
            progress.fetch_add(1, Ordering::Relaxed);
        } else if ft.is_dir() {
            copy_dir(&entry.path(), &dst_path, progress)?;
        } else {
            std::fs::copy(&entry.path(), &dst_path)?;
            progress.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    if path.is_symlink() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }
    if path.is_file() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }
    let Ok(rd) = std::fs::read_dir(path) else { return 0; };
    rd.flatten().map(|e| dir_size(&e.path())).sum()
}

fn count_files(path: &Path) -> usize {
    if path.is_file() { return 1; }
    let Ok(rd) = std::fs::read_dir(path) else { return 0; };
    rd.flatten().map(|e| count_files(&e.path())).sum()
}

fn read_image_dims(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    let mut f = std::fs::File::open(path).ok()?;
    match ext.as_str() {
        "png" => {
            let mut buf = [0u8; 24];
            f.read_exact(&mut buf).ok()?;
            if &buf[0..8] != b"\x89PNG\r\n\x1a\n" { return None; }
            let w = u32::from_be_bytes(buf[16..20].try_into().ok()?);
            let h = u32::from_be_bytes(buf[20..24].try_into().ok()?);
            Some((w, h))
        }
        "jpg" | "jpeg" => {
            let mut data = Vec::new();
            f.read_to_end(&mut data).ok()?;
            let mut i = 0usize;
            while i + 1 < data.len() {
                if data[i] != 0xFF { break; }
                let marker = data[i + 1];
                if matches!(marker, 0xC0 | 0xC1 | 0xC2) && i + 9 < data.len() {
                    let h = u16::from_be_bytes([data[i+5], data[i+6]]) as u32;
                    let w = u16::from_be_bytes([data[i+7], data[i+8]]) as u32;
                    return Some((w, h));
                }
                if marker == 0xD8 || marker == 0xFF { i += 1; continue; }
                if i + 3 >= data.len() { break; }
                let len = u16::from_be_bytes([data[i+2], data[i+3]]) as usize;
                i += 2 + len;
            }
            None
        }
        "gif" => {
            let mut buf = [0u8; 10];
            f.read_exact(&mut buf).ok()?;
            if &buf[0..3] != b"GIF" { return None; }
            let w = u16::from_le_bytes([buf[6], buf[7]]) as u32;
            let h = u16::from_le_bytes([buf[8], buf[9]]) as u32;
            Some((w, h))
        }
        "webp" => {
            let mut buf = [0u8; 30];
            f.read_exact(&mut buf).ok()?;
            if &buf[0..4] != b"RIFF" || &buf[8..12] != b"WEBP" { return None; }
            if &buf[12..16] == b"VP8 " {
                let w = (u16::from_le_bytes([buf[26], buf[27]]) & 0x3FFF) as u32;
                let h = (u16::from_le_bytes([buf[28], buf[29]]) & 0x3FFF) as u32;
                Some((w, h))
            } else if &buf[12..16] == b"VP8L" {
                let bits = u32::from_le_bytes([buf[21], buf[22], buf[23], buf[24]]);
                let w = (bits & 0x3FFF) + 1;
                let h = ((bits >> 14) & 0x3FFF) + 1;
                Some((w, h))
            } else { None }
        }
        "bmp" => {
            let mut buf = [0u8; 26];
            f.read_exact(&mut buf).ok()?;
            if &buf[0..2] != b"BM" { return None; }
            let w = u32::from_le_bytes(buf[18..22].try_into().ok()?);
            let h = u32::from_le_bytes(buf[22..26].try_into().ok()?);
            Some((w, h))
        }
        "tiff" | "tif" => {
            use std::io::{Seek, SeekFrom};
            let mut hdr = [0u8; 8];
            f.read_exact(&mut hdr).ok()?;
            let le = &hdr[0..2] == b"II";
            let u16_ = |b: [u8;2]| if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) };
            let u32_ = |b: [u8;4]| if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) };
            if u16_([hdr[2], hdr[3]]) != 42 { return None; }
            let ifd_off = u32_([hdr[4], hdr[5], hdr[6], hdr[7]]) as u64;
            f.seek(SeekFrom::Start(ifd_off)).ok()?;
            let mut cnt_buf = [0u8; 2];
            f.read_exact(&mut cnt_buf).ok()?;
            let count = u16_(cnt_buf) as usize;
            let mut w = None::<u32>;
            let mut h = None::<u32>;
            for _ in 0..count {
                let mut entry = [0u8; 12];
                f.read_exact(&mut entry).ok()?;
                let tag = u16_([entry[0], entry[1]]);
                let typ = u16_([entry[2], entry[3]]);
                let val = match typ {
                    3 => u16_([entry[8], entry[9]]) as u32, // SHORT
                    _ => u32_([entry[8], entry[9], entry[10], entry[11]]), // LONG
                };
                match tag {
                    256 => w = Some(val),
                    257 => h = Some(val),
                    _ => {}
                }
                if w.is_some() && h.is_some() { break; }
            }
            w.zip(h)
        }
        "heic" | "heif" | "avif" => {
            // ISOBMFF container: scan first 64KB for the `ispe` box (image spatial extents)
            // ispe layout: size(4) type(4="ispe") version+flags(4) width(4) height(4)
            let mut data = vec![0u8; 65536];
            let n = f.read(&mut data).ok()?;
            let data = &data[..n];
            let needle = b"ispe";
            data.windows(needle.len()).enumerate().find_map(|(i, w)| {
                if w != needle { return None; }
                let base = i + 4; // skip past "ispe"
                if base + 8 > data.len() { return None; }
                // skip 4-byte version+flags
                let width  = u32::from_be_bytes(data[base+4..base+8].try_into().ok()?);
                let height = u32::from_be_bytes(data[base+8..base+12].try_into().ok()?);
                if width == 0 || height == 0 { return None; }
                Some((width, height))
            })
        }
        _ => None,
    }
}

// returns (width, height, fps*100, duration_secs)
fn read_video_info(path: &Path) -> Option<(u32, u32, i64, i64)> {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-select_streams", "v:0",
               "-show_entries", "stream=width,height,r_frame_rate,duration",
               "-of", "csv=p=0"])
        .arg(path)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().next()?;
    let parts: Vec<&str> = line.split(',').collect();
    if parts.len() < 4 { return None; }
    let w: u32 = parts[0].trim().parse().ok()?;
    let h: u32 = parts[1].trim().parse().ok()?;
    // r_frame_rate is like "30/1" or "2997/100"
    let fps_i64 = parts[2].trim().split('/').collect::<Vec<_>>().as_slice().chunks(2).next()
        .and_then(|_| {
            let nums: Vec<f64> = parts[2].split('/').filter_map(|n| n.parse().ok()).collect();
            if nums.len() == 2 && nums[1] != 0.0 { Some(((nums[0] / nums[1]) * 100.0).round() as i64) }
            else { None }
        })?;
    let dur: i64 = parts[3].trim().parse::<f64>().ok().map(|d| d.round() as i64).unwrap_or(-1);
    Some((w, h, fps_i64, dur))
}

fn delete_recursive(path: &Path, progress: &Arc<AtomicUsize>) {
    if path.is_symlink() || !path.is_dir() {
        let _ = std::fs::remove_file(path);
        progress.fetch_add(1, Ordering::Relaxed);
    } else {
        if let Ok(rd) = std::fs::read_dir(path) {
            for e in rd.flatten() { delete_recursive(&e.path(), progress); }
        }
        let _ = std::fs::remove_dir(path);
    }
}

fn list_panes() -> Vec<PaneInfo> {
    let current_pane = std::env::var("TMUX_PANE").unwrap_or_default();
    let current_session = std::process::Command::new("tmux")
        .args(["display-message", "-p", "#S"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let Ok(out) = std::process::Command::new("tmux")
        .args(["list-panes", "-a", "-F", "#{pane_id}\t#{session_name}\t#{session_name}:#{window_index}.#{pane_index}\t#{pane_current_command}"])
        .output() else { return vec![]; };
    let mut panes: Vec<PaneInfo> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            let id      = parts.next()?.to_string();
            let session = parts.next()?.to_string();
            let coord   = parts.next()?.to_string();
            let cmd     = parts.next()?.trim().to_string();
            if id == current_pane { return None; }
            if cmd != "nvim" { return None; }
            let same_session = session == current_session;
            Some(PaneInfo { id, label: format!("{}  {}", coord, cmd), same_session })
        })
        .collect();
    // current session first
    panes.sort_by_key(|p| !p.same_session);
    panes
}

fn open_in_linked_pane(pane_id: &str, path: &Path) {
    let path_str = path.to_string_lossy();
    let _ = std::process::Command::new("tmux")
        .args(["send-keys", "-t", pane_id, &format!(":e {}\r", path_str)])
        .status();
    let _ = std::process::Command::new("tmux")
        .args(["select-pane", "-t", pane_id])
        .status();
}

fn unique_dest(dir: &Path, filename: &std::ffi::OsStr, src: &Path, is_move: bool) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() || (is_move && candidate == src) {
        return candidate;
    }
    let name = Path::new(filename);
    let stem = name.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext = name.extension().and_then(|s| s.to_str());
    let mut i = 0usize;
    loop {
        let new_name = if i == 0 {
            if let Some(e) = ext { format!("{} copy.{}", stem, e) } else { format!("{} copy", stem) }
        } else {
            if let Some(e) = ext { format!("{} copy {}.{}", stem, i + 1, e) } else { format!("{} copy {}", stem, i + 1) }
        };
        let candidate = dir.join(&new_name);
        if !candidate.exists() { return candidate; }
        i += 1;
    }
}

fn do_paste(entry: &ClipboardEntry, dst: &Path, progress: &Arc<AtomicUsize>) -> io::Result<()> {
    if entry.path == dst { return Ok(()); }
    match entry.op {
        ClipboardOp::Cut  => { std::fs::rename(&entry.path, dst)?; progress.fetch_add(1, Ordering::Relaxed); }
        ClipboardOp::Copy => {
            if entry.path.is_dir() { copy_dir(&entry.path, dst, progress)?; }
            else { std::fs::copy(&entry.path, dst).map(|_| ())?; progress.fetch_add(1, Ordering::Relaxed); }
        }
    }
    Ok(())
}

fn open_tty() -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")
}

fn load_hidden_paths() -> std::collections::HashSet<PathBuf> {
    let exe_dir = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let candidates = [
        exe_dir.as_deref().map(|d| d.join("hidden")),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hidden")),
        Some(PathBuf::from("hidden")),
    ];
    let home = std::env::var("HOME").unwrap_or_default();
    for path in candidates.into_iter().flatten() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return text.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| {
                    let expanded = l.replacen("~/", &format!("{}/", home), 1);
                    PathBuf::from(expanded)
                })
                .collect();
        }
    }
    std::collections::HashSet::new()
}

fn load_default_app_exts() -> std::collections::HashSet<String> {
    let exe_dir = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    // Look next to the binary, then next to Cargo.toml (dev), then CWD
    let candidates = [
        exe_dir.as_deref().map(|d| d.join("defaults")),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("defaults")),
        Some(PathBuf::from("defaults")),
    ];
    for path in candidates.into_iter().flatten() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return text.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.to_lowercase())
                .collect();
        }
    }
    std::collections::HashSet::new()
}

fn open_in_default_app(path: &Path) {
    let _ = std::process::Command::new("open").arg(path).status();
}

fn open_in_nvim(path: &Path) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(open_tty()?, LeaveAlternateScreen, DisableMouseCapture)?;
    let tty_in  = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let tty_out = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let tty_err = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    std::process::Command::new("nvim")
        .arg(path)
        .stdin(tty_in)
        .stdout(tty_out)
        .stderr(tty_err)
        .status()?;
    enable_raw_mode()?;
    execute!(open_tty()?, EnterAlternateScreen, EnableMouseCapture, EnableFocusChange)?;
    Ok(())
}

fn main() -> io::Result<()> {
    let start = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    let mut shelf_proc = std::process::Command::new("shelf")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn().ok();

    enable_raw_mode()?;
    let mut tty = open_tty()?;
    execute!(tty, EnterAlternateScreen, EnableMouseCapture, EnableFocusChange)?;
    let backend = CrosstermBackend::new(tty);
    let mut terminal = Terminal::new(backend)?;

    let default_app_exts = load_default_app_exts();
    crate::entry::set_hidden_paths(load_hidden_paths());
    let mut app = App::new(start);

    let mut last_refresh = std::time::Instant::now();
    const IDLE_REFRESH_MS: u64 = 250;
    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            terminal.draw(|f| render(f, &mut app))?;
            needs_redraw = false;
        }

        // Poll background delete/paste completion
        if let Some(ref rx) = app.bg_done_rx {
            match rx.try_recv() {
                Ok(()) => {
                    app.bg_done_rx = None;
                    app.bg_progress = None;
                    app.bg_total = None;
                    let was_deleting = app.is_deleting;
                    let convert_output = app.convert_output.take();
                    app.is_deleting = false;
                    app.is_pasting = false;
                    app.is_converting = false;
                    app.selection.clear(); app.selection_anchor = None; app.select_mode = false;
                    app.refresh();
                    if let Some(ref out) = convert_output {
                        if out.exists() {
                            let col = &mut app.columns[app.active_col];
                            if let Some(row) = col.grouped.row_to_entry.iter().position(|&i| col.grouped.entries[i].path == *out) {
                                col.selected_row = row;
                                col.sync_list_state();
                            }
                        } else {
                            let ext = out.extension().and_then(|s| s.to_str()).unwrap_or("pdf");
                            let tool = if ext == "pdf" { "soffice" } else { "ffmpeg/magick" };
                            app.status_flash = Some((
                                format!("Error: conversion failed (is {} installed?)", tool),
                                std::time::Instant::now(),
                            ));
                        }
                    }
                    if was_deleting {
                        let col = &mut app.columns[app.active_col];
                        if col.selected_row >= col.grouped.row_count && col.selected_row > 0 {
                            col.selected_row -= 1;
                        }
                    }
                    app.maybe_push_child_column();
                    needs_redraw = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    app.spinner_frame = app.spinner_frame.wrapping_add(1);
                    needs_redraw = true;
                }
                Err(_) => {
                    app.bg_done_rx = None;
                    app.bg_progress = None;
                    app.bg_total = None;
                    app.is_deleting = false;
                    app.is_pasting = false;
                    app.is_converting = false;
                    needs_redraw = true;
                }
            }
        }

        // Trigger preview size computation after 1s of inactivity
        let inactive_ms = app.last_key_at.elapsed().as_millis();
        let wait_to_load_preview = app::PREVIEW_DELAY_MS;
        if inactive_ms >= wait_to_load_preview {
            let col = &app.columns[app.active_col];
            let cur_path = col.grouped.entry_at_row(col.selected_row).map(|e| e.path.clone());
            let needs_preview = cur_path.as_ref().map_or(false, |p| app.preview_path.as_ref() != Some(p));
            if needs_preview {
                let path = cur_path.unwrap();
                app.preview_path = Some(path.clone());
                let size_cell = Arc::new(AtomicU64::new(u64::MAX));
                let modified_cell = Arc::new(AtomicI64::new(i64::MIN));
                let created_cell = Arc::new(AtomicI64::new(i64::MIN));
                let count_cell = Arc::new(AtomicI64::new(i64::MIN));
                let dims_cell = Arc::new(AtomicI64::new(i64::MIN));
                let fps_cell = Arc::new(AtomicI64::new(i64::MIN));
                let duration_cell = Arc::new(AtomicI64::new(i64::MIN));
                app.preview_size = Some(size_cell.clone());
                app.preview_modified = Some(modified_cell.clone());
                app.preview_created = Some(created_cell.clone());
                app.preview_count = Some(count_cell.clone());
                app.preview_dims = Some(dims_cell.clone());
                app.preview_fps = Some(fps_cell.clone());
                app.preview_duration = Some(duration_cell.clone());
                std::thread::spawn(move || {
                    let size = dir_size(&path);
                    size_cell.store(size, Ordering::Relaxed);
                    let meta = path.symlink_metadata().ok();
                    let modified = meta.as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(-1);
                    modified_cell.store(modified, Ordering::Relaxed);
                    let created = meta.as_ref()
                        .and_then(|m| m.created().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(-1);
                    created_cell.store(created, Ordering::Relaxed);
                    let count = if path.is_dir() {
                        std::fs::read_dir(&path).map(|rd| rd.flatten().count() as i64).unwrap_or(-1)
                    } else { -1 };
                    count_cell.store(count, Ordering::Relaxed);

                    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                    let is_image = matches!(ext.as_str(), "png"|"jpg"|"jpeg"|"gif"|"webp"|"bmp"|"tiff"|"tif"|"heic"|"heif"|"avif");
                    let is_video = matches!(ext.as_str(), "mp4"|"mov"|"avi"|"mkv"|"webm"|"gif");
                    if is_image && !is_video {
                        if let Some((w, h)) = read_image_dims(&path) {
                            dims_cell.store((w as i64) << 32 | h as i64, Ordering::Relaxed);
                        } else { dims_cell.store(-1, Ordering::Relaxed); }
                        fps_cell.store(-1, Ordering::Relaxed);
                        duration_cell.store(-1, Ordering::Relaxed);
                    } else if is_video {
                        if let Some((w, h, fps, dur)) = read_video_info(&path) {
                            dims_cell.store((w as i64) << 32 | h as i64, Ordering::Relaxed);
                            fps_cell.store(fps, Ordering::Relaxed);
                            duration_cell.store(dur, Ordering::Relaxed);
                        } else {
                            dims_cell.store(-1, Ordering::Relaxed);
                            fps_cell.store(-1, Ordering::Relaxed);
                            duration_cell.store(-1, Ordering::Relaxed);
                        }
                    } else {
                        dims_cell.store(-1, Ordering::Relaxed);
                        fps_cell.store(-1, Ordering::Relaxed);
                        duration_cell.store(-1, Ordering::Relaxed);
                    }
                });
                needs_redraw = true;
            }
        } else {
            // While active, clear stale preview so it recomputes on next idle
            app.preview_path = None;
            app.preview_size = None;
            app.preview_modified = None;
            app.preview_created = None;
            app.preview_count = None;
            app.preview_dims = None;
            app.preview_fps = None;
            app.preview_duration = None;
        }

        let flash_active = app.clipboard.as_ref()
            .is_some_and(|cb| cb.set_at.elapsed().as_millis() < CLIPBOARD_FLASH_MS as u128 + 50);
        let bg_active = app.bg_done_rx.is_some();
        let preview_pending = app.preview_size.as_ref()
            .map_or(false, |s| s.load(Ordering::Relaxed) == u64::MAX);
        if preview_pending && !bg_active {
            app.spinner_frame = app.spinner_frame.wrapping_add(1);
            needs_redraw = true;
        }
        let poll_ms: u64 = if flash_active || bg_active || preview_pending { 80 }
            else if inactive_ms < wait_to_load_preview { (wait_to_load_preview - inactive_ms).min(100) as u64 }
            else { 100 };
        if event::poll(Duration::from_millis(poll_ms))? {
            let ev = event::read()?;
            if matches!(ev, Event::FocusGained) { app.focused = true; needs_redraw = true; continue; }
            if matches!(ev, Event::FocusLost)   { app.focused = false; needs_redraw = true; continue; }
            if let Event::Key(key) = ev {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                needs_redraw = true;
                app.last_key_at = std::time::Instant::now();

                // Delete confirmation intercepts all keys
                if app.confirming_delete.is_some() {
                    match key.code {
                        KeyCode::Char('y') => {
                            app.confirming_delete = None;
                            let paths = std::mem::take(&mut app.pending_deletes);
                            if paths.len() == 1 && paths[0].is_file() {
                                let noop = Arc::new(AtomicUsize::new(0));
                                delete_recursive(&paths[0], &noop);
                                app.selection.clear(); app.selection_anchor = None; app.select_mode = false;
                                app.refresh();
                                let col = &mut app.columns[app.active_col];
                                if col.selected_row >= col.grouped.row_count && col.selected_row > 0 { col.selected_row -= 1; }
                                app.maybe_push_child_column();
                            } else {
                                app.is_deleting = true;
                                app.spinner_frame = 0;
                                let progress = Arc::new(AtomicUsize::new(0));
                                let total = Arc::new(AtomicUsize::new(0));
                                app.bg_progress = Some(progress.clone());
                                app.bg_total = Some(total.clone());
                                let (tx, rx) = std::sync::mpsc::channel();
                                app.bg_done_rx = Some(rx);
                                std::thread::spawn(move || {
                                    let t: usize = paths.iter().map(|p| count_files(p)).sum();
                                    total.store(t, Ordering::Relaxed);
                                    for path in &paths { delete_recursive(path, &progress); }
                                    let _ = tx.send(());
                                });
                            }
                        }
                        _ => { app.confirming_delete = None; app.pending_deletes.clear(); }
                    }
                    continue;
                }

                // Pane picker intercepts all keys
                if let Some((ref panes, ref mut sel)) = app.pane_picker {
                    let panes = panes.clone();
                    let count = panes.len();
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => { app.pane_picker = None; }
                        KeyCode::Char('j') | KeyCode::Down  => { *sel = (*sel + 1).min(count.saturating_sub(1)); }
                        KeyCode::Char('k') | KeyCode::Up    => { *sel = sel.saturating_sub(1); }
                        KeyCode::Enter => {
                            let chosen = panes[*sel].clone();
                            app.linked_pane = Some(chosen);
                            app.pane_picker = None;
                        }
                        KeyCode::Char('u') => {
                            app.linked_pane = None;
                            app.pane_picker = None;
                        }
                        _ => {}
                    }
                    continue;
                }

                // Favorites view intercepts all keys
                if app.favorites_view {
                    let favs: Vec<std::path::PathBuf> = {
                        let mut v: Vec<_> = app.favorites.iter().cloned().collect();
                        v.sort();
                        v
                    };
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('F') => {
                            app.favorites_view = false;
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            if !favs.is_empty() {
                                app.favorites_cursor = (app.favorites_cursor + 1).min(favs.len() - 1);
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            app.favorites_cursor = app.favorites_cursor.saturating_sub(1);
                        }
                        KeyCode::Enter => {
                            if let Some(path) = favs.get(app.favorites_cursor) {
                                let target = if path.is_dir() { path.clone() } else {
                                    path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| path.clone())
                                };
                                app.columns.truncate(1);
                                app.columns[0] = crate::column::Column::new(target.clone());
                                app.active_col = 0;
                                // select the specific entry if it's a file
                                if !path.is_dir() {
                                    if let Some(row) = app.columns[0].grouped.row_to_entry.iter().position(|&i| {
                                        app.columns[0].grouped.entries[i].path == *path
                                    }) {
                                        app.columns[0].selected_row = row;
                                        app.columns[0].sync_list_state();
                                    }
                                }
                                app.maybe_push_child_column();
                                app.favorites_view = false;
                            }
                        }
                        KeyCode::Char('f') => {
                            if let Some(path) = favs.get(app.favorites_cursor).cloned() {
                                app.favorites.remove(&path);
                                save_favorites(&app.favorites);
                                app.favorites_cursor = app.favorites_cursor.min(app.favorites.len().saturating_sub(1));
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Rename mode intercepts all keys
                if let Some(ref mut rs) = app.renaming {
                    match rs.mode {
                        RenameMode::Insert => match key.code {
                            KeyCode::Esc => rs.enter_normal(),
                            KeyCode::Enter => {
                                let col = &app.columns[app.active_col];
                                if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                                    let new_name = rs.text.clone();
                                    let old_path = e.path.clone();
                                    if !new_name.is_empty() && new_name != e.name {
                                        let new_path = old_path.parent().unwrap().join(&new_name);
                                        let _ = std::fs::rename(&old_path, &new_path);
                                    }
                                }
                                app.renaming = None;
                                app.refresh();
                            }
                            KeyCode::Backspace => rs.backspace(),
                            KeyCode::Left     => rs.move_left(),
                            KeyCode::Right    => rs.move_right(),
                            KeyCode::Char(c)  => rs.insert_char(c),
                            _ => {}
                        },
                        RenameMode::Visual => {
                            match key.code {
                                KeyCode::Esc | KeyCode::Char('v') => { rs.mode = rename::RenameMode::Normal; }
                                KeyCode::Char('h') | KeyCode::Left  => rs.move_left(),
                                KeyCode::Char('l') | KeyCode::Right => rs.move_right(),
                                KeyCode::Char('w') => rs.move_word_forward(),
                                KeyCode::Char('b') => rs.move_word_backward(),
                                KeyCode::Char('e') => rs.move_word_end(),
                                KeyCode::Char('0') => rs.move_line_start(),
                                KeyCode::Char('$') => rs.move_line_end(),
                                KeyCode::Char('d') | KeyCode::Char('x') => {
                                    rs.delete_visual_selection();
                                    rs.mode = rename::RenameMode::Normal;
                                }
                                KeyCode::Char('c') => {
                                    rs.delete_visual_selection();
                                    rs.enter_insert_before();
                                }
                                _ => {}
                            }
                        }
                        RenameMode::Normal => {
                            let confirm_rename = |rs: &RenameState, app: &mut App| {
                                let col = &app.columns[app.active_col];
                                if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                                    let new_name = rs.text.clone();
                                    let old_path = e.path.clone();
                                    if !new_name.is_empty() && new_name != e.name {
                                        let new_path = old_path.parent().unwrap().join(&new_name);
                                        let _ = std::fs::rename(&old_path, &new_path);
                                    }
                                }
                            };

                            // Consume pending multi-key sequences
                            let pending = rs.pending.clone();
                            match (pending.as_str(), key.code) {
                                // ── r<char>: replace ──────────────────────────
                                ("r", KeyCode::Char(c)) => { rs.replace_char(c); rs.pending.clear(); }
                                ("r", _) => { rs.pending.clear(); }

                                // ── d<motion> ─────────────────────────────────
                                ("d", KeyCode::Char('d')) => { rs.clear_text(); rs.pending.clear(); }
                                ("d", KeyCode::Char('w')) => { rs.delete_word_forward(); rs.pending.clear(); }
                                ("d", KeyCode::Char('e')) => { rs.delete_to_word_end(); rs.pending.clear(); }
                                ("d", KeyCode::Char('b')) => { rs.delete_to_word_start(); rs.pending.clear(); }
                                ("d", KeyCode::Char('0')) => { rs.delete_to_line_start(); rs.pending.clear(); }
                                ("d", KeyCode::Char('$')) => { rs.delete_to_line_end(); rs.pending.clear(); }
                                ("d", KeyCode::Char('i')) => { rs.pending = "di".into(); }
                                ("d", _) => { rs.pending.clear(); }

                                // ── di<object> ────────────────────────────────
                                ("di", KeyCode::Char('w')) => { rs.delete_inner_word(); rs.pending.clear(); }
                                ("di", _) => { rs.pending.clear(); }

                                // ── c<motion> ─────────────────────────────────
                                ("c", KeyCode::Char('c')) => { rs.clear_text(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("c", KeyCode::Char('w')) | ("c", KeyCode::Char('e')) => { rs.delete_to_word_end(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("c", KeyCode::Char('b')) => { rs.delete_to_word_start(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("c", KeyCode::Char('0')) => { rs.delete_to_line_start(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("c", KeyCode::Char('$')) => { rs.delete_to_line_end(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("c", KeyCode::Char('i')) => { rs.pending = "ci".into(); }
                                ("c", _) => { rs.pending.clear(); }

                                // ── ci<object> ────────────────────────────────
                                ("ci", KeyCode::Char('w')) => { rs.delete_inner_word(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("ci", _) => { rs.pending.clear(); }

                                // ── no pending: immediate commands ────────────
                                (_, KeyCode::Esc) => { rs.pending.clear(); app.renaming = None; }
                                (_, KeyCode::Enter) => {
                                    let rs_ref = app.renaming.as_ref().unwrap();
                                    let text = rs_ref.text.clone();
                                    let col = &app.columns[app.active_col];
                                    if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                                        if !text.is_empty() && text != e.name {
                                            let new_path = e.path.parent().unwrap().join(&text);
                                            let _ = std::fs::rename(&e.path, &new_path);
                                        }
                                    }
                                    app.renaming = None;
                                    app.refresh();
                                }
                                (_, KeyCode::Char('h')) | (_, KeyCode::Left)  => rs.move_left(),
                                (_, KeyCode::Char('l')) | (_, KeyCode::Right) => rs.move_right(),
                                (_, KeyCode::Char('w')) => rs.move_word_forward(),
                                (_, KeyCode::Char('b')) => rs.move_word_backward(),
                                (_, KeyCode::Char('e')) => rs.move_word_end(),
                                (_, KeyCode::Char('0')) => rs.move_line_start(),
                                (_, KeyCode::Char('$')) => rs.move_line_end(),
                                (_, KeyCode::Char('x')) => rs.delete_at_cursor(),
                                (_, KeyCode::Char('X')) => rs.backspace(),
                                (_, KeyCode::Char('s')) => { rs.delete_at_cursor(); rs.enter_insert_before(); }
                                (_, KeyCode::Char('D')) => rs.delete_to_line_end(),
                                (_, KeyCode::Char('C')) => { rs.delete_to_line_end(); rs.enter_insert_before(); }
                                (_, KeyCode::Char('S')) => { rs.clear_text(); rs.enter_insert_before(); }
                                (_, KeyCode::Char('i')) => rs.enter_insert_before(),
                                (_, KeyCode::Char('a')) => rs.enter_insert_after(),
                                (_, KeyCode::Char('I')) => rs.enter_insert_start(),
                                (_, KeyCode::Char('A')) => rs.enter_insert_end(),
                                (_, KeyCode::Char('v')) => rs.enter_visual(),
                                (_, KeyCode::Char('d')) => rs.pending = "d".into(),
                                (_, KeyCode::Char('c')) => rs.pending = "c".into(),
                                (_, KeyCode::Char('r')) => rs.pending = "r".into(),
                                _ => { rs.pending.clear(); }
                            }
                            // suppress unused warning
                            let _ = confirm_rename;
                        }
                    }
                    continue;
                }

                // Convert mode intercepts all keys
                if app.converting.is_some() {
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                        KeyCode::Esc | KeyCode::Char('q') => { app.converting = None; }
                        KeyCode::Left => {
                            if let Some(ref mut cs) = app.converting {
                                if cs.selected > 0 { cs.selected -= 1; }
                            }
                        }
                        KeyCode::Right => {
                            if let Some(ref mut cs) = app.converting {
                                let max = cs.formats.len().saturating_sub(1);
                                if cs.selected < max { cs.selected += 1; }
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(cs) = app.converting.take() {
                                let fmt = cs.formats[cs.selected];
                                let ext = cs.source.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                                let is_doc = matches!(ext.as_str(),
                                    "doc"|"docx"|"odt"|"rtf"|"txt"|"md"|"mdx"
                                    |"xls"|"xlsx"|"ods"|"csv"|"ppt"|"pptx"|"odp");
                                let is_image = matches!(ext.as_str(),
                                    "jpg"|"jpeg"|"png"|"webp"|"tiff"|"tif"|"heic"|"heif"|"bmp"|"gif");
                                let required_tool = if is_doc { "soffice" }
                                    else if is_image { "magick" }
                                    else { "ffmpeg" };
                                let tool_ok = std::process::Command::new(required_tool)
                                    .arg("--version").output().is_ok()
                                    || (is_image && std::process::Command::new("ffmpeg")
                                        .arg("-version").output().is_ok());
                                if !tool_ok {
                                    app.status_flash = Some((
                                        format!("Error: missing {}", required_tool),
                                        std::time::Instant::now(),
                                    ));
                                    continue;
                                }
                                let output = unique_output_path(&cs.source, fmt);
                                let source = cs.source.clone();
                                let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
                                app.bg_done_rx = Some(done_rx);
                                app.is_converting = true;
                                app.convert_output = Some(output.clone());
                                std::thread::spawn(move || {
                                    let ext = source.extension()
                                        .and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                                    let is_image = matches!(ext.as_str(),
                                        "jpg"|"jpeg"|"png"|"webp"|"tiff"|"tif"|"heic"|"heif"|"bmp"|"gif");
                                    let is_doc = matches!(ext.as_str(),
                                        "doc"|"docx"|"odt"|"rtf"|"txt"|"md"|"mdx"
                                        |"xls"|"xlsx"|"ods"|"csv"|"ppt"|"pptx"|"odp");
                                    if is_doc {
                                        // LibreOffice outputs stem.pdf into the dir; rename if needed
                                        let out_dir = output.parent().unwrap_or(Path::new("."));
                                        std::process::Command::new("soffice")
                                            .args(["--headless", "--convert-to", "pdf", "--outdir"])
                                            .arg(out_dir)
                                            .arg(&source)
                                            .stdin(std::process::Stdio::null())
                                            .stdout(std::process::Stdio::null())
                                            .stderr(std::process::Stdio::null())
                                            .output()
                                            .ok();
                                        // libreoffice creates stem.pdf next to source
                                        let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
                                        let lo_out = out_dir.join(format!("{}.pdf", stem));
                                        if lo_out.exists() && lo_out != output {
                                            std::fs::rename(&lo_out, &output).ok();
                                        }
                                    } else if is_image {
                                        let has_magick = std::process::Command::new("magick")
                                            .arg("--version").output().is_ok();
                                        if has_magick {
                                            std::process::Command::new("magick")
                                                .arg(&source).arg(&output).output().ok();
                                        } else {
                                            std::process::Command::new("ffmpeg")
                                                .args(["-y", "-i"]).arg(&source).arg(&output).output().ok();
                                        }
                                    } else {
                                        std::process::Command::new("ffmpeg")
                                            .args(["-y", "-i"]).arg(&source).arg(&output).output().ok();
                                    }
                                    let _ = done_tx.send(());
                                });
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Goto mode intercepts all keys
                if app.goto_query.is_some() {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => { app.goto_query = None; }
                        KeyCode::Backspace => {
                            if let Some(ref mut q) = app.goto_query { q.pop(); }
                        }
                        KeyCode::Char(c) => {
                            if let Some(ref mut q) = app.goto_query { q.push(c); }
                        }
                        _ => {}
                    }
                    // Live navigate as user types, segment by segment
                    if let Some(ref q) = app.goto_query.clone() {
                        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                        // Determine starting dir and segments
                        let (mut cur_dir, segments): (PathBuf, Vec<&str>) = if q.starts_with('/') {
                            (PathBuf::from("/"), q.splitn(2, '/').last().map(|s| s.split('/').collect()).unwrap_or_default())
                        } else if q.starts_with("~/") || q == "~" {
                            (PathBuf::from(&home), q[2..].split('/').collect())
                        } else {
                            let base = app.goto_base_dir.clone().unwrap_or_else(|| app.columns[0].path.clone());
                            (base, q.split('/').collect())
                        };

                        // Navigate confirmed segments (all but last)
                        let last_seg = segments.last().copied().unwrap_or("");
                        let confirmed = if segments.len() > 1 { &segments[..segments.len()-1] } else { &[] };
                        for seg in confirmed {
                            if seg.is_empty() { continue; }
                            // find first entry in cur_dir that starts with seg
                            if let Ok(rd) = std::fs::read_dir(&cur_dir) {
                                let seg_l = seg.to_lowercase();
                                let mut matched: Vec<_> = rd.flatten()
                                    .filter(|e| e.file_name().to_string_lossy().to_lowercase().starts_with(&seg_l))
                                    .collect();
                                matched.sort_by_key(|e| e.file_name());
                                if let Some(entry) = matched.into_iter().find(|e| e.path().is_dir()) {
                                    cur_dir = entry.path();
                                }
                            }
                        }

                        // Navigate to cur_dir if changed
                        if app.columns[0].path != cur_dir {
                            app.columns.truncate(1);
                            app.columns[0] = crate::column::Column::new(cur_dir.clone());
                            app.active_col = 0;
                            app.maybe_push_child_column();
                        }

                        // Highlight matching entry for last segment
                        if !last_seg.is_empty() {
                            let seg_l = last_seg.to_lowercase();
                            let col = &mut app.columns[app.active_col];
                            if let Some(row) = (0..col.grouped.row_count).find(|&r| {
                                col.grouped.entry_at_row(r)
                                    .is_some_and(|e| e.name.to_lowercase().starts_with(&seg_l))
                            }) {
                                col.selected_row = row;
                                col.sync_list_state();
                                app.maybe_push_child_column();
                            }
                        }
                    }
                    needs_redraw = true;
                    continue;
                }

                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        if app.select_mode {
                            app.select_mode = false;
                            // keep selection so user can still X/C after exiting select mode
                        } else {
                            break;
                        }
                    }
                    KeyCode::Char('V') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        if app.select_mode {
                            app.select_mode = false;
                            app.selection.clear();
                            app.selection_anchor = None;
                        } else {
                            app.select_mode = true;
                        }
                    }
                    KeyCode::Char('.') if app.select_mode => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let row = col.selected_row;
                        if let Some(e) = col.grouped.entry_at_row(row) {
                            let path = e.path.clone();
                            if app.selection.contains(&path) {
                                app.selection.remove(&path);
                            } else {
                                app.selection.insert(path);
                                app.selection_anchor = Some(row);
                            }
                        }
                    }
                    KeyCode::Char(' ') | KeyCode::Char(',') if app.select_mode => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let current_row = col.selected_row;
                        let anchor = app.selection_anchor.unwrap_or(current_row);
                        let lo = anchor.min(current_row);
                        let hi = anchor.max(current_row);
                        let paths: Vec<_> = (lo..=hi)
                            .filter_map(|r| col.grouped.entry_at_row(r).map(|e| e.path.clone()))
                            .collect();
                        for path in paths {
                            app.selection.insert(path);
                        }
                        app.selection_anchor = Some(current_row);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.pending_g = false;
                        app.move_up();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.pending_g = false;
                        app.move_down();
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.pending_g = false;
                        app.columns[app.active_col].move_by(PAGE_JUMP as isize);
                        app.maybe_push_child_column();
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.pending_g = false;
                        app.columns[app.active_col].move_by(-(PAGE_JUMP as isize));
                        app.maybe_push_child_column();
                    }
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('=') if !key.modifiers.contains(KeyModifiers::ALT) => {
                        app.pending_g = false;
                        app.move_right();
                    }
                    KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') if !key.modifiers.contains(KeyModifiers::ALT) => {
                        app.pending_g = false;
                        app.move_left();
                    }
                    KeyCode::Char('=') if key.modifiers.contains(KeyModifiers::ALT) => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let vh = app.col_viewport_height;
                        app.columns[app.active_col].scroll_by(5, vh);
                    }
                    KeyCode::Char('-') if key.modifiers.contains(KeyModifiers::ALT) => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let vh = app.col_viewport_height;
                        app.columns[app.active_col].scroll_by(-5, vh);
                    }
                    KeyCode::Char('/') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        app.goto_base_dir = Some(app.columns[app.active_col].path.clone());
                        app.goto_query = Some(String::new());
                    }
                    KeyCode::Char('n') => {
                        app.pending_g = false;
                        app.columns.drain(0..app.active_col);
                        app.active_col = 0;
                    }
                    KeyCode::Char('G') => {
                        app.pending_g = false;
                        let col = &mut app.columns[app.active_col];
                        if col.grouped.row_count > 0 {
                            col.selected_row = col.grouped.row_count - 1;
                            col.sync_list_state();
                        }
                        app.maybe_push_child_column();
                    }
                    KeyCode::Char('g') => {
                        app.pending_prefix = None;
                        if app.pending_g {
                            app.pending_g = false;
                            let col = &mut app.columns[app.active_col];
                            col.selected_row = 0;
                            col.sync_list_state();
                            app.maybe_push_child_column();
                        } else {
                            app.pending_g = true;
                        }
                    }
                    KeyCode::Char(c @ '0'..='9') => {
                        app.pending_g = false;
                        let d = c as usize - '0' as usize;
                        let n = app.pending_prefix.unwrap_or(0) * 10 + d;
                        app.pending_digits += 1;
                        app.pending_prefix = Some(n);
                        let col = &mut app.columns[app.active_col];
                        let lw = crate::grouped::label_width(col.grouped.row_count);
                        if app.pending_digits >= lw {
                            app.pending_prefix = None;
                            app.pending_digits = 0;
                            let row = n.saturating_sub(1);
                            if row < col.grouped.row_count {
                                col.selected_row = row;
                                col.sync_list_state();
                            }
                            app.maybe_push_child_column();
                        }
                    }
                    KeyCode::Enter => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            let (path, is_dir) = (e.path.clone(), e.is_dir);
                            if is_dir {
                                app.cd_target = Some(path);
                                break;
                            } else {
                                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                                if default_app_exts.contains(&ext) {
                                    open_in_default_app(&path);
                                } else if let Some(ref pane) = app.linked_pane.clone() {
                                    open_in_linked_pane(&pane.id, &path);
                                } else {
                                    open_in_nvim(&path)?;
                                    terminal.clear()?;
                                }
                            }
                        }
                    }
                    KeyCode::Char('P') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let panes = list_panes();
                        if !panes.is_empty() {
                            let sel = app.linked_pane.as_ref()
                                .and_then(|lp| panes.iter().position(|p| p.id == lp.id))
                                .unwrap_or(0);
                            app.pane_picker = Some((panes, sel));
                        }
                    }
                    KeyCode::Char('R') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            app.renaming = Some(RenameState::new(&e.name));
                        }
                    }
                    KeyCode::Char('m') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let paths: Vec<PathBuf> = if !app.selection.is_empty() {
                            let mut v: Vec<PathBuf> = app.selection.iter().cloned().collect();
                            v.sort();
                            v
                        } else if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            vec![e.path.clone()]
                        } else { vec![] };
                        if !paths.is_empty() {
                            let primary = paths[0].clone();
                            app.clipboard = Some(ClipboardEntry { op: ClipboardOp::Cut, path: primary, paths, set_at: std::time::Instant::now() });
                            app.selection.clear(); app.selection_anchor = None; app.select_mode = false;
                        }
                    }
                    KeyCode::Char('y') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let paths: Vec<PathBuf> = if !app.selection.is_empty() {
                            let mut v: Vec<PathBuf> = app.selection.iter().cloned().collect();
                            v.sort();
                            v
                        } else if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            vec![e.path.clone()]
                        } else { vec![] };
                        if !paths.is_empty() {
                            let primary = paths[0].clone();
                            app.clipboard = Some(ClipboardEntry { op: ClipboardOp::Copy, path: primary, paths, set_at: std::time::Instant::now() });
                            app.selection.clear(); app.selection_anchor = None; app.select_mode = false;
                        }
                    }
                    KeyCode::Char('p') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        if let Some(ref cb) = app.clipboard.clone() {
                            let dest_dir = app.columns[app.active_col].path.clone();
                            let is_cut = cb.op == ClipboardOp::Cut;
                            if cb.paths.len() == 1 && cb.paths[0].is_file() {
                                let src = &cb.paths[0];
                                if let Some(filename) = src.file_name() {
                                    let noop = Arc::new(AtomicUsize::new(0));
                                    let single = ClipboardEntry { op: cb.op.clone(), path: src.clone(), paths: vec![src.clone()], set_at: cb.set_at };
                                    let dst = unique_dest(&dest_dir, filename, src, is_cut);
                                    do_paste(&single, &dst, &noop).ok();
                                }
                                if is_cut { app.clipboard = None; }
                                app.selection.clear(); app.selection_anchor = None; app.select_mode = false;
                                app.refresh();
                                app.maybe_push_child_column();
                            } else {
                                app.is_pasting = true;
                                app.spinner_frame = 0;
                                let progress = Arc::new(AtomicUsize::new(0));
                                let total = Arc::new(AtomicUsize::new(0));
                                app.bg_progress = Some(progress.clone());
                                app.bg_total = Some(total.clone());
                                let (tx, rx) = std::sync::mpsc::channel();
                                app.bg_done_rx = Some(rx);
                                let cb_clone = cb.clone();
                                std::thread::spawn(move || {
                                    let t: usize = cb_clone.paths.iter().map(|p| count_files(p)).sum();
                                    total.store(t, Ordering::Relaxed);
                                    for src in &cb_clone.paths {
                                        if let Some(filename) = src.file_name() {
                                            let single = ClipboardEntry { op: cb_clone.op.clone(), path: src.clone(), paths: vec![src.clone()], set_at: cb_clone.set_at };
                                            let dst = unique_dest(&dest_dir, filename, src, is_cut);
                                            do_paste(&single, &dst, &progress).ok();
                                        }
                                    }
                                    let _ = tx.send(());
                                });
                                if is_cut { app.clipboard = None; }
                            }
                        }
                    }

                    KeyCode::Char('r') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            std::process::Command::new("open").arg("-R").arg(&e.path).spawn().ok();
                        }
                    }
                    KeyCode::Char('c') if !app.select_mode => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            if !e.is_dir {
                                let formats = convert_formats_for(&e.path);
                                if !formats.is_empty() {
                                    app.converting = Some(ConvertState {
                                        source: e.path.clone(),
                                        formats,
                                        selected: 0,
                                    });
                                }
                            }
                        }
                    }
                    KeyCode::Char('x') => {
                        use app::PreviewMode;
                        app.preview_mode = match app.preview_mode {
                            PreviewMode::Short => PreviewMode::Long,
                            PreviewMode::Long  => PreviewMode::Name,
                            PreviewMode::Name  => PreviewMode::Short,
                        };
                    }
                    KeyCode::Tab => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let paths: Vec<_> = if !app.selection.is_empty() {
                            app.selection.iter().cloned().collect()
                        } else if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            vec![e.path.clone()]
                        } else { vec![] };
                        for path in paths {
                            std::process::Command::new("shelf-add")
                                .arg(&path)
                                .spawn().ok();
                        }
                    }
                    KeyCode::Char('f') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            if app.favorites.contains(&e.path) {
                                app.favorites.remove(&e.path);
                            } else {
                                app.favorites.insert(e.path.clone());
                            }
                            save_favorites(&app.favorites);
                        }
                    }
                    KeyCode::Char('F') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        app.favorites_view = !app.favorites_view;
                        app.favorites_cursor = 0;
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            let path = e.path.to_string_lossy().into_owned();
                            let mut child = std::process::Command::new("pbcopy")
                                .stdin(std::process::Stdio::piped())
                                .spawn()
                                .ok();
                            if let Some(ref mut c) = child {
                                if let Some(stdin) = c.stdin.as_mut() {
                                    use std::io::Write;
                                    let _ = stdin.write_all(path.as_bytes());
                                }
                            }
                        }
                    }
                    KeyCode::Char('K') if key.modifiers.contains(KeyModifiers::ALT) => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            let dst = copy_dest(&e.path);
                            if e.is_dir { copy_dir(&e.path, &dst, &Arc::new(AtomicUsize::new(0))).ok(); }
                            else { std::fs::copy(&e.path, &dst).ok(); }
                            let dst_name = dst.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                            app.refresh();
                            let col = &mut app.columns[app.active_col];
                            if let Some(row) = col.grouped.row_to_entry.iter().position(|&i| {
                                col.grouped.entries[i].name == dst_name
                            }) {
                                col.selected_row = row;
                                col.sync_list_state();
                            }
                        }
                    }
                    KeyCode::Char('D') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        if !app.selection.is_empty() {
                            let mut paths: Vec<PathBuf> = app.selection.iter().cloned().collect();
                            paths.sort();
                            let count = paths.len();
                            app.pending_deletes = paths;
                            app.confirming_delete = Some(PathBuf::from(format!("{} items", count)));
                        } else {
                            let col = &app.columns[app.active_col];
                            if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                                app.pending_deletes = vec![e.path.clone()];
                                app.confirming_delete = Some(e.path.clone());
                            }
                        }
                    }
                    KeyCode::Char('d') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let base_path = col.path.clone();
                        let placeholder = (0u32..).map(|i| {
                            if i == 0 { "untitled".to_string() } else { format!("untitled {}", i) }
                        }).find(|name| !base_path.join(name).exists()).unwrap();
                        let new_dir = base_path.join(&placeholder);
                        if std::fs::create_dir(&new_dir).is_ok() {
                            app.refresh();
                            // Select the new dir in the target column and enter rename mode
                            let col = &mut app.columns[app.active_col];
                            if let Some(row) = col.grouped.row_to_entry.iter().position(|&i| {
                                col.grouped.entries[i].name == placeholder
                            }) {
                                col.selected_row = row;
                                col.sync_list_state();
                            }
                            app.renaming = Some(RenameState {
                                text: String::new(),
                                cursor: 0,
                                mode: RenameMode::Insert,
                                pending: String::new(),
                                visual_anchor: 0,
                            });
                            app.maybe_push_child_column();
                        }
                    }
                    KeyCode::Char('%') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let base_path = col.path.clone();
                        let placeholder = (0u32..).map(|i| {
                            if i == 0 { "untitled".to_string() } else { format!("untitled {}", i) }
                        }).find(|name| !base_path.join(name).exists()).unwrap();
                        let new_file = base_path.join(&placeholder);
                        if std::fs::File::create(&new_file).is_ok() {
                            app.refresh();
                            let col = &mut app.columns[app.active_col];
                            if let Some(row) = col.grouped.row_to_entry.iter().position(|&i| {
                                col.grouped.entries[i].name == placeholder
                            }) {
                                col.selected_row = row;
                                col.sync_list_state();
                            }
                            app.renaming = Some(RenameState {
                                text: String::new(),
                                cursor: 0,
                                mode: RenameMode::Insert,
                                pending: String::new(),
                                visual_anchor: 0,
                            });
                            app.maybe_push_child_column();
                        }
                    }
                    _ => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        app.pending_digits = 0;
                    }
                }
            }
        } else {
            if flash_active {
                needs_redraw = true;
            }
            if last_refresh.elapsed().as_millis() >= IDLE_REFRESH_MS as u128 {
                app.refresh();
                last_refresh = std::time::Instant::now();
                needs_redraw = true;
            }
        }
    }

    if let Some(ref mut p) = shelf_proc { p.kill().ok(); }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture, DisableFocusChange)?;
    terminal.show_cursor()?;
    if let Some(path) = app.cd_target {
        println!("{}", path.display());
    }
    Ok(())
}

//! 文件系统工具：驱动器枚举、异步目录读取、大小/日期格式化、复制移动、回收站删除、系统打开。
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------- 条目模型

#[derive(Clone)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64, // unix 秒
    pub hidden: bool,
    pub ext: String, // 小写扩展名；目录为 ""
}

pub type LsResult = Result<(PathBuf, Vec<FsEntry>), (PathBuf, String)>;

fn read_dir_bg(path: PathBuf, tx: std::sync::mpsc::Sender<LsResult>) {
    let mut entries: Vec<FsEntry> = Vec::new();
    let res = fs::read_dir(&path).map(|rd| {
        for it in rd.flatten() {
            let p = it.path();
            let md = match it.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let name = it.file_name().to_string_lossy().to_string();
            #[cfg(windows)]
            let hidden = {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
                const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
                (md.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)) != 0
            };
            #[cfg(not(windows))]
            let hidden = name.starts_with('.');
            let mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let ext = if md.is_dir() {
                String::new()
            } else {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase())
                    .unwrap_or_default()
            };
            entries.push(FsEntry {
                name,
                path: p,
                is_dir: md.is_dir(),
                size: if md.is_dir() { 0 } else { md.len() },
                mtime,
                hidden,
                ext,
            });
        }
    });
    match res {
        Ok(()) => {
            entries.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            let _ = tx.send(Ok((path, entries)));
        }
        Err(e) => {
            let _ = tx.send(Err((path, e.to_string())));
        }
    }
}

/// 后台线程读目录，不阻塞 UI
pub fn spawn_ls(path: PathBuf) -> Receiver<LsResult> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || read_dir_bg(path, tx));
    rx
}

// ---------------------------------------------------------------- 驱动器

#[cfg(windows)]
pub fn drives() -> Vec<(String, PathBuf)> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW};

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    let mut out = Vec::new();
    let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{}:\\", letter);
        let wroot = wide(&root);
        // GetDriveTypeW: 0=Unknown 1=NoRootDir 2=Removable 3=Fixed 4=Remote 5=CDRom 6=Ram
        let dt = unsafe { GetDriveTypeW(PCWSTR(wroot.as_ptr())) };
        let kind = match dt {
            2 => "可移动",
            4 => "网络",
            5 => "光驱",
            _ => "",
        };
        let mut vol = [0u16; 128];
        let label = unsafe {
            GetVolumeInformationW(
                PCWSTR(wroot.as_ptr()),
                Some(&mut vol),
                None,
                None,
                None,
                None,
            )
        };
        let name = if label.is_ok() {
            String::from_utf16_lossy(&vol)
                .trim_end_matches('\0')
                .to_string()
        } else {
            String::new()
        };
        let disp = if name.is_empty() {
            if kind.is_empty() {
                format!("{}:", letter)
            } else {
                format!("{}: ({})", letter, kind)
            }
        } else {
            format!("{} ({}:)", name, letter)
        };
        out.push((disp, PathBuf::from(root)));
    }
    out
}

#[cfg(not(windows))]
pub fn drives() -> Vec<(String, PathBuf)> {
    vec![("/".into(), PathBuf::from("/"))]
}

#[cfg(windows)]
pub fn drive_free(path: &Path) -> Option<u64> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let root = path.ancestors().last()?;
    let ws: Vec<u16> = OsStr::new(root)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(PCWSTR(ws.as_ptr()), Some(&mut free), None, None) };
    if ok.is_ok() {
        Some(free)
    } else {
        None
    }
}

#[cfg(not(windows))]
pub fn drive_free(_path: &Path) -> Option<u64> {
    None
}

// ---------------------------------------------------------------- 格式化

pub fn fmt_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{} {}", n, UNITS[u])
    } else {
        format!("{:.1} {}", v, UNITS[u])
    }
}

/// 北京时间（无夏令时），固定 +8
pub fn fmt_time(secs: i64) -> String {
    if secs == 0 {
        return "-".into();
    }
    let t = secs + 8 * 3600;
    let days = t.div_euclid(86400);
    let rem = t.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Howard Hinnant civil_from_days
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------- 操作

pub fn home_dir() -> PathBuf {
    std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(PathBuf::from))
        .unwrap_or_else(|_| PathBuf::from("/"))
}

pub fn short_name(p: &Path) -> String {
    if p.parent().map(|x| x.as_os_str().is_empty()).unwrap_or(false) || p == Path::new("/") {
        return p.to_string_lossy().to_string();
    }
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string_lossy().to_string())
}

/// 删除到回收站
#[cfg(windows)]
pub fn delete_to_recycle(paths: &[PathBuf]) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
        SHFILEOPSTRUCTW,
    };

    let mut buf: Vec<u16> = Vec::new();
    for p in paths {
        buf.extend(OsStr::new(p).encode_wide());
        buf.push(0);
    }
    buf.push(0); // 双 null 结尾

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let mut op = SHFILEOPSTRUCTW {
            hwnd: Default::default(),
            wFunc: FO_DELETE,
            pFrom: PCWSTR(buf.as_ptr()),
            pTo: PCWSTR::null(),
            fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
            fAnyOperationsAborted: Default::default(),
            hNameMappings: std::ptr::null_mut(),
            lpszProgressTitle: PCWSTR::null(),
        };
        let code = SHFileOperationW(&mut op);
        if code == 0 {
            Ok(())
        } else {
            Err(format!("SHFileOperationW 返回 {}", code))
        }
    }
}

#[cfg(not(windows))]
pub fn delete_to_recycle(paths: &[PathBuf]) -> Result<(), String> {
    for p in paths {
        let _ = if p.is_dir() {
            fs::remove_dir_all(p)
        } else {
            fs::remove_file(p)
        };
    }
    Ok(())
}

/// 用系统默认程序打开
#[cfg(windows)]
pub fn open_with_default(path: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &path.to_string_lossy()])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    Ok(())
}

#[cfg(not(windows))]
pub fn open_with_default(path: &Path) -> io::Result<()> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()?;
    Ok(())
}

#[cfg(windows)]
pub fn reveal_in_explorer(path: &Path) -> io::Result<()> {
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.to_string_lossy()))
        .spawn()?;
    Ok(())
}

#[cfg(not(windows))]
pub fn reveal_in_explorer(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub fn unique_child(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mk = |i: u32| -> PathBuf {
        let name = if i == 0 {
            if ext.is_empty() {
                stem.to_string()
            } else {
                format!("{}.{}", stem, ext)
            }
        } else if ext.is_empty() {
            format!("{} ({})", stem, i)
        } else {
            format!("{} ({}).{}", stem, i, ext)
        };
        dir.join(name)
    };
    let mut i = 0u32;
    while mk(i).exists() {
        i += 1;
    }
    mk(i)
}

// ---------------------------------------------------------------- 复制 / 移动任务

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CopyOp {
    Copy,
    Move,
}

pub struct JobProgress {
    pub text: String,
    pub done: bool,
    pub result: Option<Result<u64, String>>,
}

#[derive(Clone)]
pub struct JobHandle {
    pub progress: Arc<Mutex<JobProgress>>,
}

fn copy_file(src: &Path, dst: &Path, replace: bool) -> io::Result<u64> {
    if dst.exists() {
        if !replace {
            return Ok(0);
        }
        fs::remove_file(dst)?;
    }
    fs::copy(src, dst)?;
    Ok(fs::metadata(dst).map(|m| m.len()).unwrap_or(0))
}

fn copy_tree(src: &Path, dst: &Path, replace: bool, prog: &Mutex<JobProgress>) -> io::Result<u64> {
    if src.is_dir() {
        if !dst.exists() {
            fs::create_dir_all(dst)?;
        }
        let mut total = 0u64;
        for it in fs::read_dir(src)? {
            let it = it?;
            let p = it.path();
            let target = dst.join(it.file_name());
            // 防止把目标复制进自身
            if p == dst {
                continue;
            }
            {
                let mut g = prog.lock().unwrap();
                g.text = format!("正在复制: {}", it.file_name().to_string_lossy());
            }
            total += copy_tree(&p, &target, replace, prog)?;
        }
        Ok(total)
    } else {
        copy_file(src, dst, replace)
    }
}

fn remove_tree(p: &Path) -> io::Result<()> {
    if p.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

/// 启动后台复制/移动任务。replace=false 时跳过已存在的目标文件。
pub fn start_job(
    op: CopyOp,
    srcs: Vec<PathBuf>,
    dst_dir: PathBuf,
    replace: bool,
) -> JobHandle {
    let progress = Arc::new(Mutex::new(JobProgress {
        text: String::new(),
        done: false,
        result: None,
    }));
    let prog2 = progress.clone();
    thread::spawn(move || {
        let mut total = 0u64;
        let mut err: Option<String> = None;
        for (i, src) in srcs.iter().enumerate() {
            {
                let mut g = prog2.lock().unwrap();
                g.text = format!(
                    "{} [{}/{}]: {}",
                    if op == CopyOp::Copy { "复制" } else { "移动" },
                    i + 1,
                    srcs.len(),
                    short_name(src)
                );
            }
            let dst = dst_dir.join(short_name(src));
            if dst == *src {
                continue; // 原地粘贴
            }
            // 移动：跨盘时先复制后删除；同盘直接改名
            if op == CopyOp::Move {
                if let Ok(md) = fs::metadata(&dst) {
                    if md.is_file() && !replace {
                        continue;
                    }
                }
                match fs::rename(src, &dst) {
                    Ok(()) => {
                        total += fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
                        continue;
                    }
                    Err(_) => {
                        // 同盘目录改名到已存在目录会失败，走复制合并
                    }
                }
            }
            let r = copy_tree(src, &dst, replace, &prog2);
            match r {
                Ok(n) => {
                    total += n;
                    if op == CopyOp::Move {
                        if let Err(e) = remove_tree(src) {
                            err = Some(format!("移动后删除源失败 {}: {}", src.display(), e));
                            break;
                        }
                    }
                }
                Err(e) => {
                    err = Some(format!("{}: {}", src.display(), e));
                    break;
                }
            }
        }
        let mut g = prog2.lock().unwrap();
        g.done = true;
        g.result = Some(match err {
            None => Ok(total),
            Some(e) => Err(e),
        });
    });
    JobHandle { progress }
}

pub fn same_volume(a: &Path, b: &Path) -> bool {
    fn root_of(p: &Path) -> Option<String> {
        let s = p.to_string_lossy().to_uppercase();
        let bytes: Vec<char> = s.chars().collect();
        if bytes.len() >= 2 && bytes[1] == ':' {
            Some(bytes[0..2].iter().collect())
        } else {
            p.ancestors().last().map(|x| x.to_string_lossy().to_string())
        }
    }
    match (root_of(a), root_of(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

pub fn now() -> SystemTime {
    SystemTime::now()
}

pub fn dur_100ms() -> Duration {
    Duration::from_millis(100)
}

// ================================================================ 系统剪贴板（CF_HDROP）
//
// 目标：和 Explorer / 微信 / QQ / 桌面 双向互通。
// 写：DROPFILES + UTF-16 路径表 塞进 CF_HDROP；再写 Preferred DropEffect 标记复制(1)/剪切(2)。
// 读：反过来，DragQueryFileW 逐个取路径，读 DropEffect 判断是不是剪切。

#[cfg(windows)]
pub fn clip_write_files(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::w;
    use windows::Win32::Foundation::{GlobalFree, HANDLE, POINT};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::DROPFILES;

    if paths.is_empty() {
        return Err("没有要复制的项目".into());
    }

    // DROPFILES 头 + 双 \0 结尾的 UTF-16 路径表
    let mut list: Vec<u16> = Vec::new();
    for p in paths {
        list.extend(std::ffi::OsStr::new(p).encode_wide());
        list.push(0);
    }
    list.push(0);
    let hdr = std::mem::size_of::<DROPFILES>();
    let total = hdr + list.len() * 2;

    unsafe {
        let hmem =
            GlobalAlloc(GMEM_MOVEABLE, total).map_err(|e| format!("GlobalAlloc 失败：{e}"))?;
        let ptr = GlobalLock(hmem) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(hmem);
            return Err("GlobalLock 失败".into());
        }
        let df = DROPFILES {
            pFiles: hdr as u32,
            pt: POINT { x: 0, y: 0 },
            fNC: false.into(),
            fWide: true.into(),
        };
        std::ptr::copy_nonoverlapping(&df as *const _ as *const u8, ptr, hdr);
        std::ptr::copy_nonoverlapping(list.as_ptr() as *const u8, ptr.add(hdr), list.len() * 2);
        // 解锁到 0 时 GlobalUnlock 返回 FALSE，这是正常行为，不能当错误
        let _ = GlobalUnlock(hmem);

        // 别在这里 return：hmem 已经分配了，直接返回会泄漏
        if let Err(e) = OpenClipboard(None) {
            let _ = GlobalFree(hmem);
            return Err(format!("打开剪贴板失败：{e}"));
        }
        EmptyClipboard().ok();
        let mut ok = SetClipboardData(CF_HDROP.0 as u32, HANDLE(hmem.0)).is_ok();

        // Preferred DropEffect：让"剪切"在 Explorer 里真的走移动
        if ok {
            if let Ok(h2) = GlobalAlloc(GMEM_MOVEABLE, 4) {
                let p2 = GlobalLock(h2) as *mut u32;
                if !p2.is_null() {
                    *p2 = if cut { 2 } else { 1 }; // DROPEFFECT_MOVE / COPY
                    let _ = GlobalUnlock(h2);
                    let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
                    if fmt == 0 || SetClipboardData(fmt, HANDLE(h2.0)).is_err() {
                        let _ = GlobalFree(h2);
                    }
                } else {
                    let _ = GlobalFree(h2);
                }
            }
        }
        let _ = CloseClipboard();

        if ok {
            Ok(())
        } else {
            let _ = GlobalFree(hmem);
            Err("写入剪贴板失败".into())
        }
    }
}

/// 读系统剪贴板里的文件列表；返回 (路径, 是否剪切)。
#[cfg(windows)]
pub fn clip_read_files() -> Option<(Vec<PathBuf>, bool)> {
    use windows::core::w;
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
        RegisterClipboardFormatW,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_err() {
            return None;
        }
        if OpenClipboard(None).is_err() {
            return None;
        }
        let mut out: Vec<PathBuf> = Vec::new();
        let mut cut = false;
        if let Ok(h) = GetClipboardData(CF_HDROP.0 as u32) {
            let hdrop = HDROP(h.0);
            let n = DragQueryFileW(hdrop, u32::MAX, None); // 问条目数
            for i in 0..n {
                let need = DragQueryFileW(hdrop, i, None) as usize;
                if need == 0 {
                    continue;
                }
                let mut buf = vec![0u16; need + 1];
                let got = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
                if got > 0 {
                    buf.truncate(got);
                    out.push(PathBuf::from(String::from_utf16_lossy(&buf)));
                }
            }
            let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            if fmt != 0 {
                if let Ok(h2) = GetClipboardData(fmt) {
                    let p = GlobalLock(HGLOBAL(h2.0)) as *const u32;
                    if !p.is_null() {
                        cut = (*p & 2) != 0; // DROPEFFECT_MOVE
                        let _ = GlobalUnlock(HGLOBAL(h2.0));
                    }
                }
            }
        }
        let _ = CloseClipboard();
        if out.is_empty() {
            None
        } else {
            Some((out, cut))
        }
    }
}

#[cfg(not(windows))]
pub fn clip_write_files(_paths: &[PathBuf], _cut: bool) -> Result<(), String> {
    Err("仅 Windows 支持".into())
}

/// 清空系统剪贴板。剪切 + 粘贴（=移动）成功后要调，
/// 否则再按一次 Ctrl+V 会去移动已经不在原位的源文件。
#[cfg(windows)]
pub fn clip_clear() {
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard};
    unsafe {
        if OpenClipboard(None).is_ok() {
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
        }
    }
}

#[cfg(not(windows))]
pub fn clip_clear() {}

/// 剪贴板里是否有文件（CF_HDROP）。只做格式检查，不打开剪贴板，开销极小。
#[cfg(windows)]
pub fn clip_has_files() -> bool {
    use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
    use windows::Win32::System::Ole::CF_HDROP;
    unsafe { IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() }
}

#[cfg(not(windows))]
pub fn clip_has_files() -> bool {
    false
}

#[cfg(not(windows))]
pub fn clip_read_files() -> Option<(Vec<PathBuf>, bool)> {
    None
}

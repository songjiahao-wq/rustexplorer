#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! RustExplorer — Files 式多窗口文件管理器
//! 结构：单进程(共享剪贴板/任务/toast) → 原生多窗口 → 窗口内标签页。
//! 标签可拖出窗口撕成新原生窗口；无宫格，一窗一内容区。

mod fs_util;

use eframe::egui::{self, Align, Align2, Color32, FontDefinitions, FontData, FontFamily, FontId,
    Frame, Id, Key, Layout, Margin, Order, Pos2, Rect, ResizeDirection, Response, RichText,
    ScrollArea, Sense, StrokeKind, TextEdit, TextStyle, TopBottomPanel, Ui, Vec2, ViewportBuilder,
    ViewportClass, ViewportCommand, ViewportId, Visuals};
use fs_util::{
    clip_clear, clip_read_files, clip_write_files, delete_to_recycle, drive_free, drives, fmt_size, fmt_time,
    home_dir, open_with_default, reveal_in_explorer, same_volume, short_name, spawn_ls, start_job,
    unique_child, CopyOp, FsEntry, JobHandle, LsResult,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ================================================================ 设计 tokens
// 浅色应用主题（Files 亮色） · 边框一律黑 8% · Win11 蓝 accent · 4px 网格

const BG: Color32 = Color32::from_rgb(249, 249, 249); // 窗口底/内容区
const BG2: Color32 = Color32::from_rgb(243, 243, 243); // 工具条/激活标签底
const SURF: Color32 = Color32::from_rgb(255, 255, 255); // 浮层
const LINE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 20); // 黑8%
const LINE2: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 34);
const FG: Color32 = Color32::from_rgb(28, 29, 31);
const FG2: Color32 = Color32::from_rgb(88, 90, 94);
const MUTE: Color32 = Color32::from_rgb(140, 142, 147);
const ACC: Color32 = Color32::from_rgb(0, 102, 214); // Win11 蓝（浅底加深）
// accent(0,102,214) @ 11% → 预乘后 (0*28, 102*28, 214*28)/255 ≈ (0,11,24,28)
// 之前直接写 (0,102,214,28) 当预乘值用 → 通道大于 alpha，混合出来是青色
const ACC_BG: Color32 = Color32::from_rgba_premultiplied(0, 11, 24, 28); // accent 11%
const HOVER: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 15); // 黑6%
const HOVER2: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 28); // 黑11%（标题钮hover）
const PR: Color32 = Color32::from_rgb(0, 86, 184); // accent 按下

const AMBER: Color32 = Color32::from_rgb(239, 159, 39);
const GREEN: Color32 = Color32::from_rgb(29, 158, 117);
const PINK: Color32 = Color32::from_rgb(212, 83, 126);
const PURPLE: Color32 = Color32::from_rgb(127, 119, 221);
const TEAL: Color32 = Color32::from_rgb(45, 156, 219);
const GRAY: Color32 = Color32::from_rgb(136, 135, 128);
const RED: Color32 = Color32::from_rgb(226, 75, 74);

const TAB_H: f32 = 32.0;
const TB_H: f32 = 44.0;
const ROW_H: f32 = 28.0;
const HEAD_H: f32 = 26.0;
const SB_H: f32 = 28.0;
const SIDE_W: f32 = 200.0;

fn apply_visuals(ctx: &egui::Context) {
    let mut v = Visuals::light();
    v.window_corner_radius = 8.0.into();
    v.window_stroke = (0.5, LINE2).into();
    v.window_fill = SURF;
    v.panel_fill = BG;
    v.extreme_bg_color = BG;
    v.faint_bg_color = BG2;
    v.override_text_color = Some(FG);
    v.hyperlink_color = ACC;
    {
        let ws = [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ];
        for w in ws {
            w.bg_fill = BG2;
            w.weak_bg_fill = Color32::TRANSPARENT;
            w.bg_stroke = (0.5, LINE).into();
            w.fg_stroke = (1.0, FG).into();
        }
    }
    v.widgets.noninteractive.bg_fill = BG;
    v.widgets.noninteractive.fg_stroke = (1.0, FG2).into();
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.active.weak_bg_fill = ACC_BG;
    v.selection.bg_fill = ACC_BG;
    v.selection.stroke = (1.0, ACC).into();
    ctx.set_visuals(v);

    let mut st = (*ctx.style()).clone();
    st.spacing.item_spacing = Vec2::new(4.0, 4.0);
    st.spacing.window_margin = Margin::same(8);
    st.spacing.button_padding = Vec2::new(8.0, 4.0);
    st.spacing.scroll.bar_width = 8.0;
    st.spacing.scroll.floating = true;
    st.text_styles.insert(TextStyle::Small, FontId::proportional(11.0));
    st.text_styles.insert(TextStyle::Body, FontId::proportional(12.5));
    st.text_styles.insert(TextStyle::Button, FontId::proportional(12.5));
    st.text_styles.insert(TextStyle::Heading, FontId::proportional(15.0));
    ctx.set_style(st);
}

/// 强 ease-out（近似 cubic-bezier(0.23,1,0.32,1)）· 动效预算表统一使用
fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(5)
}

fn icon_color(e: &FsEntry) -> Color32 {
    if e.is_dir {
        return AMBER;
    }
    match e.ext.as_str() {
        "exe" | "msi" => ACC,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => GREEN,
        "doc" | "docx" | "pdf" | "md" | "txt" | "log" => PINK,
        "zip" | "rar" | "7z" | "gz" | "tar" => PURPLE,
        "rs" | "py" | "js" | "ts" | "toml" | "json" | "yaml" | "yml" => TEAL,
        _ => GRAY,
    }
}

fn type_name(e: &FsEntry) -> &'static str {
    if e.is_dir {
        "文件夹"
    } else {
        match e.ext.as_str() {
            "exe" => "应用",
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => "图片",
            "txt" | "log" => "文本",
            "md" => "Markdown",
            "zip" | "rar" | "7z" => "压缩",
            "pdf" => "PDF",
            "docx" => "DOCX",
            "rs" => "Rust",
            "py" => "Python",
            _ => "文件",
        }
    }
}

fn fmt_dt(secs: i64) -> String {
    if secs == 0 {
        return "-".into();
    }
    fmt_time(secs).replace('-', "/")
}

fn estimate_width(s: &str) -> f32 {
    let mut w = 0.0;
    for ch in s.chars() {
        w += if (ch as u32) > 0x2E80 { 12.5 } else { 6.8 };
    }
    w
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}

/// 按可用像素宽度截断（预留给省略号），用于名称列自适应
fn truncate_width(s: &str, max_w: f32) -> String {
    let ell = 10.0; // "…" 的预估宽度
    if estimate_width(s) <= max_w {
        return s.to_string();
    }
    let budget = (max_w - ell).max(0.0);
    let mut acc = 0.0f32;
    let mut out = String::new();
    for ch in s.chars() {
        let cw = if (ch as u32) > 0x2E80 { 12.5 } else { 6.8 };
        if acc + cw > budget {
            break;
        }
        acc += cw;
        out.push(ch);
    }
    if out.is_empty() {
        return String::new();
    }
    out.push('…');
    out
}

// ================================================================ 模型

#[derive(PartialEq, Clone, Copy)]
enum SortKey {
    Name,
    Date,
    Size,
    Type,
}

struct Tab {
    path: PathBuf,
    history: Vec<PathBuf>,
    hi: usize,
    entries: Vec<FsEntry>,
    loading: bool,
    rx: Option<std::sync::mpsc::Receiver<LsResult>>,
    sel: BTreeSet<usize>,
    sel_last: Option<usize>,
    sort: SortKey,
    sort_desc: bool,
    query: String,
    /// 过滤+排序后的 entry 下标缓存（脏了才重算，避免每帧多次 O(n log n)）
    view: Arc<Vec<usize>>,
    view_dirty: bool,
}

impl Tab {
    fn new(path: PathBuf) -> Self {
        let mut t = Tab {
            path,
            history: Vec::new(),
            hi: usize::MAX,
            entries: Vec::new(),
            loading: true,
            rx: None,
            sel: BTreeSet::new(),
            sel_last: None,
            sort: SortKey::Name,
            sort_desc: false,
            query: String::new(),
            view: Arc::new(Vec::new()),
            view_dirty: true,
        };
        t.reload();
        t
    }

    /// 把当前标签「摘走」成独立 Tab（原位置由调用方 remove）。
    /// 直接搬状态、不重扫目录；视图缓存置脏，避免沿用原下标的临时态。
    fn take_detached(&mut self) -> Tab {
        Tab {
            path: self.path.clone(),
            history: std::mem::take(&mut self.history),
            hi: self.hi,
            entries: std::mem::take(&mut self.entries),
            loading: self.loading,
            rx: self.rx.take(),
            sel: std::mem::take(&mut self.sel),
            sel_last: self.sel_last,
            sort: self.sort,
            sort_desc: self.sort_desc,
            query: std::mem::take(&mut self.query),
            view: Arc::new(Vec::new()),
            view_dirty: true,
        }
    }

    fn reload(&mut self) {
        self.rx = Some(spawn_ls(self.path.clone()));
        self.loading = true;
        self.sel.clear();
        self.sel_last = None;
        self.view_dirty = true;
    }

    fn poll(&mut self) {
        let mut got = None;
        if let Some(rx) = self.rx.as_ref() {
            while let Ok(r) = rx.try_recv() {
                got = Some(r);
            }
        }
        if let Some(r) = got {
            match r {
                Ok((p, es)) => {
                    self.loading = false;
                    self.rx = None;
                    if p == self.path {
                        self.entries = es;
                        self.view_dirty = true;
                    }
                }
                Err(_) => {
                    self.entries.clear();
                    self.loading = false;
                    self.rx = None;
                    self.view_dirty = true;
                }
            }
        }
    }

    fn title(&self) -> String {
        let s = self.path.to_string_lossy().to_string();
        if s.is_empty() || s == "\\\\" {
            "此电脑".into()
        } else {
            short_name(&self.path)
        }
    }

    fn navigate(&mut self, p: PathBuf) {
        if self.path == p {
            return;
        }
        if self.hi < self.history.len() {
            self.history.truncate(self.hi + 1);
        }
        // 后退后再跳转时，当前目录可能已在栈顶：避免重复入栈导致「连按两次后退不动」
        if self.history.last() != Some(&self.path) {
            self.history.push(self.path.clone());
        }
        self.hi = self.history.len();
        self.path = p;
        self.query.clear();
        self.reload();
    }

    fn back(&mut self) {
        if self.hi > 0 {
            self.hi -= 1;
            let p = self.history[self.hi].clone();
            self.path = p;
            self.reload();
        }
    }

    fn fwd(&mut self) {
        if self.hi + 1 < self.history.len() {
            self.hi += 1;
            let p = self.history[self.hi].clone();
            self.path = p;
            self.reload();
        }
    }

    fn can_back(&self) -> bool {
        self.hi > 0
    }
    fn can_fwd(&self) -> bool {
        self.hi + 1 < self.history.len()
    }

    /// 过滤 + 排序后的 entry 下标序列
    fn view(&mut self) -> Arc<Vec<usize>> {
        if self.view_dirty {
            self.rebuild_view();
        }
        self.view.clone()
    }

    fn rebuild_view(&mut self) {
        let q = self.query.to_lowercase();
        let mut v: Vec<usize> = if q.is_empty() {
            (0..self.entries.len()).collect()
        } else {
            (0..self.entries.len())
                .filter(|&i| self.entries[i].name.to_lowercase().contains(&q))
                .collect()
        };
        let desc = self.sort_desc;
        let key = self.sort;
        let es = &self.entries;
        v.sort_by(|&a, &b| {
            let (ea, eb) = (&es[a], &es[b]);
            // 文件夹恒定在前：不参与升降序翻转（Windows 行为）
            if ea.is_dir != eb.is_dir {
                return eb.is_dir.cmp(&ea.is_dir);
            }
            let o = match key {
                SortKey::Name => ea.name.to_lowercase().cmp(&eb.name.to_lowercase()),
                SortKey::Date => ea.mtime.cmp(&eb.mtime),
                SortKey::Size => ea.size.cmp(&eb.size),
                // 同类型内部再按名字排，避免同类之间顺序不确定
                SortKey::Type => ea
                    .ext
                    .cmp(&eb.ext)
                    .then_with(|| ea.name.to_lowercase().cmp(&eb.name.to_lowercase())),
            };
            if desc {
                o.reverse()
            } else {
                o
            }
        });
        self.view = Arc::new(v);
        self.view_dirty = false;
    }

    fn set_query(&mut self, q: String) {
        if self.query != q {
            self.query = q;
            self.view_dirty = true;
        }
    }

    fn set_sort(&mut self, k: SortKey) {
        if self.sort == k {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort = k;
            self.sort_desc = false;
        }
        self.view_dirty = true;
    }

    fn sel_paths(&self) -> Vec<PathBuf> {
        self.sel.iter().filter_map(|&i| self.entries.get(i)).map(|e| e.path.clone()).collect()
    }
}

#[derive(Clone)]
enum Popup {
    Tab {
        idx: usize,
    },
    /// 右键点在条目上（动作针对选区）
    File,
    /// 右键点在空白处（动作针对当前目录）
    Empty,
}

struct CrumbMenu {
    path: PathBuf,
    anchor: Pos2,
    rx: Option<std::sync::mpsc::Receiver<LsResult>>,
    dirs: Vec<FsEntry>,
}

struct TabDrag {
    idx: usize,
    press: Pos2,
    pos: Pos2,
    active: bool,
    /// 按下点是否落在关闭按钮上（防「按标签中段、松手在 ×」误关）
    press_in_close: bool,
}

struct Rename {
    target: PathBuf,
    buf: String,
    focus_once: bool,
    born: f64,
}

/// 条目拖拽（列表内 → 文件夹/侧栏/面包屑）
struct FileDrag {
    paths: Vec<PathBuf>,
    start: Pos2,
    pos: Pos2,
    active: bool,
}

/// 当前拖拽落点（目标目录 + 高亮矩形）
struct DropTarget {
    dst: PathBuf,
    rect: Rect,
}

struct WinState {
    id: ViewportId,
    tabs: Vec<Tab>,
    ti: usize,
    sidebar: bool,
    popup: Option<Popup>,
    popup_at: f64,
    /// 打开菜单那一刻的指针位置（快照）。每帧重取 interact_pos 会让菜单跟着鼠标漂
    popup_anchor: Pos2,
    popup_rect: Option<Rect>,
    crumb_menu: Option<CrumbMenu>,
    tab_drag: Option<TabDrag>,
    fdrag: Option<FileDrag>,
    drop: Option<DropTarget>,
    /// 后台任务结束后自动刷新（移动/删除后列表要跟着变）
    reload_when_idle: bool,
    rename: Option<Rename>,
    tab_rects: Vec<(usize, Rect)>,
    focus_search: bool,
    close_requested: bool,
    new_windows: Vec<(Tab, Option<[f32; 2]>)>,
    maximized: bool,
    /// 首次出现时的窗口位置（撕出时=松手点）
    spawn_at: Option<[f32; 2]>,
}

impl WinState {
    fn blank(id: ViewportId) -> Self {
        WinState {
            id,
            tabs: Vec::new(),
            ti: 0,
            sidebar: true,
            popup: None,
            popup_at: 0.0,
            popup_anchor: Pos2::ZERO,
            popup_rect: None,
            crumb_menu: None,
            tab_drag: None,
            fdrag: None,
            drop: None,
            reload_when_idle: false,
            rename: None,
            tab_rects: Vec::new(),
            focus_search: false,
            close_requested: false,
            new_windows: Vec::new(),
            maximized: false,
            spawn_at: None,
        }
    }
    fn new(id: ViewportId, path: PathBuf) -> Self {
        let mut s = WinState::blank(id);
        s.tabs.push(Tab::new(path));
        s
    }
    /// 接管一个已有 Tab（撕出/新窗口打开），不再重复扫一遍目录
    fn with_tab(id: ViewportId, tab: Tab) -> Self {
        let mut s = WinState::blank(id);
        s.tabs.push(tab);
        s
    }
    fn cur(&self) -> &Tab {
        &self.tabs[self.ti.min(self.tabs.len().saturating_sub(1))]
    }
    fn cur_mut(&mut self) -> &mut Tab {
        let i = self.ti.min(self.tabs.len().saturating_sub(1));
        &mut self.tabs[i]
    }
    fn any_popup(&self) -> bool {
        self.popup.is_some() || self.crumb_menu.is_some()
    }
    /// 打开右键菜单：锚点在此刻快照一次，之后鼠标移动不再牵动菜单
    fn open_menu(&mut self, p: Popup, anchor: Pos2, now: f64) {
        self.popup = Some(p);
        self.popup_at = now;
        self.popup_anchor = anchor;
    }
}

struct Toast {
    text: String,
    err: bool,
    born: f64,
}

struct Shared {
    clip: Mutex<(Vec<PathBuf>, bool)>,
    jobs: Mutex<Vec<(String, JobHandle)>>,
    toasts: Mutex<Vec<Toast>>,
    drives: Vec<(String, PathBuf)>,
    quick: Vec<(String, PathBuf)>,
    start: std::time::Instant,
    /// 盘符剩余空间缓存（路径 → (文案, 采样时间)）。
    /// GetDiskFreeSpaceExW 是同步调用，休眠的移动硬盘 / 断线的网络盘会把 UI 卡死几秒，
    /// 所以每个盘 5 秒内只查一次，绝不能放在绘制循环里每帧调。
    free_cache: Mutex<std::collections::HashMap<PathBuf, (String, f64)>>,
}

impl Shared {
    fn new() -> Self {
        let home = home_dir();
        let quick: Vec<(String, PathBuf)> = [
            ("桌面", home.join("Desktop")),
            ("下载", home.join("Downloads")),
            ("文档", home.join("Documents")),
            ("图片", home.join("Pictures")),
            ("音乐", home.join("Music")),
            ("视频", home.join("Videos")),
        ]
        .into_iter()
        .filter(|(_, p)| p.is_dir())
        .map(|(n, p)| (n.to_string(), p))
        .collect();
        Shared {
            clip: Mutex::new((Vec::new(), false)),
            jobs: Mutex::new(Vec::new()),
            toasts: Mutex::new(Vec::new()),
            drives: drives(),
            quick,
            start: std::time::Instant::now(),
            free_cache: Mutex::new(std::collections::HashMap::new()),
        }
    }
    fn t(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
    /// 盘符剩余空间文案；5 秒缓存，避免每帧一次同步系统调用
    fn free_text(&self, p: &std::path::Path) -> String {
        let t = self.t();
        {
            let c = self.free_cache.lock().unwrap();
            if let Some((s, at)) = c.get(p) {
                if t - at < 5.0 {
                    return s.clone();
                }
            }
        }
        let s = drive_free(p)
            .map(|f| format!("剩余 {}", fmt_size(f)))
            .unwrap_or_default();
        self.free_cache.lock().unwrap().insert(p.to_path_buf(), (s.clone(), t));
        s
    }
    fn toast(&self, text: String, err: bool) {
        let t = self.t();
        let mut v = self.toasts.lock().unwrap();
        v.retain(|x| t - x.born < 4.4);
        v.push(Toast { text, err, born: t });
    }
}

// ================================================================ App

struct App {
    root: WinState,
    secondary: Vec<WinState>,
    shared: Arc<Shared>,
    counter: AtomicU64,
    cli_extra: Vec<PathBuf>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        setup_fonts(&cc.egui_ctx);
        apply_visuals(&cc.egui_ctx);
        let mut args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
        args.retain(|p| p.is_dir());
        let start = args
            .first()
            .cloned()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| home_dir()));
        App {
            root: WinState::new(ViewportId::ROOT, start),
            secondary: Vec::new(),
            shared: Arc::new(Shared::new()),
            counter: AtomicU64::new(1),
            cli_extra: args.into_iter().skip(1).collect(),
        }
    }

    fn next_id(&self) -> ViewportId {
        ViewportId::from_hash_of(self.counter.fetch_add(1, Ordering::Relaxed))
    }

    fn spawn_pos(&self, n: usize) -> [f32; 2] {
        let n = n as f32;
        [90.0 + (n * 34.0) % 260.0, 80.0 + (n * 34.0) % 220.0]
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // CLI 参数首批开多窗
        if !self.cli_extra.is_empty() {
            let extras = std::mem::take(&mut self.cli_extra);
            for p in extras {
                let id = self.next_id();
                self.secondary.push(WinState::new(id, p));
            }
        }

        // 后台任务轮询 → toast（全局一次）
        {
            let s = &self.shared;
            let mut jobs = s.jobs.lock().unwrap();
            let mut i = 0;
            let mut active = false;
            while i < jobs.len() {
                let done = jobs[i].1.progress.lock().unwrap().done;
                if done {
                    let (label, h) = jobs.remove(i);
                    let g = h.progress.lock().unwrap();
                    match &g.result {
                        Some(Ok(n)) => s.toast(format!("{}完成 · {}", label, fmt_size(*n)), false),
                        Some(Err(e)) => s.toast(format!("{}失败：{}", label, e), true),
                        None => {}
                    }
                } else {
                    active = true;
                    i += 1;
                }
            }
            drop(jobs);
            if active {
                ctx.request_repaint_after(Duration::from_millis(400));
            }
            let t = s.t();
            s.toasts.lock().unwrap().retain(|x| t - x.born < 4.4);
        }

        // ---- 二级原生窗口
        let n = self.secondary.len();
        for wi in 0..n {
            let (vid, title, pos) = {
                let w = &self.secondary[wi];
                (
                    w.id,
                    format!("{} — RustExplorer", w.cur().title()),
                    w.spawn_at.unwrap_or_else(|| self.spawn_pos(wi + 1)),
                )
            };
            let builder = ViewportBuilder::default()
                .with_inner_size([1040.0, 660.0])
                .with_min_inner_size([680.0, 420.0])
                .with_position(pos)
                .with_decorations(false)
                .with_icon(app_icon())
                .with_title(&title);
            let shared = self.shared.clone();
            ctx.show_viewport_immediate(vid, builder, |c2, class| {
                let w = &mut self.secondary[wi];
                match class {
                    ViewportClass::Embedded => {
                        egui::Window::new("rustexplorer")
                            .default_pos([pos[0], pos[1]])
                            .show(c2, |ui| {
                                ui.allocate_ui(Vec2::new(1000.0, 620.0), |ui| {
                                    win_ui(ui.ctx(), w, &shared);
                                });
                            });
                    }
                    _ => {
                        let close = win_ui(c2, w, &shared);
                        if c2.input(|i| i.viewport().close_requested()) || close {
                            w.close_requested = true;
                        }
                    }
                }
            });
        }

        // 新窗口请求（撕出 / 右键"在新窗口打开" / Ctrl+N）
        let mut spawns: Vec<(Tab, Option<[f32; 2]>)> = Vec::new();
        spawns.extend(self.root.new_windows.drain(..));
        for w in &mut self.secondary {
            spawns.extend(w.new_windows.drain(..));
        }
        for (tab, pos) in spawns {
            let id = self.next_id();
            let mut nw = WinState::with_tab(id, tab);
            nw.spawn_at = pos;
            self.secondary.push(nw);
        }

        // 移除请求关闭的子窗口
        for w in &self.secondary {
            if w.close_requested {
                ctx.send_viewport_cmd_to(w.id, ViewportCommand::Close);
            }
        }
        self.secondary.retain(|w| !w.close_requested);

        // ---- 根窗口
        let shared = self.shared.clone();
        let close_root = {
            let root = &mut self.root;
            win_ui(ctx, root, &shared)
        };
        if self.root.close_requested || close_root {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

// 让 (Tab, Option<[f32;2]>) 可从 drain 移出：Tab 本身即拥有所有权，直接 move

// ================================================================ 窗口 UI 主体

fn win_ui(ctx: &egui::Context, w: &mut WinState, shared: &Arc<Shared>) -> bool {
    let now = shared.t();
    for t in w.tabs.iter_mut() {
        t.poll();
    }

    // 每帧重算拖拽落点（面板按 侧栏→内容 顺序绘制，后画的覆盖先画的）
    w.drop = None;
    // 指针位置与"是否已构成拖拽"统一在这里更新，松手帧也能拿到最新落点
    if let Some(d) = w.fdrag.as_mut() {
        if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
            d.pos = p;
            if d.pos.distance(d.start) > 6.0 {
                d.active = true;
            }
        }
    }
    // 后台任务收尾后自动刷新（移动/删除/拖放都要反映到列表）
    if w.reload_when_idle && shared.jobs.lock().unwrap().is_empty() {
        w.reload_when_idle = false;
        for t in w.tabs.iter_mut() {
            t.reload();
        }
    }

    let vp_focus = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    let editing = ctx.memory(|m| m.focused().is_some());

    // 点击 popup 外部 / Esc → 关闭
    if w.any_popup() && w.rename.is_none() {
        let outside = ctx.input(|i| {
            i.pointer.button_clicked(egui::PointerButton::Primary)
                && match (i.pointer.interact_pos(), w.popup_rect) {
                    (Some(p), Some(r)) => !r.expand(4.0).contains(p),
                    (Some(_), None) => true,
                    _ => false,
                }
        });
        if outside {
            w.popup = None;
            w.crumb_menu = None;
        }
    }

    // ---- 标签栏（兼作标题条：无原生标题栏，标题/控制钮在这一行）
    TopBottomPanel::top("tabbar")
        .exact_height(TAB_H + 8.0)
        .frame(Frame::none().fill(BG2).inner_margin(Margin { left: 6, right: 0, top: 8, bottom: 8 }))
        .show(ctx, |ui| {
            tab_bar(ui, w, shared);
        });
    if w.tabs.is_empty() {
        return true; // 末标签已在栏内被关闭 → 关窗
    }

    if w.tab_drag.as_ref().map(|d| d.active).unwrap_or(false) {
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    // ---- 拖标签松手：撕出 / 重排（仅松手帧处理；普通点击在 tab_bar 内直接选中）
    let released = ctx.input(|i| !i.pointer.button_down(egui::PointerButton::Primary));
    if released && w.tab_drag.as_ref().map(|d| !d.active).unwrap_or(false) {
        w.tab_drag = None; // 按下后没构成拖拽、又没形成点击 → 清掉残留
    }
    if released && w.tab_drag.as_ref().map(|d| d.active).unwrap_or(false) {
        let drag = w.tab_drag.take().unwrap();
        // 坐标注意：inner_rect 是「显示器坐标」，pointer 是「窗口内坐标」，
        // 必须加上窗口原点再比，否则拖到哪都被判成"窗外"。
        let outside = ctx.input(|i| match (i.viewport().inner_rect, i.pointer.latest_pos()) {
            (Some(r), Some(p)) => !r.expand(4.0).contains(r.min + p.to_vec2()),
            (Some(_), None) => true, // 指针已完全离开窗口
            _ => false,
        });
        if outside && w.tabs.len() > 1 {
            // 撕出：tab 状态整体搬进新窗口，新窗在松手点出现（空间一致性）
            // drag.pos 是窗口内坐标，窗口位置需要显示器坐标
            let origin = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min).unwrap_or_default());
            let at = [origin.x + drag.pos.x - 60.0, origin.y + drag.pos.y - 16.0];
            let tab = w.tabs.remove(drag.idx);
            if w.ti >= w.tabs.len() {
                w.ti = w.tabs.len().saturating_sub(1);
            }
            w.new_windows.push((tab, Some(at)));
        } else if w.tab_rects.len() > 1 {
            let mut target = drag.idx;
            let mut best = f32::MAX;
            for (i, r) in w.tab_rects.iter() {
                let d = (r.center().x - drag.pos.x).abs();
                if d < best {
                    best = d;
                    target = *i;
                }
            }
            if target != drag.idx {
                let tab = w.tabs.remove(drag.idx);
                w.tabs.insert(target.min(w.tabs.len()), tab);
                w.ti = target;
            }
        }
    }

    // ---- 工具条
    TopBottomPanel::top("toolbar")
        .exact_height(TB_H)
        .frame(Frame::none().fill(BG).inner_margin(Margin::symmetric(10, 8)))
        .show(ctx, |ui| {
            toolbar(ui, w);
        });

    // ---- 状态栏
    TopBottomPanel::bottom("statusbar")
        .exact_height(SB_H)
        .frame(Frame::none().fill(BG).inner_margin(Margin::symmetric(12, 5)))
        .show(ctx, |ui| {
            status_bar(ui, w, shared);
        });

    // ---- 侧栏
    if w.sidebar {
        egui::SidePanel::left("sidebar")
            .exact_width(SIDE_W)
            .frame(Frame::none().fill(BG).inner_margin(Margin::same(8)))
            .show(ctx, |ui| {
                sidebar(ui, w, shared);
            });
    }

    // ---- 内容区
    egui::CentralPanel::default()
        .frame(Frame::none().fill(BG))
        .show(ctx, |ui| {
            content(ui, w, shared);
        });

    // ---- 从外部拖入（桌面 / Explorer / 其他程序）：落到当前目录
    let dropped: Vec<PathBuf> = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .filter_map(|f| f.path.clone())
            .collect()
    });
    if !dropped.is_empty() {
        let dst = w.cur().path.clone();
        if dst.as_os_str().is_empty() {
            shared.toast("先打开一个文件夹再拖入".into(), true);
        } else {
            // replace=false：外部拖进来的同名文件跳过，绝不静默覆盖（与内部拖拽保持一致）
            let h = start_job(CopyOp::Copy, dropped, dst, false);
            shared.jobs.lock().unwrap().push(("复制".into(), h));
            w.reload_when_idle = true;
        }
    }
    // 外部文件悬停在窗口上时的提示
    if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
        if let Some(p) = ctx.input(|i| i.pointer.hover_pos()) {
            let txt = format!("松开 → 复制到 {}", short_name(&w.cur().path));
            let font = FontId::proportional(12.5);
            let g = ctx.fonts(|f| f.layout_no_wrap(txt, font, FG));
            let r = Rect::from_center_size(
                p + Vec2::new(0.0, 26.0),
                g.size() + Vec2::new(22.0, 14.0),
            );
            let pt = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, Id::new("dropin")));
            pt.rect_filled(r, 6.0, SURF);
            pt.rect_stroke(r, 6.0, (0.5, LINE2), StrokeKind::Inside);
            pt.galley(r.min + Vec2::new(11.0, 7.0), g, FG);
        }
    }

    // ---- 条目拖拽：松手 → 落到目标目录（本帧的 drop 已在上面绘制时算好）
    if released {
        if let Some(d) = w.fdrag.take() {
            if d.active {
                if let Some(t) = w.drop.take() {
                    do_drop(ctx, w, shared, d.paths, t.dst);
                }
            }
        }
    }

    // ---- 无边框：边缘 resize 手柄（系统级 BeginResize，跟手/吸附原生）
    edge_resize(ctx, w);

    // ---- 条目拖拽 ghost + 落点高亮
    if w.fdrag.as_ref().map(|d| d.active).unwrap_or(false) {
        let d = w.fdrag.as_ref().unwrap();
        let txt = format!("{} 个项目", d.paths.len());
        let font = FontId::proportional(12.0);
        let g = ctx.fonts(|f| f.layout_no_wrap(txt, font, FG));
        let r = Rect::from_min_size(d.pos + Vec2::new(14.0, 12.0), g.size() + Vec2::new(20.0, 12.0));
        let pt = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, Id::new("fdragghost")));
        pt.rect_filled(r, 6.0, SURF);
        pt.rect_stroke(r, 6.0, (0.5, LINE2), StrokeKind::Inside);
        pt.galley(r.min + Vec2::new(10.0, 6.0), g, FG);
        ctx.set_cursor_icon(if w.drop.is_some() {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::Grabbing
        });
        if let Some(t) = w.drop.as_ref() {
            let pt = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("droptarget")));
            pt.rect_filled(t.rect, 4.0, ACC_BG);
            pt.rect_stroke(t.rect, 4.0, (1.5, ACC), StrokeKind::Inside);
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    // ---- 拖拽 ghost（跟手标签影子）
    if let Some(d) = w.tab_drag.as_ref() {
        if d.active {
            let title = w.tabs.get(d.idx).map(|t| t.title()).unwrap_or_default();
            egui::Area::new(Id::new("tabghost"))
                .fixed_pos(d.pos + Vec2::new(14.0, 10.0))
                .order(Order::Foreground)
                .show(ctx, |ui| {
                    Frame::none()
                        .fill(SURF)
                        .rounding(6.0)
                        .inner_margin(Margin::symmetric(10, 5))
                        .stroke((0.5, LINE2))
                        .show(ui, |ui| {
                            ui.label(RichText::new(truncate(&title, 24)).size(12.5).color(FG));
                        });
                });
        }
    }

    // ---- 弹层
    draw_popups(ctx, w, shared, now);
    if now - w.popup_at < 0.4 || w.crumb_menu.is_some() {
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    // ---- 重命名对话框
    if w.rename.is_some() {
        draw_rename(ctx, w, shared, now);
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    // ---- toast（仅聚焦窗口绘制，避免多窗重复）
    if vp_focus {
        let has_toast = !shared.toasts.lock().unwrap().is_empty();
        draw_toasts(ctx, shared, now);
        if has_toast {
            // 只在有 toast 时续帧，避免空转 4Hz 重绘
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    // ---- 快捷键
    if vp_focus && !editing && w.rename.is_none() {
        shortcuts(ctx, w, shared);
    }

    if w.tabs.is_empty() {
        return true; // 最后一个标签关闭 → 关窗
    }
    false
}

// ================================================================ 标签栏

enum TabEv {
    DragMove,
    Release(usize, bool), // (tab idx, 点在关闭按钮上)
    Menu(usize),
    Plus,
}

fn tab_bar(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    let max_w = ui.available_width();
    let y = ui.cursor().min.y;
    let x0 = ui.cursor().min.x;
    let right = ui.max_rect().max.x;

    // ---- 右侧：标题 + 窗口控制钮（标题栏并入标签栏这一行）
    const WB_W: f32 = 40.0;
    const WB_H: f32 = 26.0;
    let close_r = Rect::from_min_size(Pos2::new(right - WB_W, y), Vec2::new(WB_W, WB_H));
    let max_r = Rect::from_min_size(Pos2::new(right - WB_W * 2.0, y), Vec2::new(WB_W, WB_H));
    let min_r = Rect::from_min_size(Pos2::new(right - WB_W * 3.0, y), Vec2::new(WB_W, WB_H));
    let title_w = estimate_width("RustExplorer") + 8.0;
    let title_r = Rect::from_min_size(Pos2::new(min_r.min.x - title_w - 6.0, y), Vec2::new(title_w, WB_H));
    let drag_reserve = WB_W * 3.0 + title_w + 20.0;

    // 布局：固定宽度、超界截断
    let mut rects: Vec<(usize, Rect, bool)> = Vec::new();
    let mut x = x0;
    for (i, t) in w.tabs.iter().enumerate() {
        let tw = (estimate_width(&t.title()) + 46.0).clamp(96.0, 200.0);
        if x + tw > x0 + max_w - 20.0 - drag_reserve {
            break;
        }
        rects.push((i, Rect::from_min_size(Pos2::new(x, y), Vec2::new(tw, TAB_H)), t.loading));
        x += tw + 2.0;
    }
    // 被截断的尾部标签数量提示
    let hidden = w.tabs.len() - rects.len();
    w.tab_rects = rects.iter().map(|(i, r, _)| (*i, *r)).collect();

    let plus_rect = Rect::from_min_size(
        Pos2::new(x + if hidden > 0 { 40.0 } else { 2.0 }, y + (TAB_H - 26.0) / 2.0),
        Vec2::new(26.0, 26.0),
    );
    let drag_idx = w.tab_drag.as_ref().map(|d| d.idx);
    let ptr = ui.input(|i| i.pointer.hover_pos());

    for (i, r, loading) in rects.iter() {
        let is_cur = *i == w.ti;
        let is_drag = drag_idx == Some(*i);
        let hov = ptr.map(|p| r.contains(p)).unwrap_or(false);
        let fill = if is_cur {
            SURF
        } else if hov && !is_drag {
            HOVER
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(
                Rect::from_min_max(r.min, Pos2::new(r.max.x, r.max.y + 1.0)),
                egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 },
                fill,
            );
        }
        if is_drag {
            ui.painter()
                .rect_stroke(*r, 8.0, (1.0, LINE2), StrokeKind::Inside);
        }
        if is_cur {
            ui.painter().rect_filled(
                Rect::from_min_size(Pos2::new(r.min.x + 10.0, r.max.y - 2.5), Vec2::new(r.width() - 20.0, 2.0)),
                1.0,
                ACC,
            );
        }
        let t = &w.tabs[*i];
        ui.painter().rect_filled(
            Rect::from_center_size(Pos2::new(r.min.x + 12.0, r.center().y), Vec2::new(12.0, 12.0)),
            3.0,
            tab_tint(t),
        );
        let name = t.title();
        let loadname = format!("{}…", name);
        let display = truncate(if *loading { &loadname } else { &name }, 22);
        ui.painter().text(
            Pos2::new(r.min.x + 28.0, r.center().y),
            Align2::LEFT_CENTER,
            display,
            FontId::proportional(12.5),
            if is_drag { MUTE } else if is_cur { FG } else { FG2 },
        );
        // 关闭按钮
        let cr = Rect::from_center_size(Pos2::new(r.max.x - 12.0, r.center().y), Vec2::new(18.0, 18.0));
        let chov = ptr.map(|p| cr.contains(p)).unwrap_or(false);
        if chov {
            ui.painter().rect_filled(cr, 4.0, HOVER2);
        }
        ui.painter().text(
            cr.center(),
            Align2::CENTER_CENTER,
            "×",
            FontId::proportional(13.0),
            if chov { FG } else { MUTE },
        );
    }
    if hidden > 0 {
        ui.painter().text(
            Pos2::new(x + 6.0, y + TAB_H / 2.0),
            Align2::LEFT_CENTER,
            format!("+{}", hidden),
            FontId::proportional(11.0),
            MUTE,
        );
    }

    // ＋
    let phov = ptr.map(|p| plus_rect.contains(p)).unwrap_or(false);
    if phov {
        ui.painter().rect_filled(plus_rect, 6.0, HOVER);
    }
    ui.painter().text(
        plus_rect.center(),
        Align2::CENTER_CENTER,
        "+",
        FontId::proportional(15.0),
        FG2,
    );

    // 交互收集
    let mut evs: Vec<TabEv> = Vec::new();
    let ctx = ui.ctx();
    for (i, r, _) in rects.iter() {
        let resp = ui.interact(*r, ui.id().with(("tabchip", *i)), Sense::click_and_drag());
        let xr = Rect::from_center_size(Pos2::new(r.max.x - 12.0, r.center().y), Vec2::new(18.0, 18.0));
        let in_close = ptr.map(|p| xr.contains(p)).unwrap_or(false);
        // 按下即记录按下点：clicked 帧的指针位置可能已经移到 × 上，不能拿它判关闭
        if resp.is_pointer_button_down_on() && w.tab_drag.is_none() {
            let p = ptr.unwrap_or_default();
            w.tab_drag = Some(TabDrag {
                idx: *i,
                press: p,
                pos: p,
                active: false,
                press_in_close: in_close,
            });
        }
        if resp.dragged() {
            if let Some(d) = w.tab_drag.as_mut() {
                if d.idx == *i {
                    let p = ptr.unwrap_or(d.pos);
                    d.pos = p;
                    if !d.active && (p - d.press).length() > 6.0 {
                        d.active = true;
                    }
                }
            }
            evs.push(TabEv::DragMove);
        }
        if resp.drag_stopped() {
            evs.push(TabEv::Release(*i, false));
        } else if resp.clicked() {
            let was_close =
                w.tab_drag.as_ref().map(|d| d.idx == *i && d.press_in_close).unwrap_or(false);
            evs.push(TabEv::Release(*i, was_close));
        }
        if resp.secondary_clicked() {
            evs.push(TabEv::Menu(*i));
        }
        if resp.middle_clicked() {
            evs.push(TabEv::Release(*i, true)); // 中键关标签
        }
    }
    if ui.interact(plus_rect, ui.id().with("tabplus"), Sense::click()).clicked() {
        evs.push(TabEv::Plus);
    }
    // 空白区（+ 按钮之后到控制钮之前）不再单独处理点击新建，
    // 与标题一起合并成下方的整段窗口拖动区。
    let empty_r = Rect::from_min_size(
        Pos2::new(plus_rect.max.x + 6.0, y),
        Vec2::new((title_r.min.x - 8.0 - plus_rect.max.x - 6.0).max(0.0), TAB_H),
    );

    // ---- 标题 + 控制钮绘制与交互
    let ctx0 = ui.ctx();
    w.maximized = ctx0.input(|i| i.viewport().maximized.unwrap_or(false));
    ui.painter().text(
        Pos2::new(title_r.center().x, title_r.center().y),
        Align2::CENTER_CENTER,
        "RustExplorer",
        FontId::proportional(11.5),
        MUTE,
    );
    let mon_hov = ptr.map(|p| min_r.contains(p)).unwrap_or(false);
    let mx_hov = ptr.map(|p| max_r.contains(p)).unwrap_or(false);
    let cl_hov = ptr.map(|p| close_r.contains(p)).unwrap_or(false);
    if mon_hov {
        ui.painter().rect_filled(min_r, 4.0, HOVER2);
    }
    if mx_hov {
        ui.painter().rect_filled(max_r, 4.0, HOVER2);
    }
    if cl_hov {
        ui.painter()
            .rect_filled(close_r, 4.0, Color32::from_rgb(232, 17, 35));
    }
    let gcol = |hov_ctl: bool, on_red: bool| {
        if on_red && hov_ctl {
            Color32::WHITE
        } else if hov_ctl {
            FG
        } else {
            FG2
        }
    };
    let cy = min_r.center().y;
    // —（最小化）
    ui.painter().line_segment(
        [
            Pos2::new(min_r.center().x - 5.0, cy),
            Pos2::new(min_r.center().x + 5.0, cy),
        ],
        (1.0, gcol(mon_hov, false)),
    );
    // □ / 还原
    if w.maximized {
        ui.painter().rect_stroke(
            Rect::from_min_size(Pos2::new(max_r.center().x - 4.0, cy - 5.0), Vec2::new(9.0, 9.0)),
            1.0,
            (1.0, gcol(mx_hov, false)),
            StrokeKind::Middle,
        );
        ui.painter().line_segment(
            [
                Pos2::new(max_r.center().x - 5.0, cy - 3.5),
                Pos2::new(max_r.center().x + 4.0, cy - 3.5),
            ],
            (1.0, gcol(mx_hov, false)),
        );
    } else {
        ui.painter().rect_stroke(
            Rect::from_center_size(max_r.center(), Vec2::new(10.0, 10.0)),
            1.0,
            (1.0, gcol(mx_hov, false)),
            StrokeKind::Middle,
        );
    }
    // ×（关闭）
    let cc = close_r.center();
    ui.painter().line_segment(
        [Pos2::new(cc.x - 5.0, cc.y - 5.0), Pos2::new(cc.x + 5.0, cc.y + 5.0)],
        (1.0, gcol(cl_hov, true)),
    );
    ui.painter().line_segment(
        [Pos2::new(cc.x - 5.0, cc.y + 5.0), Pos2::new(cc.x + 5.0, cc.y - 5.0)],
        (1.0, gcol(cl_hov, true)),
    );

    let mut do_min = false;
    let mut do_max = false;
    let mut do_close = false;
    if ui.interact(min_r, ui.id().with("winmin"), Sense::click()).clicked() {
        do_min = true;
    }
    if ui.interact(max_r, ui.id().with("winmax"), Sense::click()).clicked() {
        do_max = true;
    }
    if ui.interact(close_r, ui.id().with("winclose"), Sense::click()).clicked() {
        do_close = true;
    }
    // 标签栏空白 + 标题 = 整段拖动区（双击 = 最大化切换）
    let drag_r = Rect::from_min_size(
        empty_r.min,
        Vec2::new((min_r.min.x - 4.0 - empty_r.min.x).max(0.0), TAB_H),
    );
    let drag_resp = ui.interact(drag_r, ui.id().with("tbardrag"), Sense::click_and_drag());
    if drag_resp.drag_started() {
        ctx0.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag_resp.double_clicked() {
        do_max = true;
    }
    if do_min {
        ctx0.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
    if do_max {
        let nm = !w.maximized;
        ctx0.send_viewport_cmd(ViewportCommand::Maximized(nm));
        w.maximized = nm;
    }
    if do_close {
        w.close_requested = true;
        return;
    }

    // 应用
    for ev in evs {
        match ev {
            TabEv::DragMove => {
                if let Some(d) = w.tab_drag.as_mut() {
                    d.pos = ctx.input(|i| i.pointer.interact_pos()).unwrap_or(d.pos);
                }
            }
            TabEv::Release(i, close) => {
                let active = w.tab_drag.as_ref().map(|d| d.active).unwrap_or(false);
                if !active {
                    if close {
                        close_tab(w, i);
                    } else if i < w.tabs.len() {
                        w.ti = i; // 普通点击：松手即选中
                    }
                    w.tab_drag = None;
                }
                // active 的松手交给 win_ui（撕出/排序）
            }
            TabEv::Menu(i) => {
                let a = ctx.input(|inp| inp.pointer.interact_pos()).unwrap_or_default();
                w.open_menu(Popup::Tab { idx: i }, a, shared.t());
            }
            TabEv::Plus => {
                let p = w.cur().path.clone();
                let idx = w.ti + 1;
                w.tabs.insert(idx, Tab::new(p));
                w.ti = idx;
            }
        }
    }
    // 拖拽中每帧更新位置
    if let Some(d) = w.tab_drag.as_mut() {
        if d.active {
            let p = ctx.input(|i| i.pointer.interact_pos()).unwrap_or_default();
            d.pos = p;
        }
    }
}

fn tab_tint(t: &Tab) -> Color32 {
    let s = t.path.to_string_lossy().to_string();
    if s.is_empty() {
        GRAY
    } else if s.len() >= 2 && s.as_bytes()[1] == b':' && s.len() == 3 {
        GRAY // 驱动器根
    } else if s.contains("Downloads") || s.contains("下载") {
        TEAL
    } else {
        AMBER
    }
}

fn close_tab(w: &mut WinState, i: usize) {
    if i < w.tabs.len() {
        w.tabs.remove(i);
        if w.ti >= w.tabs.len() {
            w.ti = w.tabs.len().saturating_sub(1);
        }
    }
}

/// 无边框窗口的八向边缘 resize 手柄：hover 换光标，拖起发系统级 BeginResize。
fn edge_resize(ctx: &egui::Context, w: &WinState) {
    if w.maximized {
        return;
    }
    // screen_rect = 窗口内坐标 (0,0)-(w,h)；inner_rect 是显示器坐标，当画布原点会让手柄整体偏移
    let scr = ctx.screen_rect();
    if scr.width() < 10.0 || scr.height() < 10.0 {
        return;
    }
    let m = 5.0f32;
    let defs: [(&str, Pos2, Vec2, ResizeDirection, egui::CursorIcon); 8] = [
        ("w", scr.min, Vec2::new(m, scr.height()), ResizeDirection::West, egui::CursorIcon::ResizeWest),
        ("e", Pos2::new(scr.max.x - m, scr.min.y), Vec2::new(m, scr.height()), ResizeDirection::East, egui::CursorIcon::ResizeEast),
        ("n", scr.min, Vec2::new(scr.width(), m), ResizeDirection::North, egui::CursorIcon::ResizeNorth),
        ("s", Pos2::new(scr.min.x, scr.max.y - m), Vec2::new(scr.width(), m), ResizeDirection::South, egui::CursorIcon::ResizeSouth),
        ("nw", scr.min, Vec2::new(m, m), ResizeDirection::NorthWest, egui::CursorIcon::ResizeNwSe),
        ("ne", Pos2::new(scr.max.x - m, scr.min.y), Vec2::new(m, m), ResizeDirection::NorthEast, egui::CursorIcon::ResizeNeSw),
        ("sw", Pos2::new(scr.min.x, scr.max.y - m), Vec2::new(m, m), ResizeDirection::SouthWest, egui::CursorIcon::ResizeNeSw),
        ("se", Pos2::new(scr.max.x - m, scr.max.y - m), Vec2::new(m, m), ResizeDirection::SouthEast, egui::CursorIcon::ResizeSouthEast),
    ];
    for (tag, pos, size, dir, cur) in defs {
        let ctx2 = ctx.clone();
        egui::Area::new(Id::new(("edgerz", w.id, tag)))
            .fixed_pos(pos)
            .constrain(false)
            .order(Order::Foreground)
            .show(ctx, move |ui| {
                let (_, resp) = ui.allocate_exact_size(size, Sense::drag());
                if resp.hovered() {
                    ctx2.set_cursor_icon(cur);
                }
                if resp.drag_started() {
                    ctx2.send_viewport_cmd(ViewportCommand::BeginResize(dir));
                }
            });
    }
}

// ================================================================ 工具条 & 面包屑

fn toolbar(ui: &mut Ui, w: &mut WinState) {
    let h = 28.0;
    ui.horizontal(|ui| {
        let can_b = w.cur().can_back();
        if icon_btn(ui, "←", can_b, h).clicked() {
            w.cur_mut().back();
        }
        let can_f = w.cur().can_fwd();
        if icon_btn(ui, "→", can_f, h).clicked() {
            w.cur_mut().fwd();
        }
        let up = w.cur().path.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf());
        if icon_btn(ui, "↑", up.is_some(), h).clicked() {
            if let Some(p) = up.clone() {
                w.cur_mut().navigate(p);
            }
        }
        if icon_btn(ui, "⟳", true, h).clicked() {
            w.cur_mut().reload();
        }
        ui.add_space(4.0);
        breadcrumb(ui, w);
        ui.add_space(8.0);
        // 搜索框
        let want_focus = w.focus_search;
        w.focus_search = false;
        let mut q = w.cur().query.clone();
        let resp = ui.add(
            TextEdit::singleline(&mut q)
                .hint_text("搜索")
                .desired_width(130.0)
                .font(TextStyle::Body),
        );
        if want_focus {
            resp.request_focus();
        }
        w.cur_mut().set_query(q);
        ui.add_space(4.0);
        let sb = w.sidebar;
        if icon_btn(ui, if sb { "◧" } else { "▯" }, true, h).clicked() {
            w.sidebar = !w.sidebar;
        }
    });
}

fn icon_btn(ui: &mut Ui, glyph: &str, enabled: bool, h: f32) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(h, h), sense);
    let hov = resp.hovered() && enabled;
    if hov {
        ui.painter().rect_filled(rect, 6.0, HOVER);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(13.5),
        if !enabled {
            Color32::from_rgba_unmultiplied(138, 139, 143, 90)
        } else if hov {
            FG
        } else {
            FG2
        },
    );
    resp
}

fn breadcrumb(ui: &mut Ui, w: &mut WinState) {
    let parts = crumb_parts(&w.cur().path);
    let rect = ui.available_rect_before_wrap();
    let y = rect.center().y;
    let avail = (rect.width() - 330.0).max(120.0); // 给右侧搜索等留位

    struct Seg {
        rect: Rect,
        menu_rect: Rect,
        sep: Pos2,
        label: String,
        path: PathBuf,
        last: bool,
    }
    let mut segs: Vec<Seg> = Vec::new();
    let mut x = rect.min.x;
    let mut ellipsis = false;
    for (idx, (label, p)) in parts.iter().enumerate() {
        let is_last = idx == parts.len() - 1;
        let mut disp = label.clone();
        let mut lw = estimate_width(label) + 12.0;
        if x + lw > rect.min.x + avail && idx > 0 && !ellipsis {
            // 中间层级折叠为 …
            ellipsis = true;
            continue;
        }
        if ellipsis && !is_last {
            continue;
        }
        if x + lw > rect.min.x + avail {
            break;
        }
        if ellipsis && is_last {
            // 在末段前补画 … 段
            let er = Rect::from_min_size(Pos2::new(x, y - 11.0), Vec2::new(22.0, 22.0));
            segs.push(Seg {
                rect: er,
                menu_rect: Rect::NOTHING,
                sep: Pos2::new(er.max.x + 6.5, y),
                label: "…".into(),
                path: parts[0].1.clone(),
                last: false,
            });
            x = er.max.x + 14.0;
        }
        let r = Rect::from_min_size(Pos2::new(x, y - 11.0), Vec2::new(lw, 22.0));
        let mw = if is_last { 0.0 } else { 13.0 };
        let mr = Rect::from_min_size(Pos2::new(r.max.x + 1.0, y - 11.0), Vec2::new(mw, 22.0));
        x = mr.max.x;
        let sep = Pos2::new(x + 8.0, y);
        if !is_last {
            x += 17.0;
        }
        segs.push(Seg {
            rect: r,
            menu_rect: mr,
            sep,
            label: disp.clone(),
            path: p.clone(),
            last: is_last,
        });
    }

    // 绘制
    for s in segs.iter() {
        let hov = ui.rect_contains_pointer(s.rect);
        if hov {
            ui.painter().rect_filled(s.rect, 4.0, HOVER);
        }
        ui.painter().text(
            Pos2::new(s.rect.min.x + 6.0, s.rect.center().y),
            Align2::LEFT_CENTER,
            &s.label,
            FontId::proportional(if s.last { 13.0 } else { 12.5 }),
            if s.last { FG } else { FG2 },
        );
        if !s.last {
            ui.painter().text(s.sep, Align2::CENTER_CENTER, "›", FontId::proportional(11.0), MUTE);
        }
        if s.menu_rect.width() > 0.0 {
            let mh = ui.rect_contains_pointer(s.menu_rect);
            if mh {
                ui.painter().rect_filled(s.menu_rect, 4.0, HOVER);
            }
            // 小下拉箭头：手绘三角（避免字体缺字）
            let c = s.menu_rect.center();
            let col = if mh { FG } else { MUTE };
            ui.painter().add(egui::Shape::convex_polygon(
                vec![c + Vec2::new(-3.0, -1.2), c + Vec2::new(3.0, -1.2), c + Vec2::new(0.0, 2.0)],
                col,
                egui::Stroke::NONE,
            ));
        }
    }
    // 占位推进布局
    ui.allocate_space(Vec2::new((x - rect.min.x).max(0.0), 22.0));

    let mut nav: Option<PathBuf> = None;
    let mut menu: Option<(PathBuf, Pos2)> = None;
    for (idx, s) in segs.iter().enumerate() {
        // 拖拽经过面包屑某一段 → 落点 = 该层目录
        if w.fdrag.as_ref().map(|d| d.active).unwrap_or(false) && ui.rect_contains_pointer(s.rect) {
            w.drop = Some(DropTarget { dst: s.path.clone(), rect: s.rect });
        }
        if ui.interact(s.rect, ui.id().with(("crumb", idx)), Sense::click()).clicked() {
            nav = Some(s.path.clone());
        }
        if s.menu_rect.width() > 0.0
            && ui.interact(s.menu_rect, ui.id().with(("crumbm", idx)), Sense::click()).clicked()
        {
            let a = Pos2::new(s.rect.min.x, s.rect.max.y + 4.0);
            menu = Some((s.path.clone(), a));
        }
    }
    if let Some(p) = nav {
        w.cur_mut().navigate(p);
    }
    if let Some((p, a)) = menu {
        w.crumb_menu = Some(CrumbMenu {
            path: p.clone(),
            anchor: a,
            rx: Some(spawn_ls(p)),
            dirs: Vec::new(),
        });
    }
}

fn crumb_parts(path: &PathBuf) -> Vec<(String, PathBuf)> {
    let s = path.to_string_lossy().to_string();
    if s.is_empty() || s == "\\\\" {
        return vec![("此电脑".into(), PathBuf::from("\\\\"))];
    }
    let rev: Vec<PathBuf> = path.ancestors().map(|p| p.to_path_buf()).collect::<Vec<_>>().into_iter().rev().collect();
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for (i, a) in rev.iter().enumerate() {
        let ls = a.to_string_lossy().to_string();
        let label = if i == 0 {
            if ls.len() >= 2 && ls.as_bytes()[1] == b':' {
                format!("{}:", ls.chars().next().unwrap())
            } else {
                "此电脑".into()
            }
        } else {
            short_name(a)
        };
        out.push((label, a.clone()));
    }
    out
}

// ================================================================ 侧栏 & 状态栏

fn sidebar(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    let cur = w.cur().path.clone();
    let mut nav: Option<PathBuf> = None;
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.label(RichText::new("快速访问").size(11.0).color(MUTE));
        ui.add_space(2.0);
        let quick_icons = [TEAL, GREEN, PINK, PURPLE, AMBER, ACC];
        for (k, (name, p)) in shared.quick.iter().enumerate() {
            let sel = *p == cur;
            let resp = side_item(ui, name, quick_icons[k % quick_icons.len()], sel);
            // 拖拽经过 → 落点 = 该快捷目录
            if w.fdrag.as_ref().map(|d| d.active).unwrap_or(false) && resp.hovered() {
                w.drop = Some(DropTarget { dst: p.clone(), rect: resp.rect });
            }
            if resp.clicked() {
                nav = Some(p.clone());
            }
        }
        ui.add_space(10.0);
        ui.label(RichText::new("此电脑").size(11.0).color(MUTE));
        ui.add_space(2.0);
        for (name, p) in shared.drives.iter() {
            let sel = cur.starts_with(p);
            // 卷标可能是中文：必须按字符截断（按字节切会 panic）
            let short = truncate(name, 18);
            let resp = side_item(ui, &short, GRAY, sel);
            if w.fdrag.as_ref().map(|d| d.active).unwrap_or(false) && resp.hovered() {
                w.drop = Some(DropTarget { dst: p.clone(), rect: resp.rect });
            }
            if resp.clicked() {
                nav = Some(p.clone());
            }
        }
    });
    if let Some(p) = nav {
        w.cur_mut().navigate(p);
    }
}

fn side_item(ui: &mut Ui, label: &str, dot: Color32, sel: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    let hov = resp.hovered() && !sel;
    let fill = if sel { ACC_BG } else if hov { HOVER } else { Color32::TRANSPARENT };
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    if sel {
        ui.painter().rect_filled(
            Rect::from_min_size(Pos2::new(rect.min.x + 1.0, rect.center().y - 7.0), Vec2::new(2.0, 14.0)),
            1.0,
            ACC,
        );
    }
    ui.painter().rect_filled(
        Rect::from_center_size(Pos2::new(rect.min.x + 14.0, rect.center().y), Vec2::new(11.0, 11.0)),
        3.0,
        dot,
    );
    ui.painter().text(
        Pos2::new(rect.min.x + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        truncate(label, 16),
        FontId::proportional(12.5),
        if sel { FG } else { FG2 },
    );
    resp
}

fn status_bar(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    ui.horizontal(|ui| {
        let t = w.cur_mut();
        let n = t.view().len();
        let nsel = t.sel.len();
        let bytes: u64 = t.sel.iter().filter_map(|&i| t.entries.get(i)).map(|e| e.size).sum();
        let mut left = format!("{} 个项目", n);
        if nsel > 0 {
            left.push_str(&format!(" · 已选择 {} 项", nsel));
            if bytes > 0 {
                left.push_str(&format!(" · {}", fmt_size(bytes)));
            }
        }
        ui.label(RichText::new(left).size(11.0).color(MUTE));
        let jobs = shared.jobs.lock().unwrap();
        if !jobs.is_empty() {
            ui.add_space(12.0);
            ui.spinner();
            ui.label(RichText::new(format!("{} 个后台任务", jobs.len())).size(11.0).color(FG2));
            if let Some((_, h)) = jobs.first() {
                let g = h.progress.lock().unwrap();
                ui.label(RichText::new(truncate(&g.text, 30)).size(11.0).color(MUTE));
            }
        }
        drop(jobs);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new("Ctrl+T 新标签 · 拖出标签 = 拆成新窗口")
                    .size(11.0)
                    .color(MUTE),
            );
        });
    });
}

// ================================================================ 内容区

#[derive(PartialEq)]
enum ContentAct {
    None,
    Open(usize),
    Select { i: usize, shift: bool, ctrl: bool },
    Ctx,
    EmptyCtx,
}

fn content(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    let path_s = w.cur().path.to_string_lossy().to_string();
    if path_s.is_empty() || path_s == "\\\\" {
        this_pc(ui, w, shared);
        return;
    }
    let loading = w.cur().loading;
    let mut view = w.cur_mut().view();
    if view.is_empty() {
        if loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("正在读取…").size(12.5).color(MUTE));
            });
        } else {
            let hint = if !w.cur().query.is_empty() { "无匹配结果" } else { "空文件夹" };
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(hint).size(12.5).color(MUTE));
            });
        }
        return;
    }

    let hdr = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), HEAD_H));
    let col_date = 132.0;
    let col_type = 84.0;
    let col_size = 92.0;
    let pad = 10.0;
    let x_date = hdr.max.x - col_size - col_type - col_date - pad;
    let x_type = hdr.max.x - col_size - col_type - pad;
    let x_size = hdr.max.x - col_size - pad;

    let mut sort_act: Option<SortKey> = None;
    sort_act = sort_act.or(header_cell(ui, Rect::from_min_max(hdr.min, Pos2::new(x_date, hdr.max.y)), "名称", SortKey::Name, w, false));
    sort_act = sort_act.or(header_cell(ui, Rect::from_min_max(Pos2::new(x_date, hdr.min.y), Pos2::new(x_type - 8.0, hdr.max.y)), "修改日期", SortKey::Date, w, true));
    sort_act = sort_act.or(header_cell(ui, Rect::from_min_max(Pos2::new(x_type - 8.0, hdr.min.y), Pos2::new(x_size - 8.0, hdr.max.y)), "类型", SortKey::Type, w, true));
    sort_act = sort_act.or(header_cell(ui, Rect::from_min_max(Pos2::new(x_size - 8.0, hdr.min.y), Pos2::new(hdr.max.x - pad, hdr.max.y)), "大小", SortKey::Size, w, true));
    if let Some(k) = sort_act {
        w.cur_mut().set_sort(k);
        view = w.cur_mut().view(); // 排序变了，当帧就用新顺序
    }
    ui.painter().hline(hdr.min.x..=hdr.max.x, hdr.max.y - 0.5, (0.5, LINE));
    ui.allocate_space(Vec2::new(0.0, HEAD_H));

    // 中央区域整体右键 → 空白菜单（在列表之外捕获）；左键空白 → 取消选择
    let central_rect = ui.available_rect_before_wrap();
    let bg = ui.interact(central_rect, ui.id().with("centralbg"), Sense::click());
    let empty_ctx = bg.secondary_clicked();
    let empty_click = bg.clicked();

    let view_len = view.len();
    let mut act = ContentAct::None;
    ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_H, view_len, |ui, range| {
        for vi in range {
            let i = view[vi];
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click_and_drag());
            let sel = w.cur().sel.contains(&i);
            let hov = resp.hovered() && !sel;
            let fill = if sel { ACC_BG } else if hov { HOVER } else { Color32::TRANSPARENT };
            let e = match w.cur().entries.get(i) {
                Some(e) => e,
                None => {
                    ui.allocate_space(Vec2::new(0.0, 0.0));
                    continue;
                }
            };
            if fill != Color32::TRANSPARENT {
                ui.painter().rect_filled(rect, 4.0, fill);
            }
            if sel {
                ui.painter().rect_filled(
                    Rect::from_min_size(Pos2::new(rect.min.x + 1.0, rect.center().y - 7.0), Vec2::new(2.0, 14.0)),
                    1.0,
                    ACC,
                );
            }
            ui.painter().rect_filled(
                Rect::from_center_size(Pos2::new(rect.min.x + 14.0, rect.center().y), Vec2::new(13.0, 13.0)),
                3.0,
                icon_color(e),
            );
            let name_c = if e.hidden { MUTE } else { FG };
            let name_max = (x_date - 14.0 - (rect.min.x + 30.0)).max(48.0);
            ui.painter().text(
                Pos2::new(rect.min.x + 30.0, rect.center().y),
                Align2::LEFT_CENTER,
                truncate_width(&e.name, name_max),
                FontId::proportional(12.5),
                name_c,
            );
            ui.painter().text(
                Pos2::new(x_date + col_date - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                fmt_dt(e.mtime),
                FontId::proportional(12.0),
                FG2,
            );
            ui.painter().text(
                Pos2::new(x_type + col_type - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                type_name(e),
                FontId::proportional(12.0),
                MUTE,
            );
            ui.painter().text(
                Pos2::new(rect.max.x - pad, rect.center().y),
                Align2::RIGHT_CENTER,
                if e.is_dir { String::new() } else { fmt_size(e.size) },
                FontId::proportional(12.0),
                FG2,
            );

            // e 的借用到此结束（下面要改 w）：需要的标量先拷出来
            let is_dir_row = e.is_dir;

            if resp.double_clicked() {
                act = ContentAct::Open(i);
            } else if resp.secondary_clicked() {
                act = ContentAct::Ctx;
                // 右键未命中当前选区 → 单选该项
                let contains = w.cur().sel.contains(&i);
                if !contains {
                    let t = w.cur_mut();
                    t.sel.clear();
                    t.sel.insert(i);
                    t.sel_last = Some(i);
                }
            } else if resp.clicked() {
                let (shift, ctrl) = ui.input(|inp| (inp.modifiers.shift, inp.modifiers.command));
                act = ContentAct::Select { i, shift, ctrl };
            }

            // ---- 拖拽（放在本行绘制与 e 借用结束之后，避免与 &mut w 冲突）
            if resp.drag_started() {
                // 没选中就按这一项拖；已选中则拖整个选区
                if !w.cur().sel.contains(&i) {
                    let t = w.cur_mut();
                    t.sel.clear();
                    t.sel.insert(i);
                    t.sel_last = Some(i);
                }
                let paths = w.cur().sel_paths();
                let p0 = resp.interact_pointer_pos().unwrap_or(rect.center());
                w.fdrag = Some(FileDrag { paths, start: p0, pos: p0, active: false });
            }
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    if let Some(d) = w.fdrag.as_mut() {
                        d.pos = p;
                        if d.pos.distance(d.start) > 6.0 {
                            d.active = true;
                        }
                    }
                }
            }
            // 拖到一个文件夹行上（不能是正在拖的自身/选区）
            let on_dir = w
                .fdrag
                .as_ref()
                .map(|d| d.active && rect.contains(d.pos))
                .unwrap_or(false);
            if on_dir && is_dir_row && !w.cur().sel.contains(&i) {
                // 重新借一次取路径，取完即释放，别让 e 的借用拖到这里
                let dst = w.cur().entries.get(i).map(|x| x.path.clone());
                if let Some(dst) = dst {
                    w.drop = Some(DropTarget { dst, rect });
                }
            }
        }
    });

    if act == ContentAct::None && empty_ctx {
        act = ContentAct::EmptyCtx;
    }
    // 点空白 = 取消选择（Files / Windows 行为）
    if act == ContentAct::None && empty_click {
        let t = w.cur_mut();
        if !t.sel.is_empty() {
            t.sel.clear();
            t.sel_last = None;
        }
    }

    match act {
        ContentAct::None => {}
        ContentAct::Open(i) => {
            let e = &w.cur().entries[i];
            let p = e.path.clone();
            if e.is_dir {
                w.cur_mut().navigate(p);
            } else {
                let _ = open_with_default(&p);
            }
        }
        ContentAct::Select { i, shift, ctrl } => {
            let view = w.cur_mut().view();
            let t = w.cur_mut();
            if shift {
                if let Some(last) = t.sel_last {
                    if let (Some(a), Some(b)) =
                        (view.iter().position(|&x| x == last), view.iter().position(|&x| x == i))
                    {
                        let (a, b) = (a.min(b), a.max(b));
                        for &x in &view[a..=b] {
                            t.sel.insert(x);
                        }
                    }
                }
            } else if ctrl {
                if t.sel.contains(&i) {
                    t.sel.remove(&i);
                } else {
                    t.sel.insert(i);
                }
                t.sel_last = Some(i);
            } else {
                t.sel.clear();
                t.sel.insert(i);
                t.sel_last = Some(i);
            }
        }
        ContentAct::Ctx | ContentAct::EmptyCtx => {
            let a = ui.ctx().input(|inp| inp.pointer.interact_pos()).unwrap_or_default();
            let p = if act == ContentAct::Ctx { Popup::File } else { Popup::Empty };
            w.open_menu(p, a, shared.t());
        }
    }
}

fn header_cell(ui: &mut Ui, r: Rect, label: &str, key: SortKey, w: &WinState, right: bool) -> Option<SortKey> {
    let resp = ui.interact(r, ui.id().with(("hdr", label)), Sense::click());
    let hov = resp.hovered();
    if hov {
        ui.painter().rect_filled(r, 4.0, HOVER);
    }
    let t = w.cur();
    let arrow = if t.sort == key && label != "类型" {
        if t.sort_desc { " ▼" } else { " ▲" }
    } else {
        ""
    };
    ui.painter().text(
        if right {
            Pos2::new(r.max.x - 6.0, r.center().y)
        } else {
            Pos2::new(r.min.x + 30.0, r.center().y)
        },
        if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER },
        format!("{}{}", label, arrow),
        FontId::proportional(11.0),
        if hov { FG } else { MUTE },
    );
    if resp.clicked() && label != "类型" {
        Some(key)
    } else {
        None
    }
}

fn this_pc(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    let mut nav: Option<PathBuf> = None;
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(8.0);
        for (name, p) in shared.drives.iter() {
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 8.0, HOVER);
            }
            ui.painter().rect_filled(
                Rect::from_center_size(Pos2::new(rect.min.x + 28.0, rect.center().y), Vec2::new(22.0, 22.0)),
                5.0,
                GRAY,
            );
            ui.painter().text(
                Pos2::new(rect.min.x + 50.0, rect.center().y - 8.0),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(13.0),
                FG,
            );
            ui.painter().text(
                Pos2::new(rect.min.x + 50.0, rect.center().y + 9.0),
                Align2::LEFT_CENTER,
                shared.free_text(p),
                FontId::proportional(11.0),
                MUTE,
            );
            if resp.clicked() {
                nav = Some(p.clone());
            }
            ui.add_space(4.0);
        }
    });
    if let Some(p) = nav {
        w.cur_mut().navigate(p);
    }
}

// ================================================================ 弹层

fn draw_popups(ctx: &egui::Context, w: &mut WinState, shared: &Arc<Shared>, now: f64) {
    let dy = (1.0 - ease_out(((now - w.popup_at) * 1000.0 / 160.0) as f32)) * 6.0;

    // ---- 右键菜单
    if let Some(popup) = w.popup.clone() {
        let anchor = w.popup_anchor;
        let t = w.cur();
        let has_sel = !t.sel.is_empty();
        let one = t.sel.len() == 1;
        let has_clip =
            !shared.clip.lock().unwrap().0.is_empty() || fs_util::clip_has_files();
        let items: Vec<(String, bool)> = match &popup {
            Popup::Tab { .. } => vec![
                ("在新窗口中打开".into(), true),
                ("复制为新标签".into(), true),
                ("关闭标签".into(), true),
            ],
            Popup::File => vec![
                ("打开".into(), has_sel),
                ("复制路径".into(), has_sel),
                ("在资源管理器中显示".into(), has_sel),
                ("复制".into(), has_sel),
                ("剪切".into(), has_sel),
                ("粘贴".into(), has_clip),
                ("重命名".into(), one),
                ("移动到回收站".into(), has_sel),
                ("新建文件夹".into(), true),
                ("新建文本文档".into(), true),
            ],
            Popup::Empty => vec![
                ("粘贴".into(), has_clip),
                ("新建文件夹".into(), true),
                ("新建文本文档".into(), true),
                ("全选".into(), !t.entries.is_empty()),
                ("刷新".into(), true),
            ],
        };
        let mut clicked: Option<String> = None;
        let out = egui::Window::new("menu")
            .title_bar(false)
            .fixed_pos(anchor + Vec2::new(0.0, dy))
            .pivot(Align2::LEFT_TOP)
            .frame(
                Frame::none()
                    .fill(SURF)
                    .rounding(8.0)
                    .inner_margin(Margin::same(4))
                    .stroke((0.5, LINE2)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(208.0);
                for (label, enabled) in items.iter() {
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
                    let hov = resp.hovered() && *enabled;
                    if hov {
                        ui.painter().rect_filled(rect, 4.0, HOVER);
                    }
                    ui.painter().text(
                        Pos2::new(rect.min.x + 10.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(12.5),
                        if !*enabled { MUTE } else if hov { FG } else { FG2 },
                    );
                    if resp.clicked() && *enabled {
                        clicked = Some(label.clone());
                    }
                }
            });
        w.popup_rect = out.as_ref().map(|o| o.response.rect);
        if let Some(label) = clicked {
            run_menu(ctx, w, shared, &popup, &label);
            w.popup = None;
        }
    }

    // ---- 面包屑下拉
    if let Some(cm) = w.crumb_menu.as_mut() {
        if cm.rx.is_some() {
            let rx = cm.rx.take().unwrap();
            let mut keep = None;
            let mut dirs = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(Ok((_, es))) => {
                        dirs = es.into_iter().filter(|e| e.is_dir).collect();
                    }
                    Ok(Err(_)) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        keep = Some(rx);
                        break;
                    }
                    Err(_) => break,
                }
            }
            if !dirs.is_empty() {
                cm.dirs = dirs;
            }
            cm.rx = keep;
        }
        let anchor = cm.anchor;
        let mut go: Option<PathBuf> = None;
        let out = egui::Window::new("crumbmenu")
            .title_bar(false)
            .fixed_pos(anchor + Vec2::new(0.0, dy))
            .pivot(Align2::LEFT_TOP)
            .frame(
                Frame::none()
                    .fill(SURF)
                    .rounding(8.0)
                    .inner_margin(Margin::same(4))
                    .stroke((0.5, LINE2)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(220.0);
                ui.set_max_height(300.0);
                ScrollArea::vertical().show(ui, |ui| {
                    if cm.dirs.is_empty() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("…").size(12.0).color(MUTE));
                        });
                    }
                    for d in cm.dirs.iter() {
                        let (rect, resp) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 28.0),
                            Sense::click(),
                        );
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, 4.0, HOVER);
                        }
                        ui.painter().rect_filled(
                            Rect::from_center_size(
                                Pos2::new(rect.min.x + 14.0, rect.center().y),
                                Vec2::new(11.0, 11.0),
                            ),
                            3.0,
                            AMBER,
                        );
                        ui.painter().text(
                            Pos2::new(rect.min.x + 32.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            truncate(&d.name, 28),
                            FontId::proportional(12.5),
                            FG2,
                        );
                        if resp.clicked() {
                            go = Some(d.path.clone());
                        }
                    }
                });
            });
        if let Some(o) = &out {
            if w.popup.is_none() {
                w.popup_rect = Some(o.response.rect);
            }
        }
        if let Some(p) = go {
            w.crumb_menu = None;
            w.cur_mut().navigate(p);
        }
    }
}

fn run_menu(ctx: &egui::Context, w: &mut WinState, shared: &Arc<Shared>, p: &Popup, label: &str) {
    match p {
        Popup::Tab { idx } => match label {
            "在新窗口中打开" => {
                if *idx < w.tabs.len() {
                    if w.tabs.len() == 1 {
                        // Files 不允许撕走最后一张：原样不动
                        shared.toast("这是最后一个标签".into(), true);
                    } else {
                        let tab = w.tabs[*idx].take_detached();
                        w.tabs.remove(*idx);
                        if w.ti >= w.tabs.len() {
                            w.ti = w.tabs.len().saturating_sub(1);
                        }
                        w.new_windows.push((tab, None));
                    }
                }
            }
            "复制为新标签" => {
                if *idx < w.tabs.len() {
                    let np = w.tabs[*idx].path.clone();
                    w.tabs.insert(*idx + 1, Tab::new(np));
                    w.ti = *idx + 1;
                }
            }
            "关闭标签" => close_tab(w, *idx),
            _ => {}
        },
        Popup::File => {
            let paths = w.cur().sel_paths();
            match label {
                "打开" => {
                    if let Some(p0) = paths.first() {
                        if p0.is_dir() {
                            let p0 = p0.clone();
                            w.cur_mut().navigate(p0);
                        } else {
                            let _ = open_with_default(p0);
                        }
                    }
                }
                "复制路径" => {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(
                            paths.iter().map(|x| x.to_string_lossy().to_string()).collect::<Vec<_>>().join("\n"),
                        );
                    }
                }
                "在资源管理器中显示" => {
                    if let Some(p0) = paths.first() {
                        let _ = reveal_in_explorer(p0);
                    }
                }
                "复制" => {
                    if let Err(e) = clip_write_files(&paths, false) {
                        shared.toast(format!("写入剪贴板失败：{}", e), true);
                    }
                    *shared.clip.lock().unwrap() = (paths, false);
                }
                "剪切" => {
                    if let Err(e) = clip_write_files(&paths, true) {
                        shared.toast(format!("写入剪贴板失败：{}", e), true);
                    }
                    *shared.clip.lock().unwrap() = (paths, true);
                }
                "粘贴" => {
                    paste(ctx, w, shared);
                }
                "重命名" => start_rename(w, shared.t()),
                "移动到回收站" => {
                    if !paths.is_empty() {
                        do_delete(w, shared, paths);
                    }
                }
                "新建文件夹" => new_item(w, shared, false),
                "新建文本文档" => new_item(w, shared, true),
                _ => {}
            }
        }
        Popup::Empty => match label {
            "粘贴" => paste(ctx, w, shared),
            "新建文件夹" => new_item(w, shared, false),
            "新建文本文档" => new_item(w, shared, true),
            "全选" => {
                let view = w.cur_mut().view();
                let t = w.cur_mut();
                t.sel.clear();
                for &i in view.iter() {
                    t.sel.insert(i);
                }
                t.sel_last = view.last().copied();
            }
            "刷新" => w.cur_mut().reload(),
            _ => {}
        },
    }
}

/// 拖放落点执行：Ctrl 强制复制、Shift 强制移动，否则同盘移动 / 跨盘复制（Explorer 习惯）
fn do_drop(ctx: &egui::Context, w: &mut WinState, shared: &Arc<Shared>, srcs: Vec<PathBuf>, dst: PathBuf) {
    if srcs.is_empty() {
        return;
    }
    // 拖到自己或自己的子目录里 = 无限递归，直接拒
    if srcs.iter().any(|s| dst == *s || dst.starts_with(s)) {
        shared.toast("不能移动到自身或子目录".into(), true);
        return;
    }
    if w.cur().path == dst {
        return; // 原地放下，无意义
    }
    let (ctrl_m, shift_m) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
    let same_vol = srcs.iter().all(|s| same_volume(s, &dst));
    let copy = if ctrl_m {
        true
    } else if shift_m {
        false
    } else {
        !same_vol
    };
    let op = if copy { CopyOp::Copy } else { CopyOp::Move };
    let h = start_job(op, srcs, dst, false);
    shared
        .jobs
        .lock()
        .unwrap()
        .push((if copy { "复制" } else { "移动" }.to_string(), h));
    w.reload_when_idle = true;
}

fn paste(ctx: &egui::Context, w: &mut WinState, shared: &Arc<Shared>) {
    // 系统剪贴板优先：这样「在 Explorer 里 Ctrl+C → 到这里 Ctrl+V」能直接用
    let (srcs, cut) = match clip_read_files() {
        Some(v) => v,
        None => shared.clip.lock().unwrap().clone(),
    };
    if srcs.is_empty() {
        return;
    }
    let cur = w.cur().path.clone();
    if cur.as_os_str().is_empty() {
        shared.toast("请先选择一个文件夹再粘贴".into(), true);
        return;
    }
    let label = if cut { "移动" } else { "复制" };
    let op = if cut { CopyOp::Move } else { CopyOp::Copy };
    let h = start_job(op, srcs, cur, true);
    shared.jobs.lock().unwrap().push((label.to_string(), h));
    // 后台任务结束后要刷新列表，否则粘贴完看着像没生效
    w.reload_when_idle = true;
    // 剪切粘贴 = 移动，Explorer 会把剪贴板清空；不清的话再按一次 Ctrl+V 会去移动已经不存在的源
    if cut {
        clip_clear();
        *shared.clip.lock().unwrap() = (Vec::new(), false);
    }
    let _ = ctx;
}

// ================================================================ toast

fn draw_toasts(ctx: &egui::Context, shared: &Arc<Shared>, now: f64) {
    let items: Vec<(String, bool, f64)> = {
        let v = shared.toasts.lock().unwrap();
        v.iter().map(|t| (t.text.clone(), t.err, t.born)).collect()
    };
    if items.is_empty() {
        return;
    }
    egui::Area::new(Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -16.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                for (text, err, born) in items.iter().rev() {
                    let prog = ease_out(((now - born) * 1000.0 / 250.0) as f32);
                    let dx = (1.0 - prog) * 280.0;
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(320.0, 40.0), Sense::hover());
                    let r = rect.translate(Vec2::new(dx, 0.0));
                    ui.painter().rect_filled(r, 8.0, SURF);
                    ui.painter().rect_stroke(r, 8.0, (0.5, LINE2), StrokeKind::Inside);
                    ui.painter().rect_filled(
                        Rect::from_min_size(Pos2::new(r.min.x + 10.0, r.center().y - 8.0), Vec2::new(3.0, 16.0)),
                        1.5,
                        if *err { RED } else { ACC },
                    );
                    ui.painter().text(
                        Pos2::new(r.min.x + 22.0, r.center().y),
                        Align2::LEFT_CENTER,
                        truncate(text, 38),
                        FontId::proportional(12.0),
                        FG,
                    );
                }
            });
        });
}

// ================================================================ 快捷键

fn shortcuts(ctx: &egui::Context, w: &mut WinState, shared: &Arc<Shared>) {
    let ctrl = |key: Key, shift: bool| {
        ctx.input(|i| i.key_pressed(key) && i.modifiers.command && (i.modifiers.shift == shift))
    };
    let plain = |key: Key| ctx.input(|i| i.key_pressed(key));

    if ctrl(Key::N, false) {
        let p = w.cur().path.clone();
        let mut t = Tab::new(p);
        t.history = w.cur().history.clone();
        t.hi = w.cur().hi;
        w.new_windows.push((t, None));
        return;
    }
    if ctrl(Key::T, false) {
        let p = w.cur().path.clone();
        let idx = w.ti + 1;
        w.tabs.insert(idx, Tab::new(p));
        w.ti = idx;
        return;
    }
    if ctrl(Key::W, false) {
        close_tab(w, w.ti);
        return;
    }
    let nummap = [
        (Key::Num1, 0usize), (Key::Num2, 1), (Key::Num3, 2), (Key::Num4, 3),
        (Key::Num5, 4), (Key::Num6, 5), (Key::Num7, 6), (Key::Num8, 7), (Key::Num9, usize::MAX),
    ];
    for (k, n) in nummap {
        if ctrl(k, false) {
            let i = if n == usize::MAX { w.tabs.len().saturating_sub(1) } else { n };
            if i < w.tabs.len() {
                w.ti = i;
            }
            return;
        }
    }
    if ctx.input(|i| i.modifiers.alt && i.key_pressed(Key::ArrowLeft)) {
        w.cur_mut().back();
        return;
    }
    if ctx.input(|i| i.modifiers.alt && i.key_pressed(Key::ArrowRight)) {
        w.cur_mut().fwd();
        return;
    }
    // 鼠标侧键：下=后退 上=前进（浏览器惯例）
    if ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Extra1)) {
        w.cur_mut().back();
        return;
    }
    if ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Extra2)) {
        w.cur_mut().fwd();
        return;
    }
    if plain(Key::Backspace) {
        if let Some(p) = w.cur().path.parent().filter(|x| !x.as_os_str().is_empty()).map(|p| p.to_path_buf()) {
            w.cur_mut().navigate(p);
        }
        return;
    }
    if plain(Key::F5) {
        w.cur_mut().reload();
        return;
    }
    if ctrl(Key::F, false) {
        w.focus_search = true;
        return;
    }
    if plain(Key::Escape) {
        let t = w.cur_mut();
        if !t.query.is_empty() {
            t.set_query(String::new()); // 必须走 set_query，否则过滤视图不刷新
        } else {
            t.sel.clear();
            t.sel_last = None;
        }
        return;
    }
    if ctrl(Key::C, false) {
        let p = w.cur().sel_paths();
        if !p.is_empty() {
            // 同时写系统剪贴板：能直接粘到 Explorer / 微信 / 桌面
            if let Err(e) = clip_write_files(&p, false) {
                shared.toast(format!("写入剪贴板失败：{}", e), true);
            }
            *shared.clip.lock().unwrap() = (p, false);
        }
        return;
    }
    if ctrl(Key::X, false) {
        let p = w.cur().sel_paths();
        if !p.is_empty() {
            if let Err(e) = clip_write_files(&p, true) {
                shared.toast(format!("写入剪贴板失败：{}", e), true);
            }
            *shared.clip.lock().unwrap() = (p, true);
        }
        return;
    }
    if ctrl(Key::V, false) {
        paste(ctx, w, shared);
        return;
    }
    if ctrl(Key::A, false) {
        let view = w.cur_mut().view();
        let t = w.cur_mut();
        t.sel.clear();
        for &i in view.iter() {
            t.sel.insert(i);
        }
        return;
    }
    if plain(Key::F2) {
        if w.cur().sel.len() == 1 {
            start_rename(w, shared.t());
        }
        return;
    }
    if plain(Key::Delete) {
        let p = w.cur().sel_paths();
        if !p.is_empty() {
            do_delete(w, shared, p);
        }
        return;
    }
    if plain(Key::Enter) {
        let sel: Vec<usize> = w.cur().sel.iter().cloned().collect();
        if sel.len() == 1 {
            if let Some(e) = w.cur().entries.get(sel[0]) {
                let p = e.path.clone();
                let is_dir = e.is_dir;
                if is_dir {
                    w.cur_mut().navigate(p);
                } else {
                    let _ = open_with_default(&p);
                }
            }
        }
        return;
    }
    if plain(Key::ArrowDown) || plain(Key::ArrowUp) {
        let down = plain(Key::ArrowDown);
        let view = w.cur_mut().view();
        if !view.is_empty() {
            let cur = w.cur().sel.iter().cloned().max();
            let pos = cur
                .and_then(|s| view.iter().position(|&x| x == s))
                .map(|p| if down { (p + 1).min(view.len() - 1) } else { p.saturating_sub(1) })
                .unwrap_or(0);
            let i = view[pos];
            let shift = ctx.input(|i| i.modifiers.shift);
            let t = w.cur_mut();
            if !shift {
                t.sel.clear();
            }
            t.sel.insert(i);
            t.sel_last = Some(i);
        }
    }
}

// ================================================================ 操作

fn do_delete(w: &mut WinState, shared: &Arc<Shared>, paths: Vec<PathBuf>) {
    let n = paths.len();
    match delete_to_recycle(&paths) {
        Ok(()) => {
            shared.toast(format!("已将 {} 项移至回收站", n), false);
            w.cur_mut().reload();
        }
        Err(e) => shared.toast(format!("删除失败：{}", e), true),
    }
}

fn start_rename(w: &mut WinState, now: f64) {
    if let Some(p) = w.cur().sel_paths().first().cloned() {
        w.rename = Some(Rename {
            target: p.clone(),
            buf: short_name(&p),
            focus_once: true,
            born: now,
        });
    }
}

fn accent_btn(ui: &mut Ui, label: &str, primary: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(72.0, 28.0), Sense::click());
    let press = resp.is_pointer_button_down_on();
    let hover_c = Color32::from_rgb(96, 168, 255);
    let fill = if primary {
        if press { PR } else if resp.hovered() { hover_c } else { ACC }
    } else {
        if press || resp.hovered() { SURF } else { BG2 }
    };
    let scale = if press { 0.97 } else { 1.0 };
    let r2 = Rect::from_center_size(rect.center(), rect.size() * scale);
    ui.painter().rect_filled(r2, 6.0, fill);
    ui.painter().text(
        r2.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.5),
        if primary { Color32::WHITE } else { FG2 },
    );
    resp
}

fn draw_rename(ctx: &egui::Context, w: &mut WinState, shared: &Arc<Shared>, now: f64) {
    let _ = now;
    let mut ok = false;
    let mut cancel = false;
    egui::Window::new("rename")
        .title_bar(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .frame(
            Frame::none()
                .fill(SURF)
                .rounding(12.0)
                .inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 16 })
                .stroke((0.5, LINE2)),
        )
        .show(ctx, |ui| {
            ui.set_min_width(340.0);
            ui.label(RichText::new("重命名").size(15.0).color(FG));
            ui.add_space(10.0);
            if let Some(rn) = w.rename.as_mut() {
                let resp = ui.add(
                    TextEdit::singleline(&mut rn.buf)
                        .desired_width(300.0)
                        .font(TextStyle::Body),
                );
                if rn.focus_once {
                    resp.request_focus();
                    rn.focus_once = false;
                }
            }
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                ok = true;
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if accent_btn(ui, "确定", true).clicked() {
                    ok = true;
                }
                if accent_btn(ui, "取消", false).clicked() {
                    cancel = true;
                }
            });
        });
    if cancel {
        w.rename = None;
    }
    if ok {
        let rn = w.rename.take();
        if let Some(rn) = rn {
            if !rn.buf.is_empty() && rn.buf != short_name(&rn.target) {
                // 文件名里不能带路径分隔符或非法字符，"." ".." 也不行：
                // 否则 p.join("..\\x") 会把文件挪到父目录，且 Windows 的 rename 会覆盖同名文件
                let bad = rn.buf.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|'])
                    || rn.buf == "."
                    || rn.buf == "..";
                if bad {
                    shared.toast("名称里不能包含 \\ / : * ? \" < > |".into(), true);
                    return;
                }
                if let Some(dst) = rn.target.parent().map(|p| p.join(&rn.buf)) {
                    if dst != rn.target && dst.exists() {
                        shared.toast("同名文件已存在，未做修改".into(), true);
                        return;
                    }
                    match std::fs::rename(&rn.target, &dst) {
                        Ok(()) => {
                            shared.toast("已重命名".into(), false);
                            w.cur_mut().reload();
                        }
                        Err(e) => shared.toast(format!("重命名失败：{}", e), true),
                    }
                }
            }
        }
    }
}

fn new_item(w: &mut WinState, shared: &Arc<Shared>, is_file: bool) {
    let dir = w.cur().path.clone();
    if dir.as_os_str().is_empty() {
        shared.toast("请先选择一个文件夹".into(), true);
        return;
    }
    let (p, res) = if is_file {
        let p = unique_child(&dir, "新建文本文档", "txt");
        (p.clone(), std::fs::write(&p, ""))
    } else {
        let p = unique_child(&dir, "新建文件夹", "");
        (p.clone(), std::fs::create_dir(&p).map(|_| ()))
    };
    match res {
        Ok(()) => {
            w.cur_mut().reload();
            let name = short_name(&p);
            w.rename = Some(Rename { target: p, buf: name, focus_once: true, born: shared.t() });
        }
        Err(e) => shared.toast(format!("创建失败：{}", e), true),
    }
}

// ================================================================ 字体 & 入口

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for name in ["msyh.ttc", "msyh.ttf", "Deng.ttf", "simhei.ttf", "simsun.ttc"] {
        let p = format!("C:\\Windows\\Fonts\\{}", name);
        if let Ok(bytes) = std::fs::read(&p) {
            let mut fd = FontData::from_owned(bytes);
            if name.ends_with(".ttc") {
                fd.index = 0;
            }
            fonts.font_data.insert("cjk".into(), Arc::new(fd));
            for fam in [FontFamily::Proportional, FontFamily::Monospace] {
                if let Some(list) = fonts.families.get_mut(&fam) {
                    list.push("cjk".into());
                }
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

/// 应用图标：assets/icon_256.rgba 是 256×256 未预乘 RGBA 裸字节（tools/gen_icon.py 生成）。
/// egui 0.31 没有 png 解码入口，直接塞裸字节最省事、零依赖。
/// 用 OnceLock 缓存：子窗口 builder 每帧都会重建，不能每帧拷 256 KB。
fn app_icon() -> Arc<egui::IconData> {
    static CACHE: std::sync::OnceLock<Arc<egui::IconData>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            const W: u32 = 256;
            let rgba: &[u8] = include_bytes!("../assets/icon_256.rgba");
            Arc::new(if rgba.len() as u32 == W * W * 4 {
                egui::IconData {
                    rgba: rgba.to_vec(),
                    width: W,
                    height: W,
                }
            } else {
                egui::IconData::default()
            })
        })
        .clone()
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([1100.0, 680.0])
            .with_min_inner_size([680.0, 420.0])
            .with_decorations(false)
            .with_icon(app_icon())
            .with_title("RustExplorer"),
        ..Default::default()
    };
    eframe::run_native(
        "RustExplorer",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

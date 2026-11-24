#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! RustExplorer — Files 式多窗口文件管理器
//! 结构：单进程(共享剪贴板/任务/toast) → 原生多窗口 → 窗口内标签页。
//! 标签可拖出窗口撕成新原生窗口；无宫格，一窗一内容区。

mod fs_util;
mod view;

use eframe::egui::{self, Align, Align2, Color32, FontDefinitions, FontData, FontFamily, FontId,
    Frame, Id, Key, Layout, Margin, Order, Pos2, Rect, ResizeDirection, Response, RichText,
    ScrollArea, Sense, StrokeKind, TextEdit, TextStyle, TopBottomPanel, Ui, Vec2, ViewportBuilder,
    ViewportClass, ViewportCommand, ViewportId, Visuals};
use fs_util::{
    clip_clear, clip_read_files, clip_write_files, delete_to_recycle, drive_free, drive_space, drives,
    fmt_size, fmt_time, home_dir, open_with_default, reveal_in_explorer, same_volume, short_name,
    spawn_ls, start_job, unique_child, CopyOp, FsEntry, JobHandle, LsResult,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ================================================================ 设计 tokens
// 现代 Win11 资源管理器观感：三层底色（内容白 / 侧栏浅灰 / 详情淡蓝白），
// 一律 1px 发丝线分隔、不再堆卡片描边；强调色取 Win11 系统蓝。

const BG: Color32 = Color32::from_rgb(250, 250, 251); // 侧栏 / 窗口底
const BG2: Color32 = Color32::from_rgb(243, 244, 246); // 标题条
const SURF: Color32 = Color32::from_rgb(255, 255, 255); // 内容区 / 输入框
const PANE: Color32 = Color32::from_rgb(247, 249, 251); // 详情面板 / 状态栏
const LINE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 16); // 黑 6%
const LINE2: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 32); // 黑 12.5%
/// 面板分隔线用「不透明」浅灰：半透明线画在面板接缝（那一行像素是窗口清屏色）上会发黑
const SEP: Color32 = Color32::from_rgb(229, 230, 233);
const SEP2: Color32 = Color32::from_rgb(215, 217, 221);
const FG: Color32 = Color32::from_rgb(24, 25, 28);
const FG2: Color32 = Color32::from_rgb(80, 84, 90);
const MUTE: Color32 = Color32::from_rgb(136, 140, 146);
const ACC: Color32 = Color32::from_rgb(0, 103, 192); // Win11 蓝
// accent(0,103,192) @ 12.5% → 预乘 (0,12,23,32)
const ACC_BG: Color32 = Color32::from_rgba_premultiplied(0, 12, 23, 32);
const HOVER: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 12); // 黑 5%
const HOVER2: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 26); // 黑 10%
const PR: Color32 = Color32::from_rgb(0, 84, 158); // accent 按下

const AMBER: Color32 = Color32::from_rgb(233, 164, 62); // 文件夹 / 磁盘
const GREEN: Color32 = Color32::from_rgb(29, 158, 117);
const PINK: Color32 = Color32::from_rgb(212, 83, 126);
const PURPLE: Color32 = Color32::from_rgb(127, 119, 221);
const TEAL: Color32 = Color32::from_rgb(45, 156, 219);
const GRAY: Color32 = Color32::from_rgb(136, 135, 128);
const RED: Color32 = Color32::from_rgb(226, 75, 74);

const TAB_H: f32 = 34.0; // 标签 chip 高
const TITLE_H: f32 = 44.0; // 标题条整体高
const CMD_H: f32 = 46.0; // 命令条高
const ROW_H: f32 = 30.0;
const HEAD_H: f32 = 28.0;
const SB_H: f32 = 26.0;
const SIDE_W: f32 = 210.0;
const DETAIL_W: f32 = 276.0;

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
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "ico" => GREEN,
        "doc" | "docx" | "pdf" | "md" | "txt" | "log" => PINK,
        "zip" | "rar" | "7z" | "gz" | "tar" => PURPLE,
        "mp4" | "mkv" | "avi" | "mov" => AMBER,
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
            "msi" => "安装包",
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "ico" => "图片",
            "txt" | "log" => "文本",
            "md" => "Markdown",
            "zip" | "rar" | "7z" | "gz" | "tar" => "压缩包",
            "pdf" => "PDF",
            "doc" | "docx" => "Word 文档",
            "xls" | "xlsx" | "csv" => "电子表格",
            "ppt" | "pptx" => "演示文稿",
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" => "视频",
            "mp3" | "wav" | "flac" | "m4a" => "音频",
            "rs" => "Rust",
            "py" => "Python",
            "js" | "ts" => "脚本",
            "html" | "htm" => "网页",
            "json" | "toml" | "yaml" | "yml" | "ini" => "配置",
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

/// 从左截断（保留尾部）：路径类信息尾部才是关键
fn truncate_left(s: &str, max_w: f32) -> String {
    if estimate_width(s) <= max_w {
        return s.to_string();
    }
    let budget = (max_w - 10.0).max(0.0);
    let mut acc = 0.0f32;
    let mut out: Vec<char> = Vec::new();
    for ch in s.chars().rev() {
        let cw = if (ch as u32) > 0x2E80 { 12.5 } else { 6.8 };
        if acc + cw > budget {
            break;
        }
        acc += cw;
        out.push(ch);
    }
    out.reverse();
    let mut r = String::from("…");
    r.extend(out);
    r
}

// ================================================================ 模型

#[derive(PartialEq, Clone, Copy)]
enum SortKey {
    Name,
    Date,
    Size,
    Type,
}

/// 列表里的一行：分组表头（文件夹 / 文件）或真实条目。
/// 下标指向 entries，分组表头里放的是该组条目数。
#[derive(Clone, Copy, PartialEq)]
enum RowKind {
    DirHeader,
    FileHeader,
    Item,
}

#[derive(Clone, Copy)]
struct RowRef {
    kind: RowKind,
    idx: usize,
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
    /// 带分组表头的显示行（= view + 2 个表头，折叠时条目被剔除）
    rows: Arc<Vec<RowRef>>,
    view_dirty: bool,
    /// 分组折叠态（文件夹 / 文件两个区块）
    fold_dirs: bool,
    fold_files: bool,
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
            rows: Arc::new(Vec::new()),
            view_dirty: true,
            fold_dirs: false,
            fold_files: false,
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
            rows: Arc::new(Vec::new()),
            view_dirty: true,
            fold_dirs: self.fold_dirs,
            fold_files: self.fold_files,
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
        // hi 初始是 usize::MAX（还没有历史），直接 +1 会溢出
        if self.hi < self.history.len().saturating_sub(1) {
            self.hi += 1;
            let p = self.history[self.hi].clone();
            self.path = p;
            self.reload();
        }
    }

    fn can_back(&self) -> bool {
        self.hi > 0 && self.hi <= self.history.len()
    }
    fn can_fwd(&self) -> bool {
        self.hi < self.history.len().saturating_sub(1)
    }

    /// 过滤 + 排序后的 entry 下标序列
    fn view(&mut self) -> Arc<Vec<usize>> {
        if self.view_dirty {
            self.rebuild_view();
        }
        self.view.clone()
    }

    /// 带分组表头的显示行
    fn rows(&mut self) -> Arc<Vec<RowRef>> {
        if self.view_dirty {
            self.rebuild_view();
        }
        self.rows.clone()
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

        // 分组：文件夹块 + 文件块。视图已保证「目录在前」，这里只需要切一刀。
        let n_dir = v.iter().filter(|&&i| es[i].is_dir).count();
        let n_file = v.len() - n_dir;
        let mut rows: Vec<RowRef> = Vec::with_capacity(v.len() + 2);
        if n_dir > 0 {
            rows.push(RowRef { kind: RowKind::DirHeader, idx: n_dir });
            if !self.fold_dirs {
                for &i in v.iter().take(n_dir) {
                    rows.push(RowRef { kind: RowKind::Item, idx: i });
                }
            }
        }
        if n_file > 0 {
            rows.push(RowRef { kind: RowKind::FileHeader, idx: n_file });
            if !self.fold_files {
                for &i in v.iter().skip(n_dir) {
                    rows.push(RowRef { kind: RowKind::Item, idx: i });
                }
            }
        }
        self.rows = Arc::new(rows);
        self.view = Arc::new(v);
        self.view_dirty = false;
    }

    /// 当前真正显示出来的条目下标（折叠/搜索后）——全选、方向键要用这个
    fn visible(&mut self) -> Vec<usize> {
        let rows = self.rows();
        rows.iter()
            .filter(|r| r.kind == RowKind::Item)
            .map(|r| r.idx)
            .collect()
    }

    fn set_query(&mut self, q: String) {
        if self.query != q {
            self.query = q;
            self.view_dirty = true;
            // 选中是按 entries 下标存的，过滤后同一行会指向别的文件 —— 直接清掉
            self.sel.clear();
            self.sel_last = None;
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
    /// 命令条上的排序菜单
    Sort,
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
    /// 拿到焦点后预选中文件名主体（不含扩展名），像 Explorer 一样直接打字替换
    sel_once: bool,
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
    /// 搜索框是否持有焦点（上一帧）。
    /// 不能用「有任意控件获焦」判断是否在输入：egui 的 Tab 导航会把焦点给到文件行/按钮，
    /// 那样快捷键就全废了。只认真正的文本输入。
    search_focus: bool,
    close_requested: bool,
    new_windows: Vec<(Tab, Option<[f32; 2]>)>,
    maximized: bool,
    /// 右侧详情面板（Ctrl+I）
    details: bool,
    /// 地址栏直接输入路径（Ctrl+L 进入）：Some(文本) 时面包屑切成输入框
    addr: Option<String>,
    addr_focus: bool,
    /// 待全选地址文本（等输入框真正拿到焦点的下一帧再设选区，否则会被控件自身的状态覆盖）
    addr_sel: bool,
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
            search_focus: false,
            close_requested: false,
            new_windows: Vec::new(),
            maximized: false,
            details: true,
            addr: None,
            addr_focus: false,
            addr_sel: false,
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

/// 磁盘容量快照（用于侧栏/此电脑/详情面板的容量条）
#[derive(Clone, Default)]
struct SpaceInfo {
    free: u64,
    total: u64,
    frac: f32, // 已用比例
}

struct Shared {
    clip: Mutex<(Vec<PathBuf>, bool)>,
    jobs: Mutex<Vec<(String, JobHandle)>>,
    toasts: Mutex<Vec<Toast>>,
    drives: Vec<(String, PathBuf)>,
    quick: Vec<(String, PathBuf)>,
    start: std::time::Instant,
    /// 盘符容量缓存（路径 → (快照, 采样时间)）。
    /// GetDiskFreeSpaceExW 是同步调用，休眠的移动硬盘 / 断线的网络盘会把 UI 卡死几秒，
    /// 所以每个盘 5 秒内只查一次，绝不能放在绘制循环里每帧调。
    space_cache: Mutex<std::collections::HashMap<PathBuf, (SpaceInfo, f64)>>,
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
            space_cache: Mutex::new(std::collections::HashMap::new()),
        }
    }
    fn t(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
    /// 盘符容量；5 秒缓存，避免每帧一次同步系统调用
    fn space(&self, p: &std::path::Path) -> SpaceInfo {
        let t = self.t();
        {
            let c = self.space_cache.lock().unwrap();
            if let Some((s, at)) = c.get(p) {
                if t - at < 5.0 {
                    return s.clone();
                }
            }
        }
        let s = match drive_space(p) {
            Some((free, total)) => SpaceInfo {
                free,
                total,
                frac: if total > 0 { 1.0 - (free as f32 / total as f32) } else { 0.0 },
            },
            None => SpaceInfo::default(),
        };
        self.space_cache.lock().unwrap().insert(p.to_path_buf(), (s.clone(), t));
        s
    }
    /// 「可用 12.3 GB」（与「此电脑」视图里的用词保持一致）
    fn free_text(&self, p: &std::path::Path) -> String {
        let s = self.space(p);
        if s.total == 0 {
            String::new()
        } else {
            format!("可用 {}", fmt_size(s.free))
        }
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
    fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
        // 面板分界处偶尔有 1px 没被填充：清屏色给白，接缝就不会发黑
        [1.0, 1.0, 1.0, 1.0]
    }

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
    // 只有「真的在输入文字」才屏蔽快捷键（重命名框 / 搜索框 / 地址栏）。
    // 用 focused().is_some() 会把 Tab 导航到的行、按钮也算进来，导致快捷键莫名失效。
    let editing = w.rename.is_some() || w.search_focus || w.addr.is_some();

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

    // ---- 标题条（标签 + 搜索 + 窗口控制钮；无原生标题栏）
    TopBottomPanel::top("titlebar")
        .exact_height(TITLE_H)
        .frame(Frame::none().fill(BG2).inner_margin(Margin { left: 8, right: 0, top: 6, bottom: 0 }))
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
            (Some(_), None) => true,
            _ => false,
        });
        if outside && w.tabs.len() > 1 {
            // 撕出：tab 状态整体搬进新窗口，新窗在松手点出现（空间一致性）
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

    // ---- 命令条（导航 + 地址栏 + 操作）
    TopBottomPanel::top("cmdbar")
        .exact_height(CMD_H)
        .frame(Frame::none().fill(BG).inner_margin(Margin { left: 10, right: 10, top: 9, bottom: 9 }))
        .show(ctx, |ui| {
            command_bar(ui, w, shared);
        });

    // ---- 状态栏
    TopBottomPanel::bottom("statusbar")
        .exact_height(SB_H)
        .frame(Frame::none().fill(PANE).inner_margin(Margin { left: 14, right: 14, top: 4, bottom: 3 }))
        .show(ctx, |ui| {
            status_bar(ui, w, shared);
        });

    // ---- 右侧详情面板
    if w.details {
        egui::SidePanel::right("details")
            .exact_width(DETAIL_W)
            .frame(Frame::none().fill(PANE))
            .show(ctx, |ui| {
                details_pane(ui, w, shared);
            });
    }

    // ---- 侧栏
    if w.sidebar {
        egui::SidePanel::left("sidebar")
            .exact_width(SIDE_W)
            .frame(Frame::none().fill(BG).inner_margin(Margin { left: 8, right: 8, top: 8, bottom: 8 }))
            .show(ctx, |ui| {
                sidebar(ui, w, shared);
            });
    }

    // ---- 内容区
    egui::CentralPanel::default()
        .frame(Frame::none().fill(SURF))
        .show(ctx, |ui| {
            content(ui, w, shared);
        });

    // ---- 面板交界的发丝线（画在 Middle 层：压住面板底色，又不遮弹层）
    chrome_lines(ctx, w);

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
        view::soft_shadow(&pt, r, 6.0);
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
            pt.rect_filled(t.rect.expand(1.0), 6.0, ACC_BG);
            pt.rect_stroke(t.rect.expand(1.0), 6.0, (1.5, ACC), StrokeKind::Inside);
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
                        .corner_radius(7.0)
                        .inner_margin(Margin::symmetric(10, 5))
                        .stroke((0.5, LINE2))
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 4],
                            blur: 12,
                            spread: 0,
                            color: Color32::from_rgba_unmultiplied(0, 0, 0, 30),
                        })
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

/// 面板交界的 1px 线：不用给每个面板加描边，统一画在这里
fn chrome_lines(ctx: &egui::Context, w: &WinState) {
    let scr = ctx.screen_rect();
    if scr.width() < 10.0 {
        return;
    }
    let p = ctx.layer_painter(egui::LayerId::new(Order::Middle, Id::new(("chrome", w.id))));
    let y1 = TITLE_H;
    let y2 = TITLE_H + CMD_H;
    let yb = scr.max.y - SB_H;
    // 宽度用 1.0 + 不透明色：面板接缝处常有 1px 未被填充的清屏色，半透明线会把它透出来
    for y in [y1, y2, yb] {
        p.line_segment(
            [Pos2::new(scr.min.x, y), Pos2::new(scr.max.x, y)],
            egui::Stroke::new(1.0, SEP),
        );
    }
    if w.sidebar {
        p.line_segment(
            [Pos2::new(scr.min.x + SIDE_W, y2 + 1.0), Pos2::new(scr.min.x + SIDE_W, yb)],
            egui::Stroke::new(1.0, SEP),
        );
    }
    if w.details {
        p.line_segment(
            [Pos2::new(scr.max.x - DETAIL_W - 1.0, y2 + 1.0), Pos2::new(scr.max.x - DETAIL_W - 1.0, yb)],
            egui::Stroke::new(1.0, SEP),
        );
    }
}

/// 标题条里的应用图标纹理（256×256 裸 RGBA，缓存一次）
fn app_icon_texture(ctx: &egui::Context) -> Option<&'static egui::TextureHandle> {
    static TEX: std::sync::OnceLock<egui::TextureHandle> = std::sync::OnceLock::new();
    if let Some(t) = TEX.get() {
        return Some(t);
    }
    const W: usize = 256;
    let rgba: &[u8] = include_bytes!("../assets/icon_256.rgba");
    if rgba.len() != W * W * 4 {
        return None;
    }
    let img = egui::ColorImage::from_rgba_unmultiplied([W, W], rgba);
    let h = ctx.load_texture("app-icon", img, egui::TextureOptions::LINEAR);
    let _ = TEX.set(h);
    TEX.get()
}

// ================================================================ 标题条

enum TabEv {
    DragMove,
    Release(usize, bool), // (tab idx, 点在关闭按钮上)
    Menu(usize),
    Plus,
}

fn tab_bar(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    let x0 = ui.cursor().min.x;
    let y = ui.cursor().min.y + 4.0;
    let right = ui.max_rect().max.x;
    let chip_y = y;

    // 应用图标
    let icon_r = Rect::from_center_size(Pos2::new(x0 + 9.0, chip_y + TAB_H / 2.0), Vec2::splat(18.0));
    if let Some(tex) = app_icon_texture(ui.ctx()) {
        ui.painter().image(
            tex.id(),
            icon_r,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let tabs_x0 = icon_r.max.x + 12.0;

    // ---- 右侧：搜索框 + 窗口控制钮
    const WB_W: f32 = 44.0;
    const WB_H: f32 = 30.0;
    let close_r = Rect::from_min_size(Pos2::new(right - WB_W, chip_y), Vec2::new(WB_W, WB_H));
    let max_r = Rect::from_min_size(Pos2::new(right - WB_W * 2.0, chip_y), Vec2::new(WB_W, WB_H));
    let min_r = Rect::from_min_size(Pos2::new(right - WB_W * 3.0, chip_y), Vec2::new(WB_W, WB_H));

    let show_search = right - tabs_x0 > 460.0;
    let search_w = 216.0f32.min((right - tabs_x0) * 0.34).max(150.0);
    let search_r = Rect::from_min_size(
        Pos2::new(min_r.min.x - 10.0 - search_w, chip_y),
        Vec2::new(search_w, WB_H),
    );
    let mut q = w.cur().query.clone();
    let mut search_resp: Option<Response> = None;
    if show_search {
        let focused = w.search_focus;
        let inner = view::field_rect(ui.painter(), search_r, focused);
        view::search_glyph(
            ui.painter(),
            Pos2::new(search_r.min.x + 14.0, search_r.center().y),
            13.0,
            MUTE,
        );
        let te = Rect::from_min_max(
            Pos2::new(inner.min.x + 22.0, search_r.min.y + 4.0),
            Pos2::new(search_r.max.x - 8.0, search_r.max.y - 4.0),
        );
        let resp = ui.put(
            te,
            TextEdit::singleline(&mut q)
                .frame(false)
                .hint_text(RichText::new("搜索").size(12.0).color(MUTE))
                .font(TextStyle::Body)
                .desired_width(te.width()),
        );
        if w.focus_search {
            resp.request_focus();
            w.focus_search = false;
        }
        if !q.is_empty() {
            // 清空按钮
            let cr = Rect::from_center_size(Pos2::new(search_r.max.x - 13.0, search_r.center().y), Vec2::splat(18.0));
            let hov = ui.rect_contains_pointer(cr);
            if hov {
                ui.painter().rect_filled(cr, 4.0, HOVER2);
            }
            view::cross_glyph(ui.painter(), cr.center(), 10.0, if hov { FG } else { MUTE });
            if ui.interact(cr, ui.id().with("searchclr"), Sense::click()).clicked() {
                q.clear();
                w.focus_search = true;
            }
        }
        search_resp = Some(resp);
    }

    let tabs_limit = if show_search { search_r.min.x - 10.0 } else { min_r.min.x - 10.0 };

    // ---- 标签 chip 布局
    let mut rects: Vec<(usize, Rect)> = Vec::new();
    let mut x = tabs_x0;
    for (i, t) in w.tabs.iter().enumerate() {
        let tw = (estimate_width(&t.title()) + 64.0).clamp(112.0, 196.0);
        if x + tw > tabs_limit {
            break;
        }
        rects.push((i, Rect::from_min_size(Pos2::new(x, chip_y), Vec2::new(tw, TAB_H))));
        x += tw + 3.0;
    }
    let hidden = w.tabs.len() - rects.len();
    w.tab_rects = rects.clone();

    let plus_rect = Rect::from_min_size(
        Pos2::new(x + if hidden > 0 { 34.0 } else { 2.0 }, chip_y + (TAB_H - 26.0) / 2.0),
        Vec2::new(26.0, 26.0),
    );

    let drag_idx = w.tab_drag.as_ref().map(|d| d.idx);
    let ptr = ui.input(|i| i.pointer.hover_pos());

    for (i, r) in rects.iter() {
        let is_cur = *i == w.ti;
        let is_drag = drag_idx == Some(*i);
        let hov = ptr.map(|p| r.contains(p)).unwrap_or(false);
        if is_cur {
            ui.painter().rect_filled(*r, 7.0, SURF);
            ui.painter().rect_stroke(*r, 7.0, (1.0, LINE2), StrokeKind::Inside);
        } else if hov && !is_drag {
            ui.painter().rect_filled(*r, 7.0, HOVER);
        }
        if is_drag {
            ui.painter().rect_stroke(*r, 7.0, (1.0, LINE2), StrokeKind::Inside);
        }
        let t = &w.tabs[*i];
        // 标签图标：根目录画磁盘，其余画文件夹（按 tint 上色）
        let g = Rect::from_center_size(Pos2::new(r.min.x + 15.0, r.center().y), Vec2::splat(13.0));
        let tint = tab_tint(t);
        if tint == GRAY {
            view::drive_glyph(ui.painter(), g, tint);
        } else {
            view::folder_glyph(ui.painter(), g, tint);
        }
        let name = t.title();
        let loadname = format!("{}…", name);
        let display = truncate(if t.loading { &loadname } else { &name }, 20);
        let name_max = r.width() - 30.0 - 24.0;
        ui.painter().text(
            Pos2::new(r.min.x + 26.0, r.center().y),
            Align2::LEFT_CENTER,
            truncate_width(&display, name_max),
            FontId::proportional(12.0),
            if is_drag { MUTE } else if is_cur { FG } else { FG2 },
        );
        // 关闭按钮
        let cr = Rect::from_center_size(Pos2::new(r.max.x - 14.0, r.center().y), Vec2::splat(18.0));
        let chov = ptr.map(|p| cr.contains(p)).unwrap_or(false);
        if chov {
            ui.painter().rect_filled(cr, 4.0, HOVER2);
        }
        view::cross_glyph(ui.painter(), cr.center(), 11.0, if chov { FG } else { MUTE });
        // 当前标签：底部一点点强调色，跟 Argus 的页签指示条一个意思
        if is_cur {
            ui.painter().rect_filled(
                Rect::from_min_size(Pos2::new(r.min.x + 16.0, r.max.y - 1.5), Vec2::new(r.width() - 32.0, 1.5)),
                1.0,
                ACC,
            );
        }
    }
    if hidden > 0 {
        ui.painter().text(
            Pos2::new(x + 6.0, chip_y + TAB_H / 2.0),
            Align2::LEFT_CENTER,
            format!("+{}", hidden),
            FontId::proportional(10.5),
            MUTE,
        );
    }

    // ＋
    let phov = ptr.map(|p| plus_rect.contains(p)).unwrap_or(false);
    if phov {
        ui.painter().rect_filled(plus_rect, 6.0, HOVER);
    }
    view::plus_glyph(ui.painter(), plus_rect.center(), 13.0, if phov { FG } else { FG2 });

    // ---- 窗口控制钮
    w.maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    let mon_hov = ptr.map(|p| min_r.contains(p)).unwrap_or(false);
    let mx_hov = ptr.map(|p| max_r.contains(p)).unwrap_or(false);
    let cl_hov = ptr.map(|p| close_r.contains(p)).unwrap_or(false);
    if mon_hov {
        ui.painter().rect_filled(min_r.shrink2(Vec2::new(2.0, 1.0)), 6.0, HOVER2);
    }
    if mx_hov {
        ui.painter().rect_filled(max_r.shrink2(Vec2::new(2.0, 1.0)), 6.0, HOVER2);
    }
    if cl_hov {
        ui.painter()
            .rect_filled(close_r.shrink2(Vec2::new(2.0, 1.0)), 6.0, Color32::from_rgb(232, 17, 35));
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
    ui.painter().line_segment(
        [Pos2::new(min_r.center().x - 5.0, cy), Pos2::new(min_r.center().x + 5.0, cy)],
        (1.0, gcol(mon_hov, false)),
    );
    if w.maximized {
        ui.painter().rect_stroke(
            Rect::from_min_size(Pos2::new(max_r.center().x - 3.5, cy - 4.5), Vec2::new(8.0, 8.0)),
            1.0,
            (1.0, gcol(mx_hov, false)),
            StrokeKind::Middle,
        );
        ui.painter().line_segment(
            [Pos2::new(max_r.center().x - 4.5, cy - 3.0), Pos2::new(max_r.center().x + 3.5, cy - 3.0)],
            (1.0, gcol(mx_hov, false)),
        );
    } else {
        ui.painter().rect_stroke(
            Rect::from_center_size(max_r.center(), Vec2::new(9.0, 9.0)),
            1.0,
            (1.0, gcol(mx_hov, false)),
            StrokeKind::Middle,
        );
    }
    let cc = close_r.center();
    ui.painter().line_segment(
        [Pos2::new(cc.x - 4.5, cc.y - 4.5), Pos2::new(cc.x + 4.5, cc.y + 4.5)],
        (1.0, gcol(cl_hov, true)),
    );
    ui.painter().line_segment(
        [Pos2::new(cc.x - 4.5, cc.y + 4.5), Pos2::new(cc.x + 4.5, cc.y - 4.5)],
        (1.0, gcol(cl_hov, true)),
    );

    // ---- 交互收集
    let mut evs: Vec<TabEv> = Vec::new();
    let ctx = ui.ctx();
    for (i, r) in rects.iter() {
        let resp = ui.interact(*r, ui.id().with(("tabchip", *i)), Sense::click_and_drag());
        let xr = Rect::from_center_size(Pos2::new(r.max.x - 14.0, r.center().y), Vec2::splat(18.0));
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
    // 标签栏空白 = 拖动区（双击 = 最大化切换）
    // 右边界必须止于搜索框左侧，否则这个 click_and_drag 会盖住搜索框抢走点击（注册顺序在后 = 优先命中）
    let empty_x0 = plus_rect.max.x + 6.0;
    let drag_r = Rect::from_min_size(
        Pos2::new(empty_x0, y),
        Vec2::new((tabs_limit - empty_x0).max(0.0), TAB_H),
    );
    let drag_resp = ui.interact(drag_r, ui.id().with("tbardrag"), Sense::click_and_drag());
    if drag_resp.drag_started() {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag_resp.double_clicked() {
        do_max = true;
    }
    if do_min {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
    if do_max {
        let nm = !w.maximized;
        ctx.send_viewport_cmd(ViewportCommand::Maximized(nm));
        w.maximized = nm;
    }
    if do_close {
        w.close_requested = true;
        return;
    }

    // 搜索框内容变化 → 更新当前标签过滤
    if show_search {
        if let Some(resp) = search_resp {
            w.search_focus = resp.has_focus();
            if w.cur().query != q {
                w.cur_mut().set_query(q);
            }
        }
    } else {
        w.search_focus = false;
    }

    // ---- 应用事件
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
    if let Some(d) = w.tab_drag.as_mut() {
        if d.active {
            d.pos = ctx.input(|i| i.pointer.interact_pos()).unwrap_or_default();
        }
    }
}

fn tab_tint(t: &Tab) -> Color32 {
    let s = t.path.to_string_lossy().to_string();
    if s.is_empty() {
        GRAY
    } else if s.len() == 3 && s.as_bytes()[1] == b':' {
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

// ================================================================ 命令条 & 地址栏

/// cmd_button 的实际宽度（用于给地址栏留位，必须在绘制前算出来）
fn command_bar(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    let full = ui.available_rect_before_wrap();
    let row = Rect::from_min_size(full.min, Vec2::new(full.width(), 28.0));
    let narrow = row.width() < 920.0;

    // ---- 右侧命令组：先排（从右往左），排完拿到它的左边界，剩下的才给地址栏。
    // 之前用「估算宽度预留」的写法，实际间距/内边距算不准，命令会盖到地址栏上。
    let mut right = ui.new_child(
        egui::UiBuilder::new().max_rect(row).layout(Layout::right_to_left(Align::Center)),
    );
    {
        let det = w.details;
        if view::cmd_button(&mut right, None, "详情", true).clicked() {
            w.details = !det;
        }
        if !narrow {
            right.add_space(2.0);
            if view::cmd_button(&mut right, Some("▾"), "排序", true).clicked() {
                // 锚在点击处下方（draw_popups 里会做窗口内收边）
                let a = right.ctx().input(|i| i.pointer.interact_pos()).unwrap_or_default();
                w.open_menu(Popup::Sort, Pos2::new(a.x - 60.0, a.y + 18.0), shared.t());
            }
            right.add_space(2.0);
            sep(&mut right);
        }
        let has_sel = !w.cur().sel.is_empty();
        // 「此电脑」里选中的是驱动器，不是真实文件，文件操作一律不适用
        let is_pc = w.cur().path.as_os_str().is_empty() || w.cur().path.to_string_lossy() == "\\\\";
        if view::cmd_button(&mut right, None, "删除", has_sel && !is_pc).clicked() {
            let p = w.cur().sel_paths();
            do_delete(w, shared, p);
        }
        let one = w.cur().sel.len() == 1 && !is_pc;
        if !narrow && view::cmd_button(&mut right, None, "重命名", one).clicked() {
            start_rename(w, shared.t());
        }
        if !narrow {
            sep(&mut right);
        }
        let has_clip = !shared.clip.lock().unwrap().0.is_empty() || fs_util::clip_has_files();
        if view::cmd_button(&mut right, None, "粘贴", has_clip && !is_pc).clicked() {
            paste(right.ctx(), w, shared);
        }
        if view::cmd_button(&mut right, None, "复制", has_sel && !is_pc).clicked() {
            let p = w.cur().sel_paths();
            if clip_write_files(&p, false).is_err() {
                shared.toast("写入剪贴板失败".into(), true);
            }
            *shared.clip.lock().unwrap() = (p, false);
        }
        if view::cmd_button(&mut right, None, "剪切", has_sel && !is_pc).clicked() {
            let p = w.cur().sel_paths();
            if clip_write_files(&p, true).is_err() {
                shared.toast("写入剪贴板失败".into(), true);
            }
            *shared.clip.lock().unwrap() = (p, true);
        }
        if view::cmd_button(&mut right, None, "新建", !is_pc).clicked() {
            new_item(w, shared, false);
        }
    }
    let cmd_left = right.min_rect().min.x;

    // ---- 左侧：导航 + 地址栏
    let left_rect = Rect::from_min_max(
        row.min,
        Pos2::new((cmd_left - 12.0).max(row.min.x + 220.0), row.max.y),
    );
    let mut left = ui.new_child(
        egui::UiBuilder::new().max_rect(left_rect).layout(Layout::left_to_right(Align::Center)),
    );
    {
        if view::icon_button(&mut left, "←", 30.0, w.cur().can_back(), "后退  Alt+←").clicked() {
            w.cur_mut().back();
        }
        if view::icon_button(&mut left, "→", 30.0, w.cur().can_fwd(), "前进  Alt+→").clicked() {
            w.cur_mut().fwd();
        }
        let up = w.cur().path.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf());
        if view::icon_button(&mut left, "↑", 30.0, up.is_some(), "上一级  Backspace").clicked() {
            if let Some(p) = up {
                w.cur_mut().navigate(p);
            }
        }
        if view::icon_button(&mut left, "⟳", 30.0, true, "刷新  F5").clicked() {
            w.cur_mut().reload();
        }
        left.add_space(6.0);

        let addr_w = left.available_width().max(120.0);
        let (rect, resp) = left.allocate_exact_size(Vec2::new(addr_w, 28.0), Sense::click());
        let editing = w.addr.is_some();
        let inner = view::field_rect(left.painter(), rect, editing);
        if editing {
            let mut buf = w.addr.take().unwrap();
            let te = Rect::from_min_max(
                Pos2::new(inner.min.x + 6.0, rect.min.y + 3.0),
                Pos2::new(rect.max.x - 8.0, rect.max.y - 3.0),
            );
            let r2 = left.put(
                te,
                TextEdit::singleline(&mut buf)
                    .frame(false)
                    .font(TextStyle::Body)
                    .desired_width(te.width()),
            );
            if w.addr_focus {
                r2.request_focus();
                w.addr_focus = false;
                w.addr_sel = true;
            }
            // 全选必须等控件真正持有焦点的那一帧再做，否则会被控件自己的 state 覆盖掉
            if w.addr_sel && r2.has_focus() {
                if let Some(mut st) = egui::TextEdit::load_state(left.ctx(), r2.id) {
                    let n = buf.chars().count();
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(n),
                    )));
                    st.store(left.ctx(), r2.id);
                }
                w.addr_sel = false;
            }
            let (enter, esc) = left.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
            if enter {
                let t = buf.trim().trim_matches('"').to_string();
                if t.is_empty() {
                    buffer_keep(w, None);
                } else {
                    let p = PathBuf::from(&t);
                    if p.is_dir() {
                        w.cur_mut().navigate(p);
                    } else {
                        shared.toast(format!("找不到该位置：{}", truncate(&t, 40)), true);
                        w.addr = Some(buf);
                        w.addr_focus = true;
                    }
                }
            } else if esc {
                buffer_keep(w, None);
            } else {
                buffer_keep(w, Some(buf));
            }
        } else {
            let hit = breadcrumb(&mut left, inner, w);
            if resp.clicked() && !hit {
                // 点地址栏空白 → 直接编辑路径（Windows 行为）
                let p = w.cur().path.to_string_lossy().to_string();
                w.addr = Some(if p == "\\\\" { String::new() } else { p });
                w.addr_focus = true;
            }
            resp.on_hover_text("点击可直接输入路径  Ctrl+L");
        }
    }
}

fn buffer_keep(w: &mut WinState, keep: Option<String>) {
    if let Some(s) = keep {
        w.addr = Some(s);
    }
}

fn sep(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(13.0, 24.0), Sense::hover());
    view::vline(ui.painter(), r.center().x, r.min.y + 3.0, r.max.y - 3.0, LINE2);
}

/// 地址栏里的面包屑：返回 true 表示本次点击落在某一段上（用于区分"点空白进编辑"）
fn breadcrumb(ui: &mut Ui, field: Rect, w: &mut WinState) -> bool {
    let parts = crumb_parts(&w.cur().path);
    let y = field.center().y;
    let budget = field.width() - 10.0;

    // 从尾部往前塞，塞不下的中间层折叠成 …
    let mut segs: Vec<(String, PathBuf, f32)> = Vec::new();
    let mut used = 0.0f32;
    for (label, p) in parts.iter().rev() {
        let wpx = estimate_width(label) + 18.0;
        if !segs.is_empty() && used + wpx > budget - 26.0 {
            break;
        }
        used += wpx + 12.0;
        segs.push((label.clone(), p.clone(), wpx));
    }
    segs.reverse();
    let truncated = segs.len() < parts.len();

    let mut x = field.min.x + 6.0;
    let mut hit = false;
    let mut nav: Option<PathBuf> = None;
    let mut menu: Option<(PathBuf, Pos2)> = None;
    let mut crumb_i = 0usize;

    if truncated {
        let r = Rect::from_min_size(Pos2::new(x, y - 11.0), Vec2::new(24.0, 22.0));
        let hov = ui.rect_contains_pointer(r);
        if hov {
            ui.painter().rect_filled(r, 5.0, HOVER);
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "…", FontId::proportional(12.0), FG2);
        if ui.interact(r, ui.id().with("crumbdots"), Sense::click()).clicked() {
            nav = Some(parts[0].1.clone());
            hit = true;
        }
        if w.fdrag.as_ref().map(|d| d.active).unwrap_or(false) && hov {
            w.drop = Some(DropTarget { dst: parts[0].1.clone(), rect: r });
        }
        x += 26.0;
        crumb_i += 1;
    }

    for (label, p, wpx) in segs.iter() {
        let is_last = crumb_i == parts.len() - 1 - (truncated as usize) + (truncated as usize) && false;
        let _ = is_last;
        let r = Rect::from_min_size(Pos2::new(x, y - 11.0), Vec2::new(*wpx, 22.0));
        let hov = ui.rect_contains_pointer(r);
        if hov {
            ui.painter().rect_filled(r, 5.0, HOVER);
        }
        ui.painter().text(
            Pos2::new(r.min.x + 9.0, y),
            Align2::LEFT_CENTER,
            truncate_width(label, *wpx - 16.0),
            FontId::proportional(12.0),
            if hov { FG } else { FG2 },
        );
        if ui.interact(r, ui.id().with(("crumbseg", crumb_i)), Sense::click()).clicked() {
            nav = Some(p.clone());
            hit = true;
        }
        if w.fdrag.as_ref().map(|d| d.active).unwrap_or(false) && hov {
            w.drop = Some(DropTarget { dst: p.clone(), rect: r });
        }
        x += wpx + 2.0;
        // 分隔符 ›（末段不画）
        if crumb_i < parts.len() - 1 {
            ui.painter().text(
                Pos2::new(x + 4.0, y),
                Align2::CENTER_CENTER,
                "›",
                FontId::proportional(11.0),
                MUTE,
            );
            x += 10.0;
        }
        crumb_i += 1;
    }

    if let Some(p) = nav {
        w.cur_mut().navigate(p);
    }
    if let Some((p, a)) = menu {
        w.crumb_menu = Some(CrumbMenu { path: p.clone(), anchor: a, rx: Some(spawn_ls(p)), dirs: Vec::new() });
    }
    hit
}

fn crumb_parts(path: &PathBuf) -> Vec<(String, PathBuf)> {
    let s = path.to_string_lossy().to_string();
    if s.is_empty() || s == "\\\\" {
        return vec![("此电脑".into(), PathBuf::from("\\\\"))];
    }
    let rev: Vec<PathBuf> =
        path.ancestors().map(|p| p.to_path_buf()).collect::<Vec<_>>().into_iter().rev().collect();
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
        ui.add_space(2.0);
        view::section_label(ui, "快速访问");
        let tints = [TEAL, GREEN, PINK, PURPLE, AMBER, ACC];
        for (k, (name, p)) in shared.quick.iter().enumerate() {
            let sel = *p == cur;
            let resp = side_item(ui, view::Glyph::Folder, tints[k % tints.len()], name, sel, "");
            if w.fdrag.as_ref().map(|d| d.active).unwrap_or(false) && resp.hovered() {
                w.drop = Some(DropTarget { dst: p.clone(), rect: resp.rect });
            }
            if resp.clicked() {
                nav = Some(p.clone());
            }
        }
        ui.add_space(16.0);
        // 「此电脑」既是根入口，也充当下方驱动器列表的分组头（不再单独放 section_label，避免同名两行）
        let at_root = cur.as_os_str().is_empty() || cur.to_string_lossy() == "\\\\";
        if side_item(ui, view::Glyph::Pc, ACC, "此电脑", at_root, "").clicked() {
            nav = Some(PathBuf::from("\\\\"));
        }
        ui.add_space(2.0);
        for (name, p) in shared.drives.iter() {
            let sel = !cur.as_os_str().is_empty() && cur.starts_with(p);
            let free = shared.free_text(p);
            let short = truncate(name, 16);
            let resp = side_item(ui, view::Glyph::Drive, GRAY, &short, sel, &free);
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

fn side_item(
    ui: &mut Ui,
    glyph: view::Glyph,
    tint: Color32,
    label: &str,
    sel: bool,
    right_text: &str,
) -> Response {
    // 有第二行（磁盘剩余空间）就抬高一点，避免长卷标被右侧文字挤成「新加卷 (…」
    let two_line = !right_text.is_empty();
    let h = if two_line { ROW_H + 8.0 } else { ROW_H };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
    let hov = resp.hovered() && !sel;
    let fill = if sel { ACC_BG } else if hov { HOVER } else { Color32::TRANSPARENT };
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    let g = Rect::from_center_size(Pos2::new(rect.min.x + 17.0, rect.center().y), Vec2::splat(15.0));
    match glyph {
        view::Glyph::Folder => view::folder_glyph(ui.painter(), g, if sel { tint } else { tint.gamma_multiply(0.92) }),
        view::Glyph::Drive => view::drive_glyph(ui.painter(), g, GRAY),
        view::Glyph::File => view::file_glyph(ui.painter(), g, GRAY),
        view::Glyph::Pc => view::pc_glyph(ui.painter(), g, if sel { ACC } else { FG2 }),
    }
    let name_max = rect.width() - 44.0;
    if two_line {
        ui.painter().text(
            Pos2::new(rect.min.x + 32.0, rect.center().y - 7.0),
            Align2::LEFT_CENTER,
            truncate_width(label, name_max),
            FontId::proportional(12.5),
            if sel { FG } else { FG2 },
        );
        ui.painter().text(
            Pos2::new(rect.min.x + 32.0, rect.center().y + 7.5),
            Align2::LEFT_CENTER,
            right_text,
            FontId::proportional(10.0),
            MUTE,
        );
    } else {
        ui.painter().text(
            Pos2::new(rect.min.x + 32.0, rect.center().y),
            Align2::LEFT_CENTER,
            truncate_width(label, name_max),
            FontId::proportional(12.5),
            if sel { FG } else { FG2 },
        );
    }
    resp
}

fn status_bar(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    ui.horizontal(|ui| {
        let n = w.cur().entries.len();
        let nmatch = w.cur_mut().view().len();
        let nsel = w.cur().sel.len();
        let bytes: u64 = w.cur().sel.iter().filter_map(|&i| w.cur().entries.get(i)).map(|e| e.size).sum();
        let at_root = w.cur().path.as_os_str().is_empty() || w.cur().path.to_string_lossy() == "\\\\";
        // 根视图没有真实 entries（列出的是驱动器），别显示成「0 个项目」
        let mut left = if at_root {
            format!("{} 个驱动器", shared.drives.len())
        } else {
            format!("{} 个项目", n)
        };
        if !w.cur().query.is_empty() {
            // 用过滤后的真实条数，别拿「行数 - 2 个组头」推算：某一边为空时组头只有一或零个
            left = format!("{} 项匹配（共 {} 项）", nmatch, n);
        }
        if nsel > 0 {
            left.push_str(&format!("  ·  已选择 {} 项", nsel));
            if bytes > 0 {
                left.push_str(&format!("  ·  {}", fmt_size(bytes)));
            }
        }
        ui.label(RichText::new(left).size(11.0).color(FG2));

        let jobs = shared.jobs.lock().unwrap();
        if !jobs.is_empty() {
            ui.add_space(14.0);
            ui.spinner();
            ui.label(RichText::new(format!("{} 个后台任务", jobs.len())).size(11.0).color(FG2));
            if let Some((_, h)) = jobs.first() {
                let g = h.progress.lock().unwrap();
                ui.label(RichText::new(truncate(&g.text, 26)).size(11.0).color(MUTE));
            }
        }
        drop(jobs);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new("F5 刷新 · Ctrl+L 路径 · Ctrl+I 详情").size(10.5).color(MUTE));
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
    Fold(RowKind),
}

fn content(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    let path_s = w.cur().path.to_string_lossy().to_string();
    if path_s.is_empty() || path_s == "\\\\" {
        this_pc(ui, w, shared);
        return;
    }
    let loading = w.cur().loading;
    let mut rows = w.cur_mut().rows();
    if rows.is_empty() {
        if loading {
            ui.add_space(60.0);
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.spinner();
                ui.label(RichText::new("正在读取…").size(12.0).color(MUTE));
            });
        } else if !w.cur().query.is_empty() {
            empty_state(ui, "没有匹配的项目", "换个关键词，或按 Esc 清空搜索");
        } else {
            empty_state(ui, "此文件夹为空", "把文件拖进来，或在命令条点「新建」");
        }
        return;
    }

    // 列宽：窗口窄时先砍「类型」「修改日期」，名字列优先。
    // 每个数字列都记「右边界」，表头与数据用同一个锚点，才不会出现表头和数据错位。
    let avail = ui.available_width();
    let pad = 14.0;
    let x0 = ui.max_rect().min.x;
    let col_size = 88.0;
    let col_type = if avail > 560.0 { 92.0 } else { 0.0 };
    let col_date = if avail > 470.0 { 132.0 } else { 0.0 };
    let r_size = x0 + avail - pad; // 大小列右边界
    let r_type = r_size - col_size; // 类型列右边界
    let r_date = r_type - col_type; // 日期列右边界
    let x_name = x0 + 34.0;

    // ---- 列头
    let (hdr, _) = ui.allocate_exact_size(Vec2::new(avail, HEAD_H), Sense::hover());
    let mut sort_act: Option<SortKey> = None;
    sort_act = sort_act.or(header_cell(ui, Rect::from_min_max(hdr.min, Pos2::new(r_date - 8.0, hdr.max.y)), "名称", SortKey::Name, w, false));
    if col_date > 0.0 {
        sort_act = sort_act.or(header_cell(ui, Rect::from_min_max(Pos2::new(r_date - col_date, hdr.min.y), Pos2::new(r_date, hdr.max.y)), "修改日期", SortKey::Date, w, true));
    }
    if col_type > 0.0 {
        sort_act = sort_act.or(header_cell(ui, Rect::from_min_max(Pos2::new(r_type - col_type, hdr.min.y), Pos2::new(r_type, hdr.max.y)), "类型", SortKey::Type, w, true));
    }
    sort_act = sort_act.or(header_cell(ui, Rect::from_min_max(Pos2::new(r_type, hdr.min.y), Pos2::new(hdr.max.x, hdr.max.y)), "大小", SortKey::Size, w, true));
    if let Some(k) = sort_act {
        w.cur_mut().set_sort(k);
        rows = w.cur_mut().rows(); // 排序变了，当帧就用新顺序
    }
    view::hairline(ui.painter(), hdr.min.x, hdr.max.x, hdr.max.y - 0.5, SEP);
    ui.add_space(2.0);

    // 中央区域整体右键 → 空白菜单；左键空白 → 取消选择
    let central_rect = ui.available_rect_before_wrap();
    let bg = ui.interact(central_rect, ui.id().with("centralbg"), Sense::click());
    let empty_ctx = bg.secondary_clicked();
    let empty_click = bg.clicked();

    let mut act = ContentAct::None;
    ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_H, rows.len(), |ui, range| {
        for vi in range {
            let row = match rows.get(vi) {
                Some(r) => *r,
                None => continue,
            };
            match row.kind {
                RowKind::DirHeader | RowKind::FileHeader => {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                    let is_dir = row.kind == RowKind::DirHeader;
                    let folded = if is_dir { w.cur().fold_dirs } else { w.cur().fold_files };
                    let hov = resp.hovered();
                    if hov {
                        ui.painter().rect_filled(rect.shrink2(Vec2::new(2.0, 1.0)), 6.0, HOVER);
                    }
                    view::chevron(
                        ui.painter(),
                        Pos2::new(rect.min.x + 18.0, rect.center().y),
                        9.0,
                        MUTE,
                        !folded,
                    );
                    let title = if is_dir { "文件夹" } else { "文件" };
                    ui.painter().text(
                        Pos2::new(rect.min.x + 32.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        title,
                        FontId::proportional(12.0),
                        FG2,
                    );
                    let px = rect.min.x + 32.0 + estimate_width(title) + 8.0;
                    let pill = view::count_pill(ui, Pos2::new(px, rect.center().y), row.idx);
                    view::hairline(
                        ui.painter(),
                        pill.max.x + 10.0,
                        rect.max.x - 14.0,
                        rect.center().y,
                        LINE,
                    );
                    if resp.clicked() {
                        act = ContentAct::Fold(row.kind);
                    }
                }
                RowKind::Item => {
                    let i = row.idx;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click_and_drag());
                    let sel = w.cur().sel.contains(&i);
                    let hov = resp.hovered() && !sel;
                    let cell = rect.shrink2(Vec2::new(2.0, 1.0));
                    if sel {
                        ui.painter().rect_filled(cell, 6.0, ACC_BG);
                    } else if hov {
                        ui.painter().rect_filled(cell, 6.0, HOVER);
                    }
                    let e = match w.cur().entries.get(i) {
                        Some(e) => e,
                        None => continue,
                    };
                    let tint = icon_color(e);
                    let g = Rect::from_center_size(Pos2::new(rect.min.x + 17.0, rect.center().y), Vec2::splat(16.0));
                    if e.is_dir {
                        view::folder_glyph(ui.painter(), g, tint);
                    } else {
                        view::file_glyph(ui.painter(), g, tint);
                    }

                    // 名字：主体用 FG，扩展名淡化（Argus 里 mspaint.exe 的那种处理）
                    let name_max = (r_date - 10.0 - (rect.min.x + 34.0)).max(40.0);
                    let name_c = if e.hidden { MUTE } else { FG };
                    let (stem, ext_part) = if e.is_dir || e.ext.is_empty() {
                        (e.name.clone(), String::new())
                    } else {
                        let cut = e.name.len() - e.ext.len() - 1;
                        if cut > 0 && e.name.is_char_boundary(cut) {
                            (e.name[..cut].to_string(), e.name[cut..].to_string())
                        } else {
                            (e.name.clone(), String::new())
                        }
                    };
                    let fnt = FontId::proportional(12.5);
                    let stem_w = ui.fonts(|f| f.layout_no_wrap(stem.clone(), fnt.clone(), name_c).size().x);
                    let ext_w = if ext_part.is_empty() {
                        0.0
                    } else {
                        ui.fonts(|f| f.layout_no_wrap(ext_part.clone(), fnt.clone(), MUTE).size().x)
                    };
                    let (stem_s, ext_s) = if stem_w + ext_w > name_max {
                        (truncate_width(&stem, name_max), String::new())
                    } else {
                        (stem, ext_part)
                    };
                    let p_name = Pos2::new(rect.min.x + 34.0, rect.center().y);
                    // 用一个 galley 分两段着色（主体 FG / 扩展名 MUTE）：
                    // 之前按字符数估算宽度分开画两次，遇到比例不同的字会重叠或留大缝。
                    let mut job = egui::text::LayoutJob::default();
                    job.append(
                        &stem_s,
                        0.0,
                        egui::TextFormat { font_id: fnt.clone(), color: name_c, ..Default::default() },
                    );
                    if !ext_s.is_empty() {
                        job.append(
                            &ext_s,
                            0.0,
                            egui::TextFormat { font_id: fnt.clone(), color: MUTE, ..Default::default() },
                        );
                    }
                    let galley = ui.fonts(|f| f.layout_job(job));
                    let gy = rect.center().y - galley.size().y * 0.5;
                    ui.painter().galley(Pos2::new(p_name.x, gy), galley, name_c);
                    if col_date > 0.0 {
                        ui.painter().text(
                            Pos2::new(r_date - 10.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            fmt_dt(e.mtime),
                            FontId::proportional(11.5),
                            FG2,
                        );
                    }
                    if col_type > 0.0 {
                        ui.painter().text(
                            Pos2::new(r_type - 10.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            type_name(e),
                            FontId::proportional(11.5),
                            MUTE,
                        );
                    }
                    ui.painter().text(
                        Pos2::new(r_size - 10.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        if e.is_dir { String::new() } else { fmt_size(e.size) },
                        FontId::proportional(11.5),
                        FG2,
                    );

                    // e 的借用到此结束（下面要改 w）：需要的标量先拷出来
                    let is_dir_row = e.is_dir;

                    if resp.double_clicked() {
                        act = ContentAct::Open(i);
                    } else if resp.secondary_clicked() {
                        act = ContentAct::Ctx;
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

                    // ---- 拖拽
                    if resp.drag_started() {
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
                        let dst = w.cur().entries.get(i).map(|x| x.path.clone());
                        if let Some(dst) = dst {
                            w.drop = Some(DropTarget { dst, rect });
                        }
                    }
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
        ContentAct::Fold(kind) => {
            let t = w.cur_mut();
            if kind == RowKind::DirHeader {
                t.fold_dirs = !t.fold_dirs;
            } else {
                t.fold_files = !t.fold_files;
            }
            t.view_dirty = true;
        }
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
            let vis = w.cur_mut().visible();
            let t = w.cur_mut();
            if shift {
                if let Some(last) = t.sel_last {
                    if let (Some(a), Some(b)) =
                        (vis.iter().position(|&x| x == last), vis.iter().position(|&x| x == i))
                    {
                        let (a, b) = (a.min(b), a.max(b));
                        for &x in &vis[a..=b] {
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

/// 空态：大图标 + 两行说明
fn empty_state(ui: &mut Ui, title: &str, hint: &str) {
    let r = ui.available_rect_before_wrap();
    let c = Pos2::new(r.center().x, r.center().y - 34.0);
    view::folder_glyph(ui.painter(), Rect::from_center_size(c, Vec2::splat(48.0)), Color32::from_rgb(216, 219, 224));
    ui.painter().text(c + Vec2::new(0.0, 44.0), Align2::CENTER_CENTER, title, FontId::proportional(13.0), FG2);
    ui.painter().text(c + Vec2::new(0.0, 66.0), Align2::CENTER_CENTER, hint, FontId::proportional(11.5), MUTE);
}

fn header_cell(ui: &mut Ui, r: Rect, label: &str, key: SortKey, w: &mut WinState, right: bool) -> Option<SortKey> {
    let resp = ui.interact(r, ui.id().with(("hdr", label)), Sense::click());
    let hov = resp.hovered();
    if hov {
        ui.painter().rect_filled(r.shrink2(Vec2::new(2.0, 3.0)), 5.0, HOVER);
    }
    let active = w.cur().sort == key;
    let desc = w.cur().sort_desc;
    let txt = if active { FG } else { FG2 };
    let anchor_x = if right { r.max.x - 10.0 } else { r.min.x + 34.0 };
    let align = if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
    ui.painter().text(Pos2::new(anchor_x, r.center().y), align, label, FontId::proportional(11.5), txt);
    if active {
        let cw = estimate_width(label);
        let cx = if right { anchor_x - cw - 10.0 } else { anchor_x + cw + 10.0 };
        view::caret(ui.painter(), Pos2::new(cx, r.center().y), 7.0, desc, txt);
    }
    resp.on_hover_text("点击切换排序").clicked().then_some(key)
}

/// 此电脑：磁盘卡片列表（图标方块 + 容量条）
fn this_pc(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    let mut nav: Option<PathBuf> = None;
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(RichText::new("设备和驱动器").size(12.0).color(MUTE));
        });
        ui.add_space(6.0);
        for (i, (name, p)) in shared.drives.iter().enumerate() {
            let sp = shared.space(p);
            let (rect, resp) = ui.allocate_exact_size(
                Vec2::new(ui.available_width() - 28.0, 66.0),
                Sense::click(),
            );
            let picked = w.cur().sel.contains(&i);
            if picked {
                ui.painter().rect_filled(rect.shrink2(Vec2::new(2.0, 1.0)), 8.0, ACC_BG);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect.shrink2(Vec2::new(2.0, 1.0)), 8.0, BG2);
            }
            let tile = Rect::from_center_size(Pos2::new(rect.min.x + 44.0, rect.center().y), Vec2::splat(42.0));
            view::icon_tile(ui.painter(), tile, ACC);
            view::drive_glyph(ui.painter(), Rect::from_center_size(tile.center(), Vec2::splat(22.0)), ACC);
            let tx = tile.max.x + 14.0;
            let right_x = rect.max.x - 18.0;
            let name_w = (right_x - tx - 96.0).max(80.0);
            ui.painter().text(
                Pos2::new(tx, rect.center().y - 13.0),
                Align2::LEFT_CENTER,
                truncate_width(name, name_w),
                FontId::proportional(13.0),
                FG,
            );
            let sub = if sp.total > 0 {
                format!("可用 {}", fmt_size(sp.free))
            } else {
                "容量未知".to_string()
            };
            ui.painter().text(
                Pos2::new(tx, rect.center().y + 6.0),
                Align2::LEFT_CENTER,
                sub,
                FontId::proportional(11.0),
                MUTE,
            );
            if sp.total > 0 {
                // 右栏：总容量 + 已用占比（与左栏的「可用」互为补数，不重复）
                ui.painter().text(
                    Pos2::new(right_x, rect.center().y - 13.0),
                    Align2::RIGHT_CENTER,
                    fmt_size(sp.total),
                    FontId::proportional(12.0),
                    FG2,
                );
                ui.painter().text(
                    Pos2::new(right_x, rect.center().y + 6.0),
                    Align2::RIGHT_CENTER,
                    format!("已用 {:.0}%", sp.frac * 100.0),
                    FontId::proportional(10.5),
                    if sp.frac > 0.9 { RED } else { MUTE },
                );
                let bar = Rect::from_min_size(
                    Pos2::new(tx, rect.center().y + 19.0),
                    Vec2::new((right_x - tx - 150.0).clamp(60.0, 340.0), 4.0),
                );
                view::usage_bar(ui.painter(), bar, sp.frac, if sp.frac > 0.9 { RED } else { ACC });
            }
            if resp.clicked() {
                // 单击选中（详情面板显示该盘），双击才进入
                w.cur_mut().sel.clear();
                w.cur_mut().sel.insert(i);
            }
            if resp.double_clicked() {
                nav = Some(p.clone());
            }
            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.add_space(2.0);
        }
    });
    if let Some(p) = nav {
        w.cur_mut().navigate(p);
    }
}

// ================================================================ 详情面板

fn details_pane(ui: &mut Ui, w: &mut WinState, shared: &Arc<Shared>) {
    let full = ui.max_rect();

    // 头部
    let (hdr, _) = ui.allocate_exact_size(Vec2::new(full.width(), 40.0), Sense::hover());
    ui.painter().text(
        Pos2::new(hdr.min.x + 14.0, hdr.center().y),
        Align2::LEFT_CENTER,
        "详细信息",
        FontId::proportional(12.5),
        FG,
    );
    let cr = Rect::from_center_size(Pos2::new(hdr.max.x - 22.0, hdr.center().y), Vec2::splat(24.0));
    let chov = ui.rect_contains_pointer(cr);
    if chov {
        ui.painter().rect_filled(cr, 5.0, HOVER2);
    }
    view::cross_glyph(ui.painter(), cr.center(), 11.0, if chov { FG } else { MUTE });
    if ui.interact(cr, ui.id().with("detclose"), Sense::click()).clicked() {
        w.details = false;
        return;
    }
    view::hairline(ui.painter(), full.min.x, full.max.x, hdr.max.y, LINE);

    let path = w.cur().path.clone();
    let path_s = path.to_string_lossy().to_string();
    let at_pc = path_s.is_empty() || path_s == "\\\\";
    let sel: Vec<usize> = w.cur().sel.iter().cloned().collect();

    let mut nav: Option<PathBuf> = None;
    let mut open: Option<PathBuf> = None;
    let mut copy_path = false;

    ui.add_space(12.0);
    Frame::none()
        .inner_margin(Margin { left: 14, right: 14, top: 0, bottom: 0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            if at_pc {
                // 选中某个驱动器：显示该盘的容量明细
                if sel.len() == 1 {
                    if let Some((dname, dp)) = shared.drives.get(sel[0]).cloned() {
                        let sp = shared.space(&dp);
                        let tile = Rect::from_min_size(ui.cursor().min, Vec2::splat(56.0));
                        ui.painter().rect_filled(tile, 12.0, Color32::from_rgba_unmultiplied(ACC.r(), ACC.g(), ACC.b(), 24));
                        ui.painter().rect_stroke(
                            tile,
                            12.0,
                            (1.0, Color32::from_rgba_unmultiplied(ACC.r(), ACC.g(), ACC.b(), 48)),
                            StrokeKind::Inside,
                        );
                        view::drive_glyph(ui.painter(), Rect::from_center_size(tile.center(), Vec2::splat(30.0)), ACC);
                        ui.add_space(60.0);
                        ui.label(RichText::new(truncate(&dname, 30)).size(13.0).color(FG));
                        ui.add_space(2.0);
                        ui.label(RichText::new("本地磁盘").size(11.0).color(MUTE));
                        ui.add_space(14.0);
                        view::hairline(ui.painter(), ui.cursor().min.x, ui.cursor().min.x + ui.available_width(), ui.cursor().min.y, SEP);
                        ui.add_space(12.0);
                        spec_row(ui, "总容量", &fmt_size(sp.total));
                        spec_row(ui, "可用空间", &fmt_size(sp.free));
                        spec_row(
                            ui,
                            "已用",
                            &format!("{}（{:.0}%）", fmt_size(sp.total.saturating_sub(sp.free)), sp.frac * 100.0),
                        );
                        ui.add_space(2.0);
                        let bar = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 8.0));
                        view::usage_bar(ui.painter(), bar, sp.frac, if sp.frac > 0.9 { RED } else { ACC });
                        ui.add_space(18.0);
                        if accent_btn(ui, "打开", true).clicked() {
                            nav = Some(dp.clone());
                        }
                        return;
                    }
                }
                let mut tot = 0u64;
                let mut avail = 0u64;
                for (_, p) in shared.drives.iter() {
                    let sp = shared.space(p);
                    tot += sp.total;
                    avail += sp.free;
                }
                ui.label(RichText::new(format!("{} 个驱动器", shared.drives.len())).size(14.0).color(FG));
                ui.add_space(3.0);
                ui.label(
                    RichText::new(format!("共 {} · 可用 {}", fmt_size(tot), fmt_size(avail)))
                        .size(11.0)
                        .color(MUTE),
                );
                ui.add_space(12.0);
                let y = ui.cursor().min.y;
                view::hairline(ui.painter(), ui.cursor().min.x, ui.cursor().min.x + ui.available_width(), y, LINE);
                ui.add_space(12.0);
                for (name, p) in shared.drives.iter() {
                    let sp = shared.space(p);
                    ui.horizontal(|ui| {
                        let (tile, _) = ui.allocate_exact_size(Vec2::new(34.0, 34.0), Sense::hover());
                        view::icon_tile(ui.painter(), tile, ACC);
                        view::drive_glyph(ui.painter(), Rect::from_center_size(tile.center(), Vec2::splat(17.0)), ACC);
                        ui.add_space(9.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(name.clone()).size(12.5).color(FG));
                            let w_bar = (ui.available_width() - 18.0).clamp(60.0, 176.0);
                            let bar = Rect::from_min_size(
                                ui.cursor().min + Vec2::new(0.0, 3.0),
                                Vec2::new(w_bar, 4.0),
                            );
                            view::usage_bar(ui.painter(), bar, sp.frac, if sp.frac > 0.9 { RED } else { ACC });
                            ui.add_space(9.0);
                            ui.label(
                                RichText::new(format!(
                                    "可用 {} · 已用 {:.0}%",
                                    fmt_size(sp.free),
                                    sp.frac * 100.0
                                ))
                                .size(10.5)
                                .color(MUTE),
                            );
                        });
                    });
                    ui.add_space(12.0);
                }
                return;
            }

            // 单选：图标 + 名称 + 规格
            if sel.len() == 1 {
                if let Some(e) = w.cur().entries.get(sel[0]).cloned() {
                    let tint = icon_color(&e);
                    let tile = Rect::from_min_size(ui.cursor().min, Vec2::splat(56.0));
                    ui.painter().rect_filled(tile, 12.0, Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 24));
                    ui.painter().rect_stroke(
                        tile,
                        12.0,
                        (1.0, Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 48)),
                        StrokeKind::Inside,
                    );
                    let g = Rect::from_center_size(tile.center(), Vec2::splat(30.0));
                    if e.is_dir {
                        view::folder_glyph(ui.painter(), g, tint);
                    } else {
                        view::file_glyph(ui.painter(), g, tint);
                    }
                    ui.add_space(60.0);
                    ui.label(RichText::new(truncate(&e.name, 40)).size(13.0).color(FG));
                    ui.add_space(2.0);
                    let kind = type_name(&e);
                    ui.label(RichText::new(kind).size(11.0).color(MUTE));
                    ui.add_space(14.0);
                    view::hairline(ui.painter(), ui.cursor().min.x, ui.cursor().min.x + ui.available_width(), ui.cursor().min.y, SEP);
                    ui.add_space(12.0);

                    if !e.is_dir {
                        spec_row(ui, "大小", &fmt_size(e.size));
                    }
                    spec_row(ui, "修改时间", &fmt_time(e.mtime).replace('-', "/"));
                    if e.hidden {
                        spec_row(ui, "属性", "隐藏");
                    }
                    let parent = e.path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                    // 路径尾部信息更重要：从左截断
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(RichText::new("位置").size(10.5).color(MUTE));
                        let v = truncate_left(&parent, ui.available_width());
                        ui.label(RichText::new(v).size(11.0).color(FG2));
                    });
                    ui.add_space(11.0);

                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if accent_btn(ui, "打开", true).clicked() {
                            open = Some(e.path.clone());
                        }
                        if accent_btn(ui, "复制路径", false).clicked() {
                            copy_path = true;
                        }
                    });
                    let _ = nav;
                }
            } else if sel.len() > 1 {
                let mut bytes = 0u64;
                let mut dirs = 0usize;
                for &i in sel.iter() {
                    if let Some(e) = w.cur().entries.get(i) {
                        if e.is_dir {
                            dirs += 1;
                        } else {
                            bytes += e.size;
                        }
                    }
                }
                ui.label(RichText::new(format!("已选择 {} 个项目", sel.len())).size(13.0).color(FG));
                ui.add_space(10.0);
                spec_row(ui, "文件夹", &format!("{} 个", dirs));
                spec_row(ui, "文件", &format!("{} 个", sel.len() - dirs));
                spec_row(ui, "文件合计", &fmt_size(bytes));
                ui.add_space(14.0);
                view::hairline(ui.painter(), ui.cursor().min.x, ui.cursor().min.x + ui.available_width(), ui.cursor().min.y, LINE);
                ui.add_space(12.0);
                ui.label(RichText::new("按类型").size(11.0).color(MUTE));
                ui.add_space(6.0);
                // 扩展名分布 Top 4
                let mut map: std::collections::HashMap<String, (usize, u64)> = std::collections::HashMap::new();
                for &i in sel.iter() {
                    if let Some(e) = w.cur().entries.get(i) {
                        let k = if e.is_dir { "文件夹".to_string() } else if e.ext.is_empty() { "无扩展名".into() } else { e.ext.clone() };
                        let v = map.entry(k).or_insert((0, 0));
                        v.0 += 1;
                        v.1 += e.size;
                    }
                }
                let mut list: Vec<(String, (usize, u64))> = map.into_iter().collect();
                list.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then(b.1 .0.cmp(&a.1 .0)));
                for (k, (n, sz)) in list.into_iter().take(6) {
                    spec_row(ui, &k, &format!("{} 个 · {}", n, fmt_size(sz)));
                }
            } else {
                // 无选中：当前文件夹概览 + 磁盘
                let cur = w.cur();
                let nd = cur.entries.iter().filter(|e| e.is_dir).count();
                let nf = cur.entries.len() - nd;
                let bytes: u64 = cur.entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
                let name = cur.title();
                ui.label(RichText::new(truncate(&name, 22)).size(13.0).color(FG));
                ui.add_space(2.0);
                ui.label(RichText::new("当前文件夹").size(11.0).color(MUTE));
                ui.add_space(14.0);
                spec_row(ui, "文件夹", &format!("{} 个", nd));
                spec_row(ui, "文件", &format!("{} 个", nf));
                spec_row(ui, "文件合计", &fmt_size(bytes));
                ui.add_space(16.0);
                view::hairline(ui.painter(), ui.cursor().min.x, ui.cursor().min.x + ui.available_width(), ui.cursor().min.y, LINE);
                ui.add_space(14.0);
                let sp = shared.space(&path);
                if sp.total > 0 {
                    ui.label(RichText::new("所在磁盘").size(11.0).color(MUTE));
                    ui.add_space(6.0);
                    let bar = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 6.0));
                    view::usage_bar(ui.painter(), bar, sp.frac, if sp.frac > 0.9 { RED } else { ACC });
                    ui.add_space(10.0);
                    spec_row(ui, "已用", &fmt_size(sp.total - sp.free));
                    spec_row(ui, "可用", &fmt_size(sp.free));
                    spec_row(ui, "总容量", &fmt_size(sp.total));
                }
                ui.add_space(18.0);
                ui.label(RichText::new("按住 Ctrl 可多选，右键条目查看更多操作").size(10.5).color(MUTE));
            }
        });

    if let Some(p) = open {
        if p.is_dir() {
            nav = Some(p);
        } else {
            let _ = open_with_default(&p);
        }
    }
    if copy_path {
        if let Some(p) = w.cur().sel_paths().first() {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(p.to_string_lossy().to_string());
            }
            shared.toast("已复制路径".into(), false);
        }
    }
    if let Some(p) = nav {
        w.cur_mut().navigate(p);
    }
}

/// 详情面板里的「标签 + 值」两行块：标签与值贴紧，块之间留白
fn spec_row(ui: &mut Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(RichText::new(label).size(10.5).color(MUTE));
        let v = truncate_width(value, ui.available_width());
        ui.label(RichText::new(v).size(12.0).color(FG));
    });
    ui.add_space(11.0);
}

// ================================================================ 弹层

fn draw_popups(ctx: &egui::Context, w: &mut WinState, shared: &Arc<Shared>, now: f64) {
    let dy = (1.0 - ease_out(((now - w.popup_at) * 1000.0 / 160.0) as f32)) * 6.0;
    let scr = ctx.screen_rect();

    // ---- 右键菜单 / 排序菜单
    if let Some(popup) = w.popup.clone() {
        let anchor = w.popup_anchor;
        let t = w.cur();
        let has_sel = !t.sel.is_empty();
        let one = t.sel.len() == 1;
        let has_clip = !shared.clip.lock().unwrap().0.is_empty() || fs_util::clip_has_files();
        let at_pc = t.path.as_os_str().is_empty() || t.path.to_string_lossy() == "\\\\";
        // 第三项是右对齐的快捷键提示；label 为 "|" 表示分组分隔线
        let items: Vec<(String, bool, &'static str)> = match &popup {
            Popup::Tab { .. } => vec![
                ("在新窗口中打开".into(), true, ""),
                ("复制为新标签".into(), true, ""),
                ("|".into(), true, ""),
                ("关闭标签".into(), true, "Ctrl+W"),
            ],
            Popup::File => vec![
                ("打开".into(), has_sel, "Enter"),
                ("复制路径".into(), has_sel, ""),
                ("在资源管理器中显示".into(), has_sel, ""),
                ("|".into(), true, ""),
                ("复制".into(), has_sel, "Ctrl+C"),
                ("剪切".into(), has_sel, "Ctrl+X"),
                ("粘贴".into(), has_clip, "Ctrl+V"),
                ("|".into(), true, ""),
                ("重命名".into(), one, "F2"),
                ("移动到回收站".into(), has_sel, "Del"),
                ("|".into(), true, ""),
                ("新建文件夹".into(), true, "Ctrl+Shift+N"),
                ("新建文本文档".into(), true, ""),
            ],
            Popup::Empty => vec![
                ("粘贴".into(), has_clip && !at_pc, "Ctrl+V"),
                ("|".into(), true, ""),
                ("新建文件夹".into(), !at_pc, "Ctrl+Shift+N"),
                ("新建文本文档".into(), !at_pc, ""),
                ("|".into(), true, ""),
                ("全选".into(), !t.entries.is_empty(), "Ctrl+A"),
                ("刷新".into(), true, "F5"),
            ],
            Popup::Sort => vec![
                ("按名称".into(), true, ""),
                ("按修改日期".into(), true, ""),
                ("按类型".into(), true, ""),
                ("按大小".into(), true, ""),
            ],
        };

        // 排序菜单：当前项用强调色 + 手绘箭头（与列头一致），不在文字里拼 ↑↓
        let sort_active: Option<(&'static str, bool)> = match &popup {
            Popup::Sort => Some((
                match t.sort {
                    SortKey::Name => "按名称",
                    SortKey::Date => "按修改日期",
                    SortKey::Type => "按类型",
                    SortKey::Size => "按大小",
                },
                t.sort_desc,
            )),
            _ => None,
        };

        // 菜单不能出血：贴右缘/下缘时向内收
        let mw = 216.0;
        let mh: f32 = items
            .iter()
            .map(|(l, _, _)| if l == "|" { 6.0 } else { 28.0 })
            .sum::<f32>()
            + 10.0;
        let mut pos = anchor + Vec2::new(0.0, dy);
        if pos.x + mw > scr.max.x - 6.0 {
            pos.x = (scr.max.x - mw - 6.0).max(scr.min.x + 6.0);
        }
        if pos.y + mh > scr.max.y - 6.0 {
            pos.y = (anchor.y - mh - 4.0).max(scr.min.y + 6.0);
        }

        let mut clicked: Option<String> = None;
        // 用 Area 而不是 Window：Window 默认可缩放，菜单会被撑成一大块空白
        let out = egui::Area::new(Id::new(("menu", w.id)))
            .fixed_pos(pos)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                Frame::none()
                    .fill(SURF)
                    .corner_radius(8.0)
                    .inner_margin(Margin::symmetric(5, 5))
                    .stroke((1.0, SEP2))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 20,
                        spread: 0,
                        color: Color32::from_rgba_unmultiplied(10, 14, 20, 46),
                    })
                    .show(ui, |ui| {
                        ui.set_min_width(206.0);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for (label, enabled, hint) in items.iter() {
                            if label == "|" {
                                // 分组分隔线：左右各缩进 8，居中留 3px 上下透气
                                let (r, _) = ui.allocate_exact_size(Vec2::new(206.0, 6.0), Sense::hover());
                                ui.painter().hline(
                                    (r.min.x + 8.0)..=(r.max.x - 8.0),
                                    r.center().y,
                                    egui::Stroke::new(1.0, SEP),
                                );
                                continue;
                            }
                            // 固定行宽：Area 里 available_width 是「到屏幕右缘」，
                            // 用它分配会把菜单撑满整屏（还被 constrain 挤到左边）
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(206.0, 28.0), Sense::click());
                            let hov = resp.hovered() && *enabled;
                            let active = sort_active.map(|(l, _)| l == label.as_str()).unwrap_or(false);
                            if hov {
                                ui.painter().rect_filled(rect, 5.0, HOVER);
                            }
                            let lc = if active {
                                ACC
                            } else if !*enabled {
                                Color32::from_rgba_unmultiplied(MUTE.r(), MUTE.g(), MUTE.b(), 132)
                            } else if hov {
                                FG
                            } else {
                                FG2
                            };
                            ui.painter().text(
                                Pos2::new(rect.min.x + 10.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                label,
                                FontId::proportional(12.5),
                                lc,
                            );
                            if let Some((_, desc)) = sort_active {
                                if active {
                                    let cx = rect.min.x + 10.0 + estimate_width(label) + 9.0;
                                    view::caret(ui.painter(), Pos2::new(cx, rect.center().y), 7.0, desc, ACC);
                                }
                            }
                            if !hint.is_empty() {
                                ui.painter().text(
                                    Pos2::new(rect.max.x - 10.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    *hint,
                                    FontId::proportional(10.5),
                                    if !*enabled {
                                        Color32::from_rgba_unmultiplied(MUTE.r(), MUTE.g(), MUTE.b(), 90)
                                    } else {
                                        MUTE
                                    },
                                );
                            }
                            if resp.clicked() && *enabled {
                                clicked = Some(label.clone());
                            }
                        }
                    });
            });
        w.popup_rect = Some(out.response.rect);
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
        let out = egui::Area::new(Id::new(("crumbmenu", w.id)))
            .fixed_pos(anchor + Vec2::new(0.0, dy))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                Frame::none()
                    .fill(SURF)
                    .corner_radius(8.0)
                    .inner_margin(Margin::symmetric(5, 5))
                    .stroke((1.0, SEP2))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 20,
                        spread: 0,
                        color: Color32::from_rgba_unmultiplied(10, 14, 20, 46),
                    })
                    .show(ui, |ui| {
                        ui.set_min_width(224.0);
                        ui.set_max_height(320.0);
                        ScrollArea::vertical().show(ui, |ui| {
                            if cm.dirs.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new("…").size(12.0).color(MUTE));
                                });
                            }
                            for d in cm.dirs.iter() {
                                let (rect, resp) = ui.allocate_exact_size(Vec2::new(214.0, 28.0), Sense::click());
                                if resp.hovered() {
                                    ui.painter().rect_filled(rect, 5.0, HOVER);
                                }
                                view::folder_glyph(
                                    ui.painter(),
                                    Rect::from_center_size(
                                        Pos2::new(rect.min.x + 15.0, rect.center().y),
                                        Vec2::splat(13.0),
                                    ),
                                    AMBER,
                                );
                                ui.painter().text(
                                    Pos2::new(rect.min.x + 30.0, rect.center().y),
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
            });
        if w.popup.is_none() {
            w.popup_rect = Some(out.response.rect);
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
                let vis = w.cur_mut().visible();
                let t = w.cur_mut();
                t.sel.clear();
                for &i in vis.iter() {
                    t.sel.insert(i);
                }
                t.sel_last = vis.last().copied();
            }
            "刷新" => w.cur_mut().reload(),
            _ => {}
        },
        Popup::Sort => {
            let k = if label.contains("名称") {
                SortKey::Name
            } else if label.contains("修改日期") {
                SortKey::Date
            } else if label.contains("类型") {
                SortKey::Type
            } else {
                SortKey::Size
            };
            w.cur_mut().set_sort(k);
        }
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

    // 焦点在文本框里（搜索 / 地址栏 / 重命名）时把视图级快捷键让出去：
    // 否则 Ctrl+V 会同时触发「粘贴文件」、Delete 会删选中的文件。只放行 Esc 交给下面处理。
    let editing = w.search_focus || w.addr.is_some() || w.rename.is_some();
    if editing && !plain(Key::Escape) {
        return;
    }

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
        // Esc 的第一优先级是关掉浮层（下拉/右键菜单），其次才清搜索/取消选择
        if w.any_popup() {
            w.popup = None;
            w.crumb_menu = None;
            return;
        }
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
        let vis = w.cur_mut().visible();
        let t = w.cur_mut();
        t.sel.clear();
        for &i in vis.iter() {
            t.sel.insert(i);
        }
        return;
    }
    if ctrl(Key::L, false) {
        // 地址栏直接输入路径
        let p = w.cur().path.to_string_lossy().to_string();
        w.addr = Some(if p == "\\\\" { String::new() } else { p });
        w.addr_focus = true;
        return;
    }
    if ctrl(Key::I, false) {
        w.details = !w.details;
        return;
    }
    if ctrl(Key::B, false) {
        w.sidebar = !w.sidebar;
        return;
    }
    if ctrl(Key::N, true) {
        new_item(w, shared, false);
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
            // 「此电脑」里选的是驱动器，entries 为空，得走 drives
            if let Some((_, dp)) = shared.drives.get(sel[0]).cloned() {
                if w.cur().entries.is_empty() {
                    w.cur_mut().navigate(dp);
                    return;
                }
            }
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
        // 根视图：驱动器是合成列表，直接按 drives 下标走
        if w.cur().entries.is_empty() && !shared.drives.is_empty() {
            let n = shared.drives.len();
            let cur = w.cur().sel.iter().cloned().max();
            let pos = match cur {
                Some(c) if c < n => if down { (c + 1).min(n - 1) } else { c.saturating_sub(1) },
                _ => 0,
            };
            let t = w.cur_mut();
            t.sel.clear();
            t.sel.insert(pos);
            t.sel_last = Some(pos);
            return;
        }
        let view = w.cur_mut().visible();
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
            sel_once: true,
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
    // 用居中的 Area 而不是 Window：Window 默认可缩放、还会记住上次尺寸，
    // 结果就是一个带 resize 斜纹、下半截全空白的大方框。
    egui::Area::new(Id::new(("rename", w.id)))
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            Frame::none()
                .fill(SURF)
                .corner_radius(12.0)
                .inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 16 })
                .stroke((1.0, SEP2))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 12],
                    blur: 28,
                    spread: 0,
                    color: Color32::from_rgba_unmultiplied(10, 14, 20, 52),
                })
                .show(ui, |ui| {
                    ui.set_width(320.0);
                    ui.label(RichText::new("重命名").size(14.0).color(FG));
                    ui.add_space(10.0);
                    if let Some(rn) = w.rename.as_mut() {
                        let resp = ui.add(
                            TextEdit::singleline(&mut rn.buf)
                                .desired_width(320.0)
                                .font(TextStyle::Body),
                        );
                        if rn.focus_once {
                            resp.request_focus();
                            rn.focus_once = false;
                        }
                        // 预选中文件名主体（不含扩展名），直接打字即可替换
                        if rn.sel_once && resp.has_focus() {
                            if let Some(mut st) = TextEdit::load_state(ui.ctx(), resp.id) {
                                let n = rn.buf.chars().count();
                                let dot = rn.buf.rfind('.').unwrap_or(n);
                                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(dot),
                                )));
                                st.store(ui.ctx(), resp.id);
                            }
                            rn.sel_once = false;
                        }
                    }
                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                        ok = true;
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        cancel = true;
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new("Enter 确认 · Esc 取消").size(10.5).color(MUTE));
                    ui.add_space(12.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if accent_btn(ui, "确定", true).clicked() {
                            ok = true;
                        }
                        if accent_btn(ui, "取消", false).clicked() {
                            cancel = true;
                        }
                    });
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
            w.rename = Some(Rename { target: p, buf: name, focus_once: true, sel_once: true, born: shared.t() });
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

//! 视觉层：手绘图标 + 基础控件。
//!
//! 为什么不引图标字体/图片：单个 exe、零资源依赖，而且矢量绘制在任何 DPI 下都清晰。
//! 图标全部由矩形/圆/线段拼出来，尺寸以传入的 rect 为准（按高度等比缩）。

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
};

use crate::{ACC, BG2, FG, FG2, HOVER, LINE, LINE2, MUTE, SURF, estimate_width};

// ---------------------------------------------------------------- 几何图标

/// 文件夹：后盖（带小舌）+ 前盖，两层色阶让它在 16px 也认得出
pub fn folder_glyph(p: &egui::Painter, r: Rect, color: Color32) {
    let h = r.height();
    let w = r.width();
    let tongue_h = (h * 0.22).max(1.0);
    let tongue_w = (w * 0.46).max(2.0);
    // 后盖
    let back = Rect::from_min_size(r.min, Vec2::new(tongue_w, tongue_h * 1.9));
    p.rect_filled(back, 1.5, color.gamma_multiply(0.82));
    // 前盖
    let body = Rect::from_min_max(r.min + Vec2::new(0.0, tongue_h * 0.9), r.max);
    p.rect_filled(body, 2.0, color);
    // 顶部高光
    let gloss = Rect::from_min_max(
        body.min + Vec2::new(w * 0.08, h * 0.06),
        Pos2::new(body.max.x - w * 0.08, body.min.y + h * 0.22),
    );
    p.rect_filled(gloss, 1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 70));
}

/// 磁盘：机身 + 底部指示灯
pub fn drive_glyph(p: &egui::Painter, r: Rect, color: Color32) {
    let body = Rect::from_min_max(
        Pos2::new(r.min.x, r.min.y + r.height() * 0.16),
        Pos2::new(r.max.x, r.max.y - r.height() * 0.16),
    );
    p.rect_filled(body, 2.5, color.gamma_multiply(0.9));
    p.rect_filled(
        Rect::from_min_max(Pos2::new(body.min.x, body.min.y), Pos2::new(body.max.x, body.min.y + body.height() * 0.42)),
        2.0,
        Color32::from_rgba_unmultiplied(255, 255, 255, 46),
    );
    let led = Rect::from_min_size(
        Pos2::new(body.min.x + 2.5, body.max.y - body.height() * 0.26),
        Vec2::new(body.width() * 0.34, body.height() * 0.10),
    );
    p.rect_filled(led, 0.5, Color32::from_rgba_unmultiplied(255, 255, 255, 120));
}

/// 文件：纸张 + 折角 + 类型色条
pub fn file_glyph(p: &egui::Painter, r: Rect, color: Color32) {
    let fold = (r.width() * 0.34).max(2.0);
    let paper = Rect::from_min_max(r.min + Vec2::new(0.5, 0.0), r.max - Vec2::new(0.5, 0.0));
    p.rect_filled(paper, 2.0, Color32::from_rgba_unmultiplied(255, 255, 255, 235));
    p.rect_stroke(paper, 2.0, Stroke::new(1.0, color.gamma_multiply(0.55)), StrokeKind::Inside);
    // 右上折角
    let tri = vec![
        Pos2::new(paper.max.x - fold, paper.min.y + 1.0),
        Pos2::new(paper.max.x - 1.0, paper.min.y + fold),
        Pos2::new(paper.max.x - fold, paper.min.y + fold),
    ];
    p.add(egui::Shape::convex_polygon(
        tri,
        Color32::from_rgba_unmultiplied(0, 0, 0, 0),
        Stroke::new(1.0, color.gamma_multiply(0.55)),
    ));
    // 文本行（两条淡线）+ 底部类型色条
    let inset = r.width() * 0.18;
    for k in 0..2 {
        let y = paper.min.y + paper.height() * (0.42 + 0.18 * k as f32);
        p.line_segment(
            [Pos2::new(paper.min.x + inset, y), Pos2::new(paper.max.x - inset - fold * 0.2, y)],
            Stroke::new(1.0, color.gamma_multiply(0.45)),
        );
    }
    let bar = Rect::from_min_max(
        Pos2::new(paper.min.x + 1.0, paper.max.y - r.height() * 0.16),
        Pos2::new(paper.max.x - 1.0, paper.max.y - 1.0),
    );
    p.rect_filled(bar, 1.0, color);
}

/// 显示器：「此电脑」入口
pub fn pc_glyph(p: &egui::Painter, r: Rect, color: Color32) {
    let scr = Rect::from_min_max(
        Pos2::new(r.min.x, r.min.y + r.height() * 0.12),
        Pos2::new(r.max.x, r.max.y - r.height() * 0.3),
    );
    p.rect_stroke(scr, 2.0, Stroke::new(1.4, color), StrokeKind::Inside);
    let cx = r.center().x;
    p.line_segment(
        [Pos2::new(cx - r.width() * 0.16, scr.max.y + r.height() * 0.16), Pos2::new(cx + r.width() * 0.16, scr.max.y + r.height() * 0.16)],
        Stroke::new(1.4, color),
    );
    p.line_segment(
        [Pos2::new(cx, scr.max.y), Pos2::new(cx, scr.max.y + r.height() * 0.16)],
        Stroke::new(1.4, color),
    );
}

/// 圆形玻璃块：详情面板/此电脑里的大图标底座
pub fn icon_tile(p: &egui::Painter, r: Rect, tint: Color32) {
    p.rect_filled(r, 10.0, Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 26));
    p.rect_stroke(
        r,
        10.0,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 46)),
        StrokeKind::Inside,
    );
}

pub fn chevron(p: &egui::Painter, c: Pos2, size: f32, color: Color32, open: bool) {
    let s = size * 0.5;
    let (a, b, d) = if open {
        (Pos2::new(c.x - s, c.y - s * 0.55), Pos2::new(c.x, c.y + s * 0.5), Pos2::new(c.x + s, c.y - s * 0.55))
    } else {
        (Pos2::new(c.x - s * 0.55, c.y - s), Pos2::new(c.x + s * 0.5, c.y), Pos2::new(c.x - s * 0.55, c.y + s))
    };
    p.line_segment([a, b], Stroke::new(1.4, color));
    p.line_segment([b, d], Stroke::new(1.4, color));
}

/// 排序箭头：三角形
pub fn caret(p: &egui::Painter, c: Pos2, size: f32, desc: bool, color: Color32) {
    let s = size * 0.5;
    let pts = if desc {
        vec![
            Pos2::new(c.x - s, c.y - s * 0.6),
            Pos2::new(c.x + s, c.y - s * 0.6),
            Pos2::new(c.x, c.y + s * 0.7),
        ]
    } else {
        vec![
            Pos2::new(c.x - s, c.y + s * 0.6),
            Pos2::new(c.x + s, c.y + s * 0.6),
            Pos2::new(c.x, c.y - s * 0.7),
        ]
    };
    p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

pub fn search_glyph(p: &egui::Painter, c: Pos2, size: f32, color: Color32) {
    let r = size * 0.30;
    let center = Pos2::new(c.x - size * 0.06, c.y - size * 0.06);
    p.circle_stroke(center, r, Stroke::new(1.4, color));
    let d = r * 0.78;
    p.line_segment(
        [center + Vec2::new(d, d), center + Vec2::new(d * 1.85, d * 1.85)],
        Stroke::new(1.4, color),
    );
}

pub fn plus_glyph(p: &egui::Painter, c: Pos2, size: f32, color: Color32) {
    let s = size * 0.42;
    p.line_segment([Pos2::new(c.x - s, c.y), Pos2::new(c.x + s, c.y)], Stroke::new(1.5, color));
    p.line_segment([Pos2::new(c.x, c.y - s), Pos2::new(c.x, c.y + s)], Stroke::new(1.5, color));
}

pub fn cross_glyph(p: &egui::Painter, c: Pos2, size: f32, color: Color32) {
    let s = size * 0.34;
    p.line_segment([c + Vec2::new(-s, -s), c + Vec2::new(s, s)], Stroke::new(1.3, color));
    p.line_segment([c + Vec2::new(-s, s), c + Vec2::new(s, -s)], Stroke::new(1.3, color));
}

// ---------------------------------------------------------------- 基础控件

/// 竖向像素发丝线（1px 物理像素的近似：0.5pt 描边）
pub fn hairline(p: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    p.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], Stroke::new(0.5, color));
}

pub fn vline(p: &egui::Painter, x: f32, y0: f32, y1: f32, color: Color32) {
    p.line_segment([Pos2::new(x, y0), Pos2::new(x, y1)], Stroke::new(1.0, color));
}

/// 分组小标题（侧栏 / 详情面板）
pub fn section_label(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    ui.painter().text(
        Pos2::new(rect.min.x + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(11.0),
        MUTE,
    );
}

/// 容量条：底槽 + 填充（frac 0..1）
pub fn usage_bar(p: &egui::Painter, r: Rect, frac: f32, color: Color32) {
    let f = frac.clamp(0.0, 1.0);
    p.rect_filled(r, r.height() / 2.0, Color32::from_rgba_unmultiplied(0, 0, 0, 24));
    if f > 0.0 {
        let mut fill = r;
        fill.set_width((r.width() * f).max(r.height()));
        p.rect_filled(fill, r.height() / 2.0, color);
    }
}

/// 计数胶囊（分组表头右侧的小数字）
pub fn count_pill(ui: &mut Ui, cur: Pos2, n: usize) -> Rect {
    let txt = n.to_string();
    let w = estimate_width(&txt) + 12.0;
    let r = Rect::from_min_size(Pos2::new(cur.x, cur.y - 8.0), Vec2::new(w, 16.0));
    ui.painter().rect_filled(r, 8.0, BG2);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, txt, FontId::proportional(10.5), FG2);
    r
}

/// 圆角命令按钮：可选前置图标（几何绘制），hover 淡灰，按下变深
pub fn cmd_button(ui: &mut Ui, glyph: Option<&str>, label: &str, enabled: bool) -> Response {
    let label_w = if label.is_empty() { 0.0 } else { estimate_width(label) + 6.0 };
    let w = 14.0 + label_w + if glyph.is_some() { 16.0 } else { 0.0 };
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), sense);
    let hov = resp.hovered() && enabled;
    let down = resp.is_pointer_button_down_on() && enabled;
    if hov || down {
        ui.painter()
            .rect_filled(rect, 6.0, if down { Color32::from_rgba_unmultiplied(0, 0, 0, 26) } else { HOVER });
    }
    let col = if !enabled {
        Color32::from_rgba_unmultiplied(MUTE.r(), MUTE.g(), MUTE.b(), 120)
    } else if hov {
        FG
    } else {
        FG2
    };
    let mut x = rect.min.x + 8.0;
    if let Some(g) = glyph {
        ui.painter().text(
            Pos2::new(x, rect.center().y),
            Align2::LEFT_CENTER,
            g,
            FontId::proportional(12.5),
            col,
        );
        x += 14.0;
    }
    if !label.is_empty() {
        ui.painter().text(
            Pos2::new(x, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.5),
            col,
        );
    }
    resp
}

/// 图标按钮（正方形，几何图标或字形）
pub fn icon_button(ui: &mut Ui, glyph: &str, size: f32, enabled: bool, tip: &str) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(size, size), sense);
    let hov = resp.hovered() && enabled;
    if hov {
        ui.painter().rect_filled(rect, 6.0, HOVER);
    }
    let col = if !enabled {
        Color32::from_rgba_unmultiplied(MUTE.r(), MUTE.g(), MUTE.b(), 110)
    } else if hov {
        FG
    } else {
        FG2
    };
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), col);
    if !tip.is_empty() {
        resp.on_hover_text(tip)
    } else {
        resp
    }
}

/// 详情面板里的「图标方块 + 三行」规格块
pub fn spec_block(ui: &mut Ui, tint: Color32, glyph: Glyph, label: &str, value: &str, sub: &str) {
    let w = ui.available_width();
    ui.horizontal(|ui| {
        let (tile, _) = ui.allocate_exact_size(Vec2::new(34.0, 34.0), Sense::hover());
        icon_tile(ui.painter(), tile, tint);
        let g = Rect::from_center_size(tile.center(), Vec2::splat(17.0));
        match glyph {
            Glyph::Folder => folder_glyph(ui.painter(), g, tint),
            Glyph::Drive => drive_glyph(ui.painter(), g, tint),
            Glyph::File => file_glyph(ui.painter(), g, tint),
            Glyph::Pc => pc_glyph(ui.painter(), g, tint),
        }
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(egui::RichText::new(label).size(11.0).color(MUTE));
            let name = crate::truncate_width(value, w - 54.0);
            ui.label(egui::RichText::new(name).size(12.5).color(FG));
            if !sub.is_empty() {
                let s = crate::truncate_width(sub, w - 54.0);
                ui.label(egui::RichText::new(s).size(11.0).color(FG2));
            }
        });
    });
}

#[derive(Clone, Copy)]
pub enum Glyph {
    Folder,
    Drive,
    File,
    Pc,
}

/// 圆角输入框背景（搜索/地址栏共用），返回内部可用文本区
pub fn field_rect(p: &egui::Painter, r: Rect, focused: bool) -> Rect {
    p.rect_filled(r, 6.0, SURF);
    p.rect_stroke(
        r,
        6.0,
        Stroke::new(1.0, if focused { ACC } else { LINE2 }),
        StrokeKind::Inside,
    );
    r.shrink2(Vec2::new(8.0, 0.0))
}

/// 浮层阴影（用几层递减 alpha 的圆角矩形近似，egui 无原生 blur）
pub fn soft_shadow(p: &egui::Painter, r: Rect, radius: f32) {
    for i in 0..4 {
        let k = (i as f32 + 1.0) * 1.5;
        let a = 10 - i * 2;
        p.rect_filled(
            r.translate(Vec2::new(0.0, k * 0.5)) .expand(k),
            radius + k,
            Color32::from_rgba_unmultiplied(15, 20, 30, a as u8),
        );
    }
    let _ = LINE;
}

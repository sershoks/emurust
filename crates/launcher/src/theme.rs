//! Thème « néon rétro » du launcher : couleurs, fond animé, scanlines CRT,
//! panneaux lumineux et pastilles de touches.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke,
    StrokeKind, Vec2, epaint::Shadow,
};

pub const BG_TOP: Color32 = Color32::from_rgb(22, 10, 46);
pub const BG_BOTTOM: Color32 = Color32::from_rgb(4, 3, 14);
pub const NEON_CYAN: Color32 = Color32::from_rgb(0, 229, 255);
pub const NEON_PINK: Color32 = Color32::from_rgb(255, 46, 151);
pub const NEON_ORANGE: Color32 = Color32::from_rgb(255, 145, 50);
pub const TEXT: Color32 = Color32::from_rgb(232, 230, 255);
pub const TEXT_DIM: Color32 = Color32::from_rgb(140, 136, 186);
pub const PANEL: Color32 = Color32::from_rgba_premultiplied(14, 10, 32, 225);

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

pub fn prop(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Couleur avec une opacité de 0,0 à 1,0.
pub fn alpha(color: Color32, opacity: f32) -> Color32 {
    color.gamma_multiply(opacity.clamp(0.0, 1.0))
}

/// Réglages globaux d'egui pour coller au thème (sombre, coins arrondis).
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG_BOTTOM;
    visuals.window_fill = PANEL;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = alpha(NEON_PINK, 0.6);
    ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
    ctx.set_visuals_of(egui::Theme::Light, visuals);
}

fn vertical_gradient(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(Shape::mesh(mesh));
}

/// Fond complet : ciel dégradé, soleil couchant à bandes et sol quadrillé
/// qui défile vers le spectateur.
pub fn background(painter: &Painter, rect: Rect, time: f64) {
    let horizon = rect.top() + rect.height() * 0.64;
    let sky = Rect::from_min_max(rect.left_top(), Pos2::new(rect.right(), horizon));
    let floor = Rect::from_min_max(Pos2::new(rect.left(), horizon), rect.right_bottom());

    vertical_gradient(painter, sky, BG_TOP, Color32::from_rgb(60, 14, 70));
    vertical_gradient(painter, floor, Color32::from_rgb(18, 6, 36), BG_BOTTOM);

    // Soleil : un disque coupé de bandes horizontales de plus en plus larges.
    let sun_center = Pos2::new(rect.center().x, horizon - rect.height() * 0.02);
    let sun_radius = rect.height() * 0.2;
    let sun_painter = painter.with_clip_rect(sky);
    sun_painter.circle_filled(sun_center, sun_radius * 1.25, alpha(NEON_PINK, 0.08));
    sun_painter.circle_filled(sun_center, sun_radius, alpha(NEON_ORANGE, 0.55));
    sun_painter.circle_filled(
        sun_center + Vec2::new(0.0, sun_radius * 0.35),
        sun_radius * 0.9,
        alpha(NEON_PINK, 0.35),
    );
    for i in 0..6 {
        let y = sun_center.y - sun_radius * 0.05 + i as f32 * sun_radius * 0.17;
        let h = 2.0 + i as f32 * 1.6;
        sun_painter.rect_filled(
            Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), h)),
            0.0,
            Color32::from_rgb(52, 13, 66),
        );
    }

    // Sol : lignes horizontales en perspective qui avancent avec le temps.
    let floor_painter = painter.with_clip_rect(floor);
    let grid = alpha(NEON_PINK, 0.55);
    let phase = (time * 0.35).fract() as f32;
    let lines = 12;
    for i in 0..lines {
        let f = (i as f32 + phase) / lines as f32;
        let y = horizon + floor.height() * f * f;
        floor_painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0 + f * 1.5, alpha(grid, 0.25 + f * 0.75)),
        );
    }
    // Lignes de fuite vers le centre de l'horizon.
    let spacing = rect.width() / 9.0;
    for j in -14..=14 {
        let bottom_x = rect.center().x + j as f32 * spacing;
        let top_x = rect.center().x + j as f32 * spacing * 0.06;
        floor_painter.line_segment(
            [
                Pos2::new(top_x, horizon),
                Pos2::new(bottom_x, rect.bottom()),
            ],
            Stroke::new(1.2, alpha(grid, 0.45)),
        );
    }
    // Lueur de l'horizon.
    painter.line_segment(
        [
            Pos2::new(rect.left(), horizon),
            Pos2::new(rect.right(), horizon),
        ],
        Stroke::new(2.0, alpha(NEON_PINK, 0.9)),
    );
    vertical_gradient(
        painter,
        Rect::from_min_max(
            Pos2::new(rect.left(), horizon - 40.0),
            Pos2::new(rect.right(), horizon),
        ),
        Color32::TRANSPARENT,
        alpha(NEON_PINK, 0.18),
    );
}

/// Lignes sombres horizontales, comme sur un écran cathodique.
pub fn scanlines(painter: &Painter, rect: Rect, spacing: f32, opacity: f32) {
    let color = Color32::from_black_alpha((opacity * 255.0) as u8);
    let mut y = rect.top();
    while y < rect.bottom() {
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.left(), y),
                Vec2::new(rect.width(), spacing * 0.4),
            ),
            0.0,
            color,
        );
        y += spacing;
    }
}

/// Texte avec halo lumineux.
pub fn neon_text(
    painter: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    color: Color32,
) -> Rect {
    for (dx, dy) in [(-2.0, 0.0), (2.0, 0.0), (0.0, -2.0), (0.0, 2.0)] {
        painter.text(
            pos + Vec2::new(dx, dy),
            anchor,
            text,
            font.clone(),
            alpha(color, 0.18),
        );
    }
    painter.text(pos, anchor, text, font, color)
}

/// Panneau sombre translucide, bordé d'un trait néon et entouré d'une lueur.
pub fn glow_panel(painter: &Painter, rect: Rect, accent: Color32, glow: f32, radius: u8) {
    if glow > 0.0 {
        let shadow = Shadow {
            offset: [0, 0],
            blur: (28.0 * glow) as u8,
            spread: (2.0 * glow) as u8,
            color: alpha(accent, 0.45 * glow),
        };
        painter.add(shadow.as_shape(rect, radius));
    }
    painter.rect_filled(rect, radius, PANEL);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.5 + glow, alpha(accent, 0.35 + 0.65 * glow)),
        StrokeKind::Inside,
    );
}

/// Pastille de touche suivie de sa légende, ex. `[ENTRÉE] Jouer`.
/// Renvoie la largeur occupée.
pub fn key_hint(painter: &Painter, left_center: Pos2, key: &str, label: &str) -> f32 {
    let key_galley = painter.layout_no_wrap(key.to_owned(), mono(13.0), BG_BOTTOM);
    let pill = Rect::from_min_size(
        Pos2::new(left_center.x, left_center.y - 11.0),
        Vec2::new(key_galley.size().x + 16.0, 22.0),
    );
    painter.rect_filled(pill, CornerRadius::same(11), NEON_CYAN);
    painter.galley(
        Pos2::new(
            pill.left() + 8.0,
            pill.center().y - key_galley.size().y / 2.0,
        ),
        key_galley,
        BG_BOTTOM,
    );
    let text_rect = painter.text(
        Pos2::new(pill.right() + 8.0, left_center.y),
        Align2::LEFT_CENTER,
        label,
        prop(14.0),
        TEXT,
    );
    text_rect.right() - left_center.x
}

/// Barre d'aide en bas de l'écran, comme sur une console.
pub fn footer(painter: &Painter, rect: Rect, hints: &[(&str, &str)]) {
    vertical_gradient(
        painter,
        rect,
        Color32::TRANSPARENT,
        Color32::from_black_alpha(220),
    );
    painter.line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0, alpha(NEON_CYAN, 0.25)),
    );
    let mut x = rect.left() + 24.0;
    for (key, label) in hints {
        x += key_hint(painter, Pos2::new(x, rect.center().y), key, label) + 28.0;
    }
}

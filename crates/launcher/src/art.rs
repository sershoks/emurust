//! Illustrations des consoles, dessinées entièrement en code
//! (aucune image ni logo externe).

use eframe::egui::{Color32, CornerRadius, Painter, Rect, Stroke, StrokeKind, Vec2};

use crate::theme::{self, alpha};

/// Console portable stylisée : coque sombre bordée de néon, écran allumé,
/// croix directionnelle, deux boutons et grille de haut-parleur.
pub fn handheld(painter: &Painter, rect: Rect, accent: Color32, screen: Color32, time: f64) {
    // Proportions d'une portable verticale (largeur / hauteur ≈ 0,62).
    let height = rect.height().min(rect.width() / 0.62);
    let body = Rect::from_center_size(rect.center(), Vec2::new(height * 0.62, height));
    let u = body.width() / 100.0; // unité de dessin

    painter.rect_filled(
        body,
        CornerRadius {
            nw: 12,
            ne: 12,
            sw: 12,
            se: 48,
        },
        theme::PANEL,
    );
    painter.rect_stroke(
        body,
        CornerRadius {
            nw: 12,
            ne: 12,
            sw: 12,
            se: 48,
        },
        Stroke::new(2.5, accent),
        StrokeKind::Inside,
    );

    // Cadre et écran, qui « respire » doucement.
    let bezel = Rect::from_min_size(
        body.min + Vec2::new(10.0 * u, 10.0 * u),
        Vec2::new(80.0 * u, 66.0 * u),
    );
    painter.rect_filled(
        bezel,
        CornerRadius {
            nw: 6,
            ne: 6,
            sw: 6,
            se: 24,
        },
        Color32::from_rgb(10, 8, 24),
    );
    painter.rect_stroke(
        bezel,
        CornerRadius {
            nw: 6,
            ne: 6,
            sw: 6,
            se: 24,
        },
        Stroke::new(1.0, alpha(accent, 0.5)),
        StrokeKind::Inside,
    );
    let pulse = 0.75 + 0.25 * ((time * 1.5).sin() as f32 * 0.5 + 0.5);
    let display = Rect::from_min_size(
        bezel.min + Vec2::new(12.0 * u, 8.0 * u),
        Vec2::new(56.0 * u, 50.0 * u),
    );
    painter.rect_filled(display, 2, alpha(screen, pulse));
    theme::scanlines(painter, display, 3.0, 0.25);
    // Voyant d'alimentation.
    painter.circle_filled(
        bezel.min + Vec2::new(5.0 * u, 24.0 * u),
        1.6 * u,
        theme::NEON_PINK,
    );

    // Croix directionnelle.
    let dpad = body.min + Vec2::new(24.0 * u, 100.0 * u);
    let arm = Vec2::new(18.0 * u, 6.0 * u);
    painter.rect_filled(Rect::from_center_size(dpad, arm), 2, alpha(accent, 0.9));
    painter.rect_filled(
        Rect::from_center_size(dpad, Vec2::new(arm.y, arm.x)),
        2,
        alpha(accent, 0.9),
    );

    // Boutons A et B.
    for (dx, dy) in [(80.0, 94.0), (66.0, 102.0)] {
        let center = body.min + Vec2::new(dx * u, dy * u);
        painter.circle_filled(center, 6.0 * u, alpha(theme::NEON_PINK, 0.9));
        painter.circle_stroke(center, 6.0 * u, Stroke::new(1.0, theme::NEON_PINK));
    }

    // Start / Select.
    for dx in [38.0, 54.0] {
        let center = body.min + Vec2::new(dx * u, 124.0 * u);
        painter.rect_filled(
            Rect::from_center_size(center, Vec2::new(10.0 * u, 3.0 * u)),
            2,
            alpha(accent, 0.6),
        );
    }

    // Haut-parleur.
    for i in 0..5 {
        let start = body.min + Vec2::new((70.0 + i as f32 * 5.0) * u, 150.0 * u);
        painter.line_segment(
            [start, start + Vec2::new(4.0 * u, -10.0 * u)],
            Stroke::new(1.5 * u, alpha(accent, 0.6)),
        );
    }
}

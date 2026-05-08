use egui::{Color32, FontId, Pos2, Response, Sense, Stroke, Ui, Vec2};

const SNAP_RADIUS: f32 = 10.0;

pub struct FanCurveWidget<'a> {
    points: &'a mut Vec<(f32, f32)>,
    current_temp: f32,
    dragging: &'a mut Option<usize>,
    view_temp: &'a mut (f32, f32),
}

impl<'a> FanCurveWidget<'a> {
    pub fn new(
        points: &'a mut Vec<(f32, f32)>,
        current_temp: f32,
        dragging: &'a mut Option<usize>,
        view_temp: &'a mut (f32, f32),
    ) -> Self {
        Self { points, current_temp, dragging, view_temp }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let desired = Vec2::new(ui.available_width(), 220.0);
        let (response, painter) = ui.allocate_painter(desired, Sense::click_and_drag());
        let rect = response.rect;

        let vt = *self.view_temp;
        let grid_color = Color32::from_rgb(50, 50, 70);
        let label_font = FontId::proportional(10.0);

        painter.rect_filled(rect, 4.0, Color32::from_rgb(20, 20, 30));

        // Vertical grid + X labels (temperature, adaptive ticks)
        for t in nice_ticks(vt.0, vt.1, 6) {
            let x = rect.left() + (t - vt.0) / (vt.1 - vt.0) * rect.width();
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, grid_color),
            );
            painter.text(
                Pos2::new(x, rect.bottom() + 4.0),
                egui::Align2::CENTER_TOP,
                format!("{t:.0}°C"),
                label_font.clone(),
                Color32::GRAY,
            );
        }

        // Horizontal grid + Y labels (fan speed %, fixed)
        for s in [0u32, 25, 50, 75, 100] {
            let y = rect.bottom() - (s as f32 / 100.0) * rect.height();
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(1.0, grid_color),
            );
            painter.text(
                Pos2::new(rect.left() - 4.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{s}%"),
                label_font.clone(),
                Color32::GRAY,
            );
        }

        let to_screen = |temp: f32, speed: f32| -> Pos2 {
            Pos2::new(
                rect.left() + (temp - vt.0) / (vt.1 - vt.0) * rect.width(),
                rect.bottom() - (speed / 100.0) * rect.height(),
            )
        };

        let to_data = |pos: Pos2| -> (f32, f32) {
            let temp = (vt.0 + (pos.x - rect.left()) / rect.width() * (vt.1 - vt.0))
                .clamp(vt.0, vt.1);
            let speed = ((rect.bottom() - pos.y) / rect.height() * 100.0).clamp(0.0, 100.0);
            (temp, speed)
        };

        // Scroll to zoom around cursor position
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                let factor = (1.0 - scroll * 0.008).clamp(0.5, 2.0);
                let pivot = response
                    .hover_pos()
                    .map(|p| vt.0 + (p.x - rect.left()) / rect.width() * (vt.1 - vt.0))
                    .unwrap_or((vt.0 + vt.1) / 2.0);
                let new_min = (pivot - (pivot - vt.0) * factor).max(0.0);
                let new_max = (pivot + (vt.1 - pivot) * factor).min(100.0);
                if new_max - new_min > 5.0 {
                    *self.view_temp = (new_min, new_max);
                }
            }
        }

        // Drag interaction
        if let Some(cursor) = response.interact_pointer_pos() {
            if response.drag_started() {
                let nearest = self
                    .points
                    .iter()
                    .enumerate()
                    .map(|(i, &(t, s))| (i, to_screen(t, s).distance(cursor)))
                    .filter(|(_, d)| *d < SNAP_RADIUS * 2.0)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                *self.dragging = nearest.map(|(i, _)| i);
            }
            if response.dragged() {
                if let Some(idx) = *self.dragging {
                    self.points[idx] = {
                    let (temp, mut speed) = to_data(cursor);
                    // Ensure monotonic speed: clamp between neighboring points
                    if idx > 0 {
                        let prev_speed = self.points[idx - 1].1;
                        if speed < prev_speed { speed = prev_speed; }
                    }
                    if idx + 1 < self.points.len() {
                        let next_speed = self.points[idx + 1].1;
                        if speed > next_speed { speed = next_speed; }
                    }
                    (temp, speed)
                };
                }
            }
        }
        if response.drag_stopped() {
            *self.dragging = None;
            self.points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        }

        // Connecting curve
        let screen_pts: Vec<Pos2> =
            self.points.iter().map(|&(t, s)| to_screen(t, s)).collect();
        for w in screen_pts.windows(2) {
            painter.line_segment([w[0], w[1]], Stroke::new(2.0, Color32::from_rgb(100, 200, 255)));
        }

        // Control points
        for (i, &(t, s)) in self.points.iter().enumerate() {
            let pos = to_screen(t, s);
            let active = *self.dragging == Some(i);
            let color = if active { Color32::WHITE } else { Color32::from_rgb(100, 200, 255) };
            let r = if active { 8.0 } else { 6.0 };
            painter.circle_filled(pos, r, color);
            painter.circle_stroke(pos, r, Stroke::new(1.5, Color32::WHITE));
        }

        // Value label for the dragged point, or the nearest hovered point
        let label_idx = self.dragging.or_else(|| {
            response.hover_pos().and_then(|cursor| {
                self.points
                    .iter()
                    .enumerate()
                    .map(|(i, &(t, s))| (i, to_screen(t, s).distance(cursor)))
                    .filter(|(_, d)| *d < SNAP_RADIUS * 3.0)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                    .map(|(i, _)| i)
            })
        });
        if let Some(idx) = label_idx {
            let (t, s) = self.points[idx];
            let pos = to_screen(t, s);
            painter.text(
                pos + Vec2::new(10.0, -10.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{t:.0}°C  {s:.0}%"),
                FontId::proportional(11.0),
                Color32::WHITE,
            );
        }

        // Live temp indicator
        if self.current_temp > vt.0 && self.current_temp < vt.1 {
            let x = rect.left() + (self.current_temp - vt.0) / (vt.1 - vt.0) * rect.width();
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.5, Color32::from_rgb(255, 80, 80)),
            );
            painter.text(
                Pos2::new(x + 3.0, rect.top() + 4.0),
                egui::Align2::LEFT_TOP,
                format!("{:.0}°C", self.current_temp),
                FontId::proportional(10.0),
                Color32::from_rgb(255, 120, 120),
            );
        }

        painter.rect_stroke(rect, 4.0, Stroke::new(1.0, Color32::from_rgb(70, 70, 90)));

        response
    }
}

/// Returns nicely-rounded tick values for a given range.
fn nice_ticks(min: f32, max: f32, target_count: usize) -> Vec<f32> {
    let range = (max - min).abs();
    if range < 1e-6 {
        return vec![];
    }
    let raw_step = range / target_count as f32;
    let magnitude = 10f32.powf(raw_step.log10().floor());
    let step = [1.0f32, 2.0, 5.0, 10.0]
        .iter()
        .map(|&s| s * magnitude)
        .find(|&s| s >= raw_step)
        .unwrap_or(10.0 * magnitude);
    let mut ticks = Vec::new();
    let mut t = (min / step).ceil() * step;
    while t <= max + step * 0.01 {
        ticks.push(t);
        t += step;
    }
    ticks
}

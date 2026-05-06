use egui::{Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};

const SNAP_RADIUS: f32 = 10.0;

pub struct FanCurveWidget<'a> {
    points: &'a mut Vec<(f32, f32)>,
    current_temp: f32,
    dragging: &'a mut Option<usize>,
}

impl<'a> FanCurveWidget<'a> {
    pub fn new(
        points: &'a mut Vec<(f32, f32)>,
        current_temp: f32,
        dragging: &'a mut Option<usize>,
    ) -> Self {
        Self { points, current_temp, dragging }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let desired = Vec2::new(ui.available_width(), 220.0);
        let (response, painter) = ui.allocate_painter(desired, Sense::click_and_drag());
        let rect = response.rect;

        // Draw background
        painter.rect_filled(rect, 4.0, Color32::from_rgb(20, 20, 30));

        // Grid lines
        let grid_color = Color32::from_rgb(50, 50, 70);
        for i in 0..=10 {
            let frac = i as f32 / 10.0;
            let x = rect.left() + frac * rect.width();
            let y = rect.top() + frac * rect.height();
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, grid_color),
            );
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(1.0, grid_color),
            );
        }

        // Axis labels
        for i in (0..=100u32).step_by(20) {
            let frac = i as f32 / 100.0;
            let x = rect.left() + frac * rect.width();
            let y = rect.bottom() - frac * rect.height();
            painter.text(
                Pos2::new(x, rect.bottom() + 4.0),
                egui::Align2::CENTER_TOP,
                format!("{i}°"),
                egui::FontId::proportional(10.0),
                Color32::GRAY,
            );
            painter.text(
                Pos2::new(rect.left() - 4.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{i}%"),
                egui::FontId::proportional(10.0),
                Color32::GRAY,
            );
        }

        let to_screen = |temp: f32, speed: f32| -> Pos2 {
            Pos2::new(
                rect.left() + (temp / 100.0) * rect.width(),
                rect.bottom() - (speed / 100.0) * rect.height(),
            )
        };

        let to_data = |pos: Pos2| -> (f32, f32) {
            let temp = ((pos.x - rect.left()) / rect.width() * 100.0).clamp(0.0, 100.0);
            let speed = ((rect.bottom() - pos.y) / rect.height() * 100.0).clamp(0.0, 100.0);
            (temp, speed)
        };

        // Handle drag interaction
        if let Some(cursor) = response.interact_pointer_pos() {
            if response.drag_started() {
                // Find nearest point within snap radius
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
                    let (t, s) = to_data(cursor);
                    self.points[idx] = (t, s);
                }
            }
        }
        if response.drag_stopped() {
            *self.dragging = None;
            self.points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        }

        // Draw connecting curve
        let screen_pts: Vec<Pos2> =
            self.points.iter().map(|&(t, s)| to_screen(t, s)).collect();
        for w in screen_pts.windows(2) {
            painter.line_segment([w[0], w[1]], Stroke::new(2.0, Color32::from_rgb(100, 200, 255)));
        }

        // Draw draggable points
        for (i, &(t, s)) in self.points.iter().enumerate() {
            let pos = to_screen(t, s);
            let is_dragging = *self.dragging == Some(i);
            let color = if is_dragging {
                Color32::WHITE
            } else {
                Color32::from_rgb(100, 200, 255)
            };
            painter.circle_filled(pos, if is_dragging { 8.0 } else { 6.0 }, color);
            painter.circle_stroke(pos, if is_dragging { 8.0 } else { 6.0 }, Stroke::new(1.5, Color32::WHITE));
        }

        // Live temp indicator (vertical red line)
        if self.current_temp > 0.0 {
            let x = rect.left() + (self.current_temp / 100.0) * rect.width();
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.5, Color32::from_rgb(255, 80, 80)),
            );
            painter.text(
                Pos2::new(x + 3.0, rect.top() + 4.0),
                egui::Align2::LEFT_TOP,
                format!("{:.0}°C", self.current_temp),
                egui::FontId::proportional(10.0),
                Color32::from_rgb(255, 120, 120),
            );
        }

        // Border
        painter.rect_stroke(rect, 4.0, Stroke::new(1.0, Color32::from_rgb(70, 70, 90)));

        response
    }
}

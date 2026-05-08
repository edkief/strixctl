use egui::{Color32, FontId, Pos2, Response, Sense, Stroke, Ui, Vec2};

const SNAP_RADIUS: f32 = 10.0;

pub struct FanCurveWidget<'a> {
    points: &'a mut Vec<(f32, f32)>,
    current_temp: f32,
    dragging: &'a mut Option<usize>,
    view_temp: &'a mut (f32, f32),
    selected: &'a mut Vec<usize>,
    select_drag: &'a mut Option<f32>,
}

impl<'a> FanCurveWidget<'a> {
    pub fn new(
        points: &'a mut Vec<(f32, f32)>,
        current_temp: f32,
        dragging: &'a mut Option<usize>,
        view_temp: &'a mut (f32, f32),
        selected: &'a mut Vec<usize>,
        select_drag: &'a mut Option<f32>,
    ) -> Self {
        Self { points, current_temp, dragging, view_temp, selected, select_drag }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        sanitize(self.points);

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

        let nearest_point = |cursor: Pos2| -> Option<usize> {
            self.points
                .iter()
                .enumerate()
                .map(|(i, &(t, s))| (i, to_screen(t, s).distance(cursor)))
                .filter(|(_, d)| *d < SNAP_RADIUS * 2.0)
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .map(|(i, _)| i)
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

        // Cancel selection on click over empty space
        if response.clicked() {
            let on_dot = response
                .interact_pointer_pos()
                .and_then(|c| nearest_point(c))
                .is_some();
            if !on_dot {
                self.selected.clear();
            }
        }

        let shift_held = ui.input(|i| i.modifiers.shift);

        if let Some(cursor) = response.interact_pointer_pos() {
            if response.drag_started() {
                if shift_held {
                    let (temp, _) = to_data(cursor);
                    *self.select_drag = Some(temp);
                    *self.dragging = None;
                } else {
                    match nearest_point(cursor) {
                        Some(idx) => {
                            if !self.selected.contains(&idx) {
                                self.selected.clear();
                            }
                            *self.dragging = Some(idx);
                        }
                        None => {
                            self.selected.clear();
                            *self.dragging = None;
                        }
                    }
                }
            }

            if response.dragged() {
                if let Some(start_temp) = *self.select_drag {
                    // Update rubber-band selection by temperature range
                    let (cur_temp, _) = to_data(cursor);
                    let (lo, hi) = if start_temp <= cur_temp {
                        (start_temp, cur_temp)
                    } else {
                        (cur_temp, start_temp)
                    };
                    *self.selected = self.points
                        .iter()
                        .enumerate()
                        .filter(|&(_, &(t, _))| t >= lo && t <= hi)
                        .map(|(i, _)| i)
                        .collect();
                } else if let Some(idx) = *self.dragging {
                    if self.selected.len() > 1 && self.selected.contains(&idx) {
                        // Group move: both axes
                        let drag_delta = response.drag_delta();
                        let delta_speed = -drag_delta.y / rect.height() * 100.0;
                        let delta_temp = drag_delta.x / rect.width() * (vt.1 - vt.0);
                        let clamped_speed =
                            clamp_group_delta(self.points, self.selected, delta_speed);
                        let clamped_temp =
                            clamp_group_temp_delta(self.points, self.selected, delta_temp, vt);
                        for &i in self.selected.iter() {
                            self.points[i].0 =
                                (self.points[i].0 + clamped_temp).clamp(vt.0, vt.1);
                            self.points[i].1 =
                                (self.points[i].1 + clamped_speed).clamp(0.0, 100.0);
                        }
                    } else {
                        // Single-point drag: move both axes with monotonic constraint
                        self.points[idx] = {
                            let (temp, mut speed) = to_data(cursor);
                            if idx > 0 {
                                let prev = self.points[idx - 1].1;
                                if speed < prev { speed = prev; }
                            }
                            if idx + 1 < self.points.len() {
                                let next = self.points[idx + 1].1;
                                if speed > next { speed = next; }
                            }
                            (temp, speed)
                        };
                    }
                }
            }
        }

        if response.drag_stopped() {
            let was_select_drag = self.select_drag.is_some();
            *self.select_drag = None;
            let is_group_move = self.dragging
                .map(|idx| self.selected.len() > 1 && self.selected.contains(&idx))
                .unwrap_or(false);
            // Only clear selection + re-sort on a plain single-point drag.
            // Rubber-band end and group-move end must keep the selection alive.
            if !was_select_drag && !is_group_move {
                self.points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                self.selected.clear();
            }
            *self.dragging = None;
        }

        // Draw rubber-band selection rectangle
        if let (Some(start_temp), Some(cursor)) =
            (*self.select_drag, response.interact_pointer_pos())
        {
            let (cur_temp, _) = to_data(cursor);
            let x1 = rect.left() + (start_temp - vt.0) / (vt.1 - vt.0) * rect.width();
            let x2 = rect.left() + (cur_temp - vt.0) / (vt.1 - vt.0) * rect.width();
            let sel_rect = egui::Rect::from_x_y_ranges(
                x1.min(x2)..=x1.max(x2),
                rect.top()..=rect.bottom(),
            );
            painter.rect_filled(
                sel_rect,
                0.0,
                Color32::from_rgba_unmultiplied(100, 160, 255, 35),
            );
            painter.rect_stroke(
                sel_rect,
                0.0,
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(100, 160, 255, 160)),
            );
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
            let is_selected = self.selected.contains(&i);
            let active = *self.dragging == Some(i);
            let color = if active {
                Color32::WHITE
            } else if is_selected {
                Color32::from_rgb(255, 210, 60)
            } else {
                Color32::from_rgb(100, 200, 255)
            };
            let r = if active { 8.0 } else if is_selected { 7.0 } else { 6.0 };
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

/// Sorts points by temperature and enforces monotonically non-decreasing fan speed.
/// Any point whose speed is below its predecessor's speed is snapped up to match it.
pub fn sanitize(points: &mut Vec<(f32, f32)>) {
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    for i in 1..points.len() {
        let prev_speed = points[i - 1].1;
        if points[i].1 < prev_speed {
            points[i].1 = prev_speed;
        }
    }
}

/// Clamps a group temperature delta so all selected points stay within the view range
/// and do not cross their non-selected boundary neighbours.
fn clamp_group_temp_delta(
    points: &[(f32, f32)],
    selected: &[usize],
    delta: f32,
    vt: (f32, f32),
) -> f32 {
    if selected.is_empty() {
        return delta;
    }
    let mut lo = -f32::INFINITY;
    let mut hi = f32::INFINITY;

    for &i in selected {
        lo = lo.max(vt.0 - points[i].0);
        hi = hi.min(vt.1 - points[i].0);
    }

    let min_sel = *selected.iter().min().unwrap();
    let max_sel = *selected.iter().max().unwrap();

    if min_sel > 0 {
        lo = lo.max(points[min_sel - 1].0 - points[min_sel].0);
    }
    if max_sel + 1 < points.len() {
        hi = hi.min(points[max_sel + 1].0 - points[max_sel].0);
    }

    if lo > hi { 0.0 } else { delta.clamp(lo, hi) }
}

/// Clamps a group speed delta so all selected points stay in [0, 100] and respect
/// their non-selected boundary neighbours (monotonic constraint).
fn clamp_group_delta(points: &[(f32, f32)], selected: &[usize], delta: f32) -> f32 {
    if selected.is_empty() {
        return delta;
    }
    let mut lo = -f32::INFINITY;
    let mut hi = f32::INFINITY;

    for &i in selected {
        lo = lo.max(-points[i].1);           // speed + delta >= 0
        hi = hi.min(100.0 - points[i].1);    // speed + delta <= 100
    }

    let min_sel = *selected.iter().min().unwrap();
    let max_sel = *selected.iter().max().unwrap();

    if min_sel > 0 {
        // leftmost selected must stay >= its left neighbour
        lo = lo.max(points[min_sel - 1].1 - points[min_sel].1);
    }
    if max_sel + 1 < points.len() {
        // rightmost selected must stay <= its right neighbour
        hi = hi.min(points[max_sel + 1].1 - points[max_sel].1);
    }

    if lo > hi { 0.0 } else { delta.clamp(lo, hi) }
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

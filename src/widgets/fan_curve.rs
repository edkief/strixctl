use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::theme;

const MARGIN: f32 = 36.0;
const HIT_RADIUS: f32 = 14.0;

pub struct FanCurveProgram<'a> {
    pub points: &'a [(f32, f32)],
    pub current_temp: f32,
    pub view_range: (f32, f32),
}

pub fn view<'a>(
    points: &'a [(f32, f32)],
    current_temp: f32,
    view_range: (f32, f32),
) -> iced::Element<'a, Message> {
    Canvas::new(FanCurveProgram {
        points,
        current_temp,
        view_range,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

impl<'a> canvas::Program<Message> for FanCurveProgram<'a> {
    type State = DragState;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let local = cursor.position_in(bounds);
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = local {
                    for (i, p) in self.points.iter().enumerate() {
                        let sp = world_to_screen(*p, bounds.size(), self.view_range);
                        if (sp.x - pos.x).hypot(sp.y - pos.y) < HIT_RADIUS {
                            state.dragging = Some(i);
                            return (canvas::event::Status::Captured, None);
                        }
                    }
                }
                (canvas::event::Status::Ignored, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = None;
                (canvas::event::Status::Ignored, None)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let (Some(idx), Some(pos)) = (state.dragging, local) {
                    let (t, s) = screen_to_world(pos, bounds.size(), self.view_range);
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::FanPointDragged(idx, t, s)),
                    );
                }
                (canvas::event::Status::Ignored, None)
            }
            _ => (canvas::event::Status::Ignored, None),
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = frame.size();

        // Plot background
        frame.fill_rectangle(
            Point::ORIGIN,
            size,
            Color {
                r: 0.07,
                g: 0.07,
                b: 0.10,
                a: 1.0,
            },
        );

        let plot_w = size.width - MARGIN * 2.0;
        let plot_h = size.height - MARGIN * 2.0;
        if plot_w <= 0.0 || plot_h <= 0.0 {
            return vec![frame.into_geometry()];
        }

        let grid = Stroke {
            style: canvas::Style::Solid(with_alpha(theme::SURFACE1, 0.8)),
            width: 1.0,
            ..Stroke::default()
        };

        // Horizontal grid + Y labels (fan %)
        for i in 0..=4 {
            let y = MARGIN + plot_h * (i as f32 / 4.0);
            frame.stroke(
                &Path::line(Point::new(MARGIN, y), Point::new(MARGIN + plot_w, y)),
                grid.clone(),
            );
            let pct = 100 - 25 * i;
            frame.fill_text(Text {
                content: format!("{pct}%"),
                position: Point::new(6.0, y - 7.0),
                color: theme::OVERLAY1,
                size: 11.0.into(),
                ..Text::default()
            });
        }
        // Vertical grid + X labels (°C)
        for i in 0..=4 {
            let x = MARGIN + plot_w * (i as f32 / 4.0);
            frame.stroke(
                &Path::line(Point::new(x, MARGIN), Point::new(x, MARGIN + plot_h)),
                grid.clone(),
            );
            let t = self.view_range.0 + (self.view_range.1 - self.view_range.0) * (i as f32 / 4.0);
            frame.fill_text(Text {
                content: format!("{:.0}°C", t),
                position: Point::new(x - 14.0, size.height - MARGIN + 6.0),
                color: theme::OVERLAY1,
                size: 11.0.into(),
                ..Text::default()
            });
        }

        // Current temp indicator
        if self.current_temp > 0.0 {
            let x = MARGIN + temp_to_x(self.current_temp, plot_w, self.view_range);
            if x >= MARGIN && x <= MARGIN + plot_w {
                frame.stroke(
                    &Path::line(Point::new(x, MARGIN), Point::new(x, MARGIN + plot_h)),
                    Stroke {
                        style: canvas::Style::Solid(theme::RED),
                        width: 2.0,
                        ..Stroke::default()
                    },
                );
                frame.fill_text(Text {
                    content: format!("{:.1}°C", self.current_temp),
                    position: Point::new(x + 4.0, MARGIN + 4.0),
                    color: theme::RED,
                    size: 11.0.into(),
                    ..Text::default()
                });
            }
        }

        // Curve line
        if self.points.len() >= 2 {
            let path = Path::new(|b| {
                let p0 = world_to_screen(self.points[0], size, self.view_range);
                b.move_to(p0);
                for &p in &self.points[1..] {
                    b.line_to(world_to_screen(p, size, self.view_range));
                }
            });
            frame.stroke(
                &path,
                Stroke {
                    style: canvas::Style::Solid(theme::BLUE),
                    width: 2.5,
                    ..Stroke::default()
                },
            );
        }

        // Identify hovered/dragged point so we can highlight it and show a tooltip.
        let highlight = state.dragging.or_else(|| {
            cursor.position_in(bounds).and_then(|pos| {
                self.points
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &p)| {
                        let sp = world_to_screen(p, size, self.view_range);
                        let d = (sp.x - pos.x).hypot(sp.y - pos.y);
                        if d < HIT_RADIUS { Some((i, d)) } else { None }
                    })
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(i, _)| i)
            })
        });

        // Points
        for (i, &p) in self.points.iter().enumerate() {
            let sp = world_to_screen(p, size, self.view_range);
            let active = Some(i) == highlight;
            let radius = if active { 7.5 } else { 6.0 };
            frame.fill(&Path::circle(sp, radius), theme::TEXT);
            frame.stroke(
                &Path::circle(sp, radius),
                Stroke {
                    style: canvas::Style::Solid(if active { theme::MAUVE } else { theme::BLUE }),
                    width: 2.0,
                    ..Stroke::default()
                },
            );
        }

        // Hover/drag tooltip
        if let Some(i) = highlight {
            let p = self.points[i];
            let sp = world_to_screen(p, size, self.view_range);
            draw_tooltip(&mut frame, sp, p, size);
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging.is_some() {
            return mouse::Interaction::Grabbing;
        }
        if let Some(pos) = cursor.position_in(bounds) {
            for &p in self.points {
                let sp = world_to_screen(p, bounds.size(), self.view_range);
                if (sp.x - pos.x).hypot(sp.y - pos.y) < HIT_RADIUS {
                    return mouse::Interaction::Grab;
                }
            }
        }
        mouse::Interaction::default()
    }
}

#[derive(Default)]
pub struct DragState {
    dragging: Option<usize>,
}

fn world_to_screen(p: (f32, f32), size: Size, view: (f32, f32)) -> Point {
    let plot_w = size.width - MARGIN * 2.0;
    let plot_h = size.height - MARGIN * 2.0;
    let x = MARGIN + temp_to_x(p.0, plot_w, view);
    let y = MARGIN + plot_h * (1.0 - (p.1 / 100.0).clamp(0.0, 1.0));
    Point::new(x, y)
}

fn screen_to_world(p: Point, size: Size, view: (f32, f32)) -> (f32, f32) {
    let plot_w = size.width - MARGIN * 2.0;
    let plot_h = size.height - MARGIN * 2.0;
    let tx = ((p.x - MARGIN) / plot_w).clamp(0.0, 1.0);
    let ty = ((p.y - MARGIN) / plot_h).clamp(0.0, 1.0);
    let temp = view.0 + (view.1 - view.0) * tx;
    let speed = (1.0 - ty) * 100.0;
    (temp, speed)
}

fn temp_to_x(temp: f32, plot_w: f32, view: (f32, f32)) -> f32 {
    let span = (view.1 - view.0).max(0.001);
    ((temp - view.0) / span) * plot_w
}

fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

fn draw_tooltip(frame: &mut Frame, anchor: Point, value: (f32, f32), canvas_size: Size) {
    let label = format!("{:.0}°C   {:.0}%", value.0, value.1);
    // Approximate text width (size 12 → ~7px per char).
    let w = (label.chars().count() as f32) * 7.0 + 16.0;
    let h = 22.0;

    // Prefer placing the tooltip above-right of the point; flip when near edges.
    let mut x = anchor.x + 12.0;
    let mut y = anchor.y - h - 10.0;
    if x + w > canvas_size.width - 4.0 {
        x = anchor.x - w - 12.0;
    }
    if y < 4.0 {
        y = anchor.y + 12.0;
    }

    let bg = Color { r: 0.11, g: 0.11, b: 0.17, a: 0.95 };
    frame.fill_rectangle(Point::new(x, y), Size::new(w, h), bg);
    // Outline
    let outline = Path::rectangle(Point::new(x, y), Size::new(w, h));
    frame.stroke(
        &outline,
        Stroke {
            style: canvas::Style::Solid(theme::MAUVE),
            width: 1.0,
            ..Stroke::default()
        },
    );
    frame.fill_text(Text {
        content: label,
        position: Point::new(x + 8.0, y + 5.0),
        color: theme::TEXT,
        size: 12.0.into(),
        ..Text::default()
    });
}

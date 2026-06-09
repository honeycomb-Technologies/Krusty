use gpui::{
    canvas, div, fill, point, px, Bounds, Context, Hsla, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, PathBuilder, PathStyle,
    Pixels, Render, StatefulInteractiveElement as _, StrokeOptions, Styled as _, Window,
};

use crate::design::theme;

#[derive(Clone, Copy, Debug, PartialEq)]
struct StrokePoint {
    x: f32,
    y: f32,
}

impl StrokePoint {
    fn from_mouse_down(event: &MouseDownEvent) -> Self {
        Self {
            x: f32::from(event.position.x),
            y: f32::from(event.position.y),
        }
    }

    fn from_mouse_move(event: &MouseMoveEvent) -> Self {
        Self {
            x: f32::from(event.position.x),
            y: f32::from(event.position.y),
        }
    }
}

#[derive(Default)]
pub struct ScratchCanvasPanel {
    strokes: Vec<Vec<StrokePoint>>,
    draft: Option<Vec<StrokePoint>>,
}

impl ScratchCanvasPanel {
    pub fn new() -> Self {
        Self::default()
    }

    fn start_stroke(&mut self, point: StrokePoint, cx: &mut Context<Self>) {
        self.draft = Some(vec![point]);
        cx.notify();
    }

    fn update_stroke(&mut self, point: StrokePoint, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        let should_add = draft
            .last()
            .map(|last| ((last.x - point.x).powi(2) + (last.y - point.y).powi(2)).sqrt() > 2.0)
            .unwrap_or(true);
        if should_add {
            draft.push(point);
            cx.notify();
        }
    }

    fn finish_stroke(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        if !draft.is_empty() {
            self.strokes.push(draft);
        }
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.strokes.clear();
        self.draft = None;
        cx.notify();
    }
}

impl Render for ScratchCanvasPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let strokes = self.strokes.clone();
        let draft = self.draft.clone();
        let stroke_color = theme::text();
        let draft_color = theme::accent();
        let grid_minor = theme::hairline().opacity(0.38);

        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(theme::app_bg())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|panel, event: &MouseDownEvent, _window, cx| {
                    panel.start_stroke(StrokePoint::from_mouse_down(event), cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|panel, event: &MouseMoveEvent, _window, cx| {
                if event.dragging() {
                    panel.update_stroke(StrokePoint::from_mouse_move(event), cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|panel, _, _window, cx| {
                    panel.finish_stroke(cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|panel, _, _window, cx| {
                    panel.finish_stroke(cx);
                }),
            )
            .child(draw_surface(
                strokes,
                draft,
                stroke_color,
                draft_color,
                grid_minor,
            ))
            .child(
                div()
                    .absolute()
                    .top_2()
                    .left_2()
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::surface())
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(theme::text_muted())
                    .child("Draw with left mouse")
                    .child(
                        div()
                            .id("scratch-clear")
                            .mt_1()
                            .px_2()
                            .py_1()
                            .border_1()
                            .border_color(theme::hairline())
                            .bg(theme::app_bg())
                            .hover(|style| style.bg(theme::surface_hover()))
                            .cursor_pointer()
                            .child("Clear")
                            .on_click(cx.listener(|panel, _, _window, cx| {
                                panel.clear(cx);
                                cx.stop_propagation();
                            })),
                    ),
            )
    }
}

fn draw_surface(
    strokes: Vec<Vec<StrokePoint>>,
    draft: Option<Vec<StrokePoint>>,
    stroke_color: Hsla,
    draft_color: Hsla,
    grid_color: Hsla,
) -> impl IntoElement {
    canvas(
        move |_, _, _| (),
        move |bounds, _, window, _| {
            paint_grid(bounds, grid_color, window);
            for stroke in &strokes {
                paint_stroke(stroke, stroke_color, 2.0, window);
            }
            if let Some(draft) = draft.as_ref() {
                paint_stroke(draft, draft_color, 2.0, window);
            }
        },
    )
    .absolute()
    .top_0()
    .right_0()
    .bottom_0()
    .left_0()
}

fn paint_grid(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let spacing = 28.0;
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let right = left + f32::from(bounds.size.width);
    let bottom = top + f32::from(bounds.size.height);

    let mut x = (left / spacing).floor() * spacing;
    while x <= right {
        paint_line(
            StrokePoint { x, y: top },
            StrokePoint { x, y: bottom },
            color,
            1.0,
            window,
        );
        x += spacing;
    }

    let mut y = (top / spacing).floor() * spacing;
    while y <= bottom {
        paint_line(
            StrokePoint { x: left, y },
            StrokePoint { x: right, y },
            color,
            1.0,
            window,
        );
        y += spacing;
    }
}

fn paint_stroke(points: &[StrokePoint], color: Hsla, width: f32, window: &mut Window) {
    match points {
        [] => {}
        [stroke_point] => {
            let diameter = px(width + 2.0);
            window.paint_quad(
                fill(
                    Bounds::centered_at(
                        point(px(stroke_point.x), px(stroke_point.y)),
                        gpui::size(diameter, diameter),
                    ),
                    color,
                )
                .corner_radii(diameter / 2.0),
            );
        }
        _ => {
            let mut builder = PathBuilder::default().with_style(PathStyle::Stroke(
                StrokeOptions::DEFAULT
                    .with_line_width(width)
                    .with_tolerance(0.03),
            ));
            builder.move_to(point(px(points[0].x), px(points[0].y)));
            for stroke_point in &points[1..] {
                builder.line_to(point(px(stroke_point.x), px(stroke_point.y)));
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
    }
}

fn paint_line(start: StrokePoint, end: StrokePoint, color: Hsla, width: f32, window: &mut Window) {
    paint_stroke(&[start, end], color, width, window);
}

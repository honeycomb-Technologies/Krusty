use std::f32::consts::TAU;
use std::time::Duration;

use gpui::{
    div, hsla, img, px, Animation, AnimationExt as _, AnyElement, Hsla, ImageSource, IntoElement,
    ParentElement as _, Resource, SharedString, Styled as _,
};
use gpui_component::animation::cubic_bezier;
use gpui_component::Icon;

use crate::branding;
use crate::design::theme;

const WORDMARK_LINES: [&str; 5] = [
    "▄ •▄ ▄▄▄  ▄• ▄▌.▄▄ · ▄▄▄▄▄ ▄· ▄▌",
    "█▌▄▌▪▀▄ █·█▪██▌▐█ ▀. •██  ▐█▪██▌",
    "▐▀▀▄·▐▀▀▄ █▌▐█▌▄▀▀▀█▄ ▐█.▪▐█▌▐█▪",
    "▐█.█▌▐█•█▌▐█▄█▌▐█▄▪▐█ ▐█▌· ▐█▀·.",
    "·▀  ▀.▀  ▀ ▀▀▀  ▀▀▀▀  ▀▀▀   ▀ • ",
];

const WORDMARK_COLUMNS: usize = 32;
const WORDMARK_ROWS: usize = 5;
const CELL_WIDTH: f32 = 10.0;
const CELL_HEIGHT: f32 = 18.0;
const WORDMARK_WIDTH: f32 = CELL_WIDTH * WORDMARK_COLUMNS as f32;
const WORDMARK_HEIGHT: f32 = CELL_HEIGHT * WORDMARK_ROWS as f32;
const WORDMARK_COLOR_MS: u64 = 4_000;
const WORDMARK_FLOAT_MS: u64 = 3_200;
const WORDMARK_ENTER_MS: u64 = 720;
const THEMED_K_ICON: &str = "icons/krusty-k-theme.svg";

const FULL: [Fragment; 1] = [Fragment::new(0.0, 0.0, 1.0, 1.0)];
const LOWER: [Fragment; 1] = [Fragment::new(0.0, 0.5, 1.0, 0.5)];
const UPPER: [Fragment; 1] = [Fragment::new(0.0, 0.0, 1.0, 0.5)];
const LEFT: [Fragment; 1] = [Fragment::new(0.0, 0.0, 0.5, 1.0)];
const RIGHT: [Fragment; 1] = [Fragment::new(0.5, 0.0, 0.5, 1.0)];
const SMALL_SQUARE: [Fragment; 1] = [Fragment::new(0.31, 0.38, 0.38, 0.38)];
const BULLET: [Fragment; 1] = [Fragment::new(0.32, 0.30, 0.36, 0.36)];
const MIDDLE_DOT: [Fragment; 1] = [Fragment::new(0.40, 0.43, 0.20, 0.20)];
const PERIOD: [Fragment; 1] = [Fragment::new(0.42, 0.74, 0.16, 0.16)];
const EMPTY: [Fragment; 0] = [];

#[derive(Clone, Copy)]
struct Fragment {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Fragment {
    const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

pub fn mark(size: f32) -> AnyElement {
    img(ImageSource::Resource(Resource::Embedded(
        SharedString::from(branding::KRUSTY_K_ICON),
    )))
    .size(px(size))
    .into_any_element()
}

pub fn themed_mark(size: f32) -> AnyElement {
    Icon::empty()
        .path(THEMED_K_ICON)
        .size(px(size))
        .into_any_element()
}

pub fn animated_wordmark() -> AnyElement {
    div()
        .relative()
        .w(px(WORDMARK_WIDTH))
        .h(px(WORDMARK_HEIGHT))
        .children(wordmark_fragments())
        .with_animations(
            "krusty-wordmark-motion",
            vec![
                Animation::new(Duration::from_millis(WORDMARK_ENTER_MS))
                    .with_easing(cubic_bezier(0.32, 0.72, 0.0, 1.0)),
                Animation::new(Duration::from_millis(WORDMARK_FLOAT_MS)).repeat(),
            ],
            |this, animation_index, delta| {
                if animation_index == 0 {
                    this.opacity(delta).top(px((1.0 - delta) * 18.0))
                } else {
                    let y = (delta * TAU).sin() * 2.0;
                    this.opacity(1.0).top(px(y))
                }
            },
        )
        .into_any_element()
}

fn wordmark_fragments() -> Vec<AnyElement> {
    WORDMARK_LINES
        .iter()
        .enumerate()
        .flat_map(|(row, line)| {
            line.chars().enumerate().flat_map(move |(column, ch)| {
                fragments_for_char(ch)
                    .iter()
                    .enumerate()
                    .map(move |(fragment_index, fragment)| {
                        render_fragment(row, column, fragment_index, *fragment)
                    })
            })
        })
        .collect()
}

fn render_fragment(
    row: usize,
    column: usize,
    fragment_index: usize,
    fragment: Fragment,
) -> AnyElement {
    let left = (column as f32 + fragment.x) * CELL_WIDTH;
    let top = (row as f32 + fragment.y) * CELL_HEIGHT;
    let width = fragment.width * CELL_WIDTH;
    let height = fragment.height * CELL_HEIGHT;
    let phase_seed = ((column as f32 / WORDMARK_COLUMNS as f32) * 0.72 + row as f32 * 0.09) % 1.0;
    let animation_id = SharedString::from(format!(
        "krusty-wordmark-fragment-{row}-{column}-{fragment_index}"
    ));

    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width))
        .h(px(height))
        .bg(gradient_color(phase_seed))
        .with_animation(
            animation_id,
            Animation::new(Duration::from_millis(WORDMARK_COLOR_MS)).repeat(),
            move |this, delta| this.bg(gradient_color((phase_seed + delta) % 1.0)),
        )
        .into_any_element()
}

fn fragments_for_char(ch: char) -> &'static [Fragment] {
    match ch {
        '█' => &FULL,
        '▄' => &LOWER,
        '▀' => &UPPER,
        '▌' => &LEFT,
        '▐' => &RIGHT,
        '▪' => &SMALL_SQUARE,
        '•' => &BULLET,
        '·' => &MIDDLE_DOT,
        '.' => &PERIOD,
        _ => &EMPTY,
    }
}

fn gradient_color(phase: f32) -> Hsla {
    let phase = phase.clamp(0.0, 1.0);
    let colors = theme::logo_gradient_stops();
    let stops = [
        (0.00, colors[0]),
        (0.18, colors[1]),
        (0.34, colors[2]),
        (0.50, colors[3]),
        (0.66, colors[4]),
        (0.82, colors[5]),
        (1.00, colors[6]),
    ];

    for pair in stops.windows(2) {
        let (start_at, start) = pair[0];
        let (end_at, end) = pair[1];
        if phase <= end_at {
            let local = ((phase - start_at) / (end_at - start_at)).clamp(0.0, 1.0);
            return lerp_color(start, end, local);
        }
    }

    stops[stops.len() - 1].1
}

fn lerp_color(start: Hsla, end: Hsla, t: f32) -> Hsla {
    hsla(
        lerp(start.h, end.h, t),
        lerp(start.s, end.s, t),
        lerp(start.l, end.l, t),
        lerp(start.a, end.a, t),
    )
}

fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

//! Stable text serialization for layout golden tests.

use std::fmt::Write;

use crate::tui_v2::layout::snapshot::LayoutSnapshot;

pub fn serialize_layout(snapshot: &LayoutSnapshot) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "viewport {} {} {} {} class={:?} route={:?}",
        snapshot.viewport.x,
        snapshot.viewport.y,
        snapshot.viewport.width,
        snapshot.viewport.height,
        snapshot.class,
        snapshot.route
    )
    .expect("string write");
    for region in &snapshot.regions {
        writeln!(
            output,
            "region {:?} {} {} {} {} clip={} {} {} {}",
            region.id,
            region.rect.x,
            region.rect.y,
            region.rect.width,
            region.rect.height,
            region.clip.x,
            region.clip.y,
            region.clip.width,
            region.clip.height
        )
        .expect("string write");
    }
    for interaction in &snapshot.interactions {
        writeln!(
            output,
            "interaction {} {} {} {} {} {:?}",
            interaction.id.as_str(),
            interaction.bounds.x,
            interaction.bounds.y,
            interaction.bounds.width,
            interaction.bounds.height,
            interaction.intent
        )
        .expect("string write");
    }
    if let Some(focus) = snapshot.focus_rect {
        writeln!(
            output,
            "focus {} {} {} {}",
            focus.x, focus.y, focus.width, focus.height
        )
        .expect("string write");
    } else {
        writeln!(output, "focus none").expect("string write");
    }
    output.trim_end().to_owned()
}

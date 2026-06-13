use gpui::{div, AnyElement, Context, IntoElement, ParentElement as _, Styled as _};

use crate::components::button::{krusty_button, KrustyButtonKind};
use crate::design::theme;
use crate::panels::chat::{ChatPanel, PendingToolApproval};

pub fn tool_approval_bar(
    approval: &PendingToolApproval,
    cx: &mut Context<ChatPanel>,
) -> AnyElement {
    let approve_id = approval.tool_call_id.clone();
    let deny_id = approval.tool_call_id.clone();

    div()
        .border_1()
        .border_color(theme::complement())
        .bg(theme::surface_selected())
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().text_color(theme::text()).child(format!(
            "Approve tool call: {} ({})",
            approval.tool_name, approval.tool_call_id
        )))
        .child(
            div()
                .flex()
                .gap_2()
                .child(
                    krusty_button("approve-tool", "Approve", KrustyButtonKind::Primary, cx)
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.respond_tool_approval(&approve_id, true, cx);
                        })),
                )
                .child(
                    krusty_button("deny-tool", "Deny", KrustyButtonKind::Danger, cx).on_click(
                        cx.listener(move |panel, _, _, cx| {
                            panel.respond_tool_approval(&deny_id, false, cx);
                        }),
                    ),
                ),
        )
        .into_any_element()
}

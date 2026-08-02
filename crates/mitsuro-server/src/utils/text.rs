pub fn trimmed_nonempty(value: Option<&str>) -> Option<&str> {
    value.and_then(|text| {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

#[cfg(test)]
mod tests {
    use super::trimmed_nonempty;

    #[test]
    fn trimmed_nonempty_trims_and_drops_blank_strings() {
        assert_eq!(
            trimmed_nonempty(Some("  openai/gpt-5  ")),
            Some("openai/gpt-5")
        );
        assert_eq!(trimmed_nonempty(Some("   ")), None);
        assert_eq!(trimmed_nonempty(None), None);
    }
}

pub(crate) const MAX_TEXT_RESULT_BYTES: usize = 256 * 1024;

pub(crate) fn bounded_text(mut value: String) -> String {
    if value.len() <= MAX_TEXT_RESULT_BYTES {
        return value;
    }
    let original = value.len();
    let footer =
        format!("\n[output truncated: {original} bytes; limit {MAX_TEXT_RESULT_BYTES} bytes]\n");
    let mut end = MAX_TEXT_RESULT_BYTES.saturating_sub(footer.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.push_str(&footer);
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_utf8_safe_and_bounded() {
        let output = bounded_text("界".repeat(MAX_TEXT_RESULT_BYTES));
        assert!(output.len() <= MAX_TEXT_RESULT_BYTES);
        assert!(output.contains("output truncated"));
    }
}

//! Ctrl-A / Ctrl-X: add to the number at or after the cursor.
use super::super::super::doc::Buffer;
use super::super::mode::Effect;
use super::super::state::VimEngine;

/// A number on a line: char columns `start..end`, value and radix.
struct Number {
    start: usize,
    end: usize,
    value: i128,
    hex: bool,
}

/// The number under or after `col` (decimal with sign, or `0x` hex).
fn number_at(chars: &[char], col: usize) -> Option<Number> {
    let is_digit = |i: usize| chars.get(i).is_some_and(char::is_ascii_digit);
    let mut start = (col..chars.len()).find(|i| is_digit(*i))?;
    // Inside a number: back to its first digit.
    if start == col {
        while start > 0 && is_digit(start.saturating_sub(1)) {
            start = start.saturating_sub(1);
        }
    }
    let lower = |i: usize| chars.get(i).map(char::to_ascii_lowercase);
    // Inside the digits of `0x…`: back to the prefix.
    let mut hex_start = col;
    while hex_start > 0
        && chars
            .get(hex_start.saturating_sub(1))
            .is_some_and(char::is_ascii_hexdigit)
    {
        hex_start = hex_start.saturating_sub(1);
    }
    if chars.get(col).is_some_and(char::is_ascii_hexdigit)
        && hex_start >= 2
        && lower(hex_start.saturating_sub(1)) == Some('x')
        && chars.get(hex_start.saturating_sub(2)) == Some(&'0')
    {
        start = hex_start.saturating_sub(2);
    }
    if chars.get(start) == Some(&'0')
        && lower(start.saturating_add(1)) == Some('x')
        && chars
            .get(start.saturating_add(2))
            .is_some_and(char::is_ascii_hexdigit)
    {
        let digits_from = start.saturating_add(2);
        let mut end = digits_from;
        while chars.get(end).is_some_and(char::is_ascii_hexdigit) {
            end = end.saturating_add(1);
        }
        let digits: String = chars.get(digits_from..end)?.iter().collect();
        let value = i128::from_str_radix(&digits, 16).ok()?;
        return Some(Number {
            start: digits_from,
            end,
            value,
            hex: true,
        });
    }
    let mut end = start;
    while is_digit(end) {
        end = end.saturating_add(1);
    }
    let negative = start > 0 && chars.get(start.saturating_sub(1)) == Some(&'-');
    let digits: String = chars.get(start..end)?.iter().collect();
    let mut value: i128 = digits.parse().ok()?;
    if negative {
        start = start.saturating_sub(1);
        value = value.saturating_neg();
    }
    Some(Number {
        start,
        end,
        value,
        hex: false,
    })
}

impl VimEngine {
    pub(crate) fn increment<B: Buffer>(
        &mut self,
        doc: &mut B,
        count: usize,
        add: bool,
    ) -> Vec<Effect> {
        let line = self.cursor.line;
        let chars: Vec<char> = doc.line_text(line).chars().collect();
        let Some(number) = number_at(&chars, self.cursor.col) else {
            return self.finish_simple(false);
        };
        let delta = i128::try_from(count).unwrap_or(1);
        let value = if add {
            number.value.saturating_add(delta)
        } else {
            number.value.saturating_sub(delta)
        };
        let text = if number.hex {
            let width = number.end.saturating_sub(number.start);
            let upper = chars
                .get(number.start..number.end)
                .is_some_and(|d| d.iter().any(char::is_ascii_uppercase));
            let masked = value.rem_euclid(1i128 << 64);
            if upper {
                format!("{masked:0width$X}")
            } else {
                format!("{masked:0width$x}")
            }
        } else {
            value.to_string()
        };
        self.x.keys_change = true;
        let from = doc.char_at_line_col(line, number.start);
        let to = doc.char_at_line_col(line, number.end);
        self.replace_text(doc, (from, to), &text);
        self.cursor.col = number
            .start
            .saturating_add(text.chars().count())
            .saturating_sub(1);
        self.preferred_col = self.cursor.col;
        self.clear_pending();
        self.clamp(doc);
        self.rec_commit();
        Vec::new()
    }
}

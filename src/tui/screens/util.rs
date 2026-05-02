/// Right-pad `s` with spaces (so `s` sits on the left).
pub(super) fn pad_right(s: String, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s
    } else {
        let mut out = s;
        out.extend(std::iter::repeat_n(' ', width - len));
        out
    }
}

/// Left-pad `s` with spaces (so `s` sits on the right — for right-aligned numerics).
pub(super) fn pad_left(s: String, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s
    } else {
        let pad: String = std::iter::repeat_n(' ', width - len).collect();
        pad + &s
    }
}

pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}

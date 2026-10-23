//! Wording shared by the screens.

/// "осталось 12 мин"; empty when the length is unknown.
pub fn remaining(position: f64, duration: f64) -> String {
    if duration <= 0. {
        return String::new();
    }
    let minutes = ((duration - position) / 60.).floor();
    if minutes < 1. {
        "осталось меньше минуты".into()
    } else {
        format!("осталось {minutes} мин")
    }
}

/// The Russian word for `n`: "серия", "серии" or "серий".
pub fn plural<'a>(n: i64, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
    let (n10, n100) = (n.abs() % 10, n.abs() % 100);
    if n10 == 1 && n100 != 11 {
        one
    } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
        few
    } else {
        many
    }
}

/// "Сериал", "Фильм"; an unknown kind in capitals.
pub fn kind_label(kind: Option<&str>) -> String {
    match kind {
        Some("tv") => "Сериал".into(),
        Some("movie") => "Фильм".into(),
        Some("ova") => "OVA".into(),
        Some("ona") => "ONA".into(),
        Some("special" | "tv_special") => "Спешл".into(),
        Some("music") => "Клип".into(),
        Some(other) => other.to_uppercase(),
        None => String::new(),
    }
}

/// The name of a library list: "Смотрю", "В планах".
pub fn library_status(status: &str) -> &'static str {
    match status {
        "watching" => "Смотрю",
        "planned" => "В планах",
        "completed" => "Просмотрено",
        "on_hold" => "Отложено",
        "dropped" => "Брошено",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{plural, remaining};

    #[test]
    fn plural_follows_russian_endings() {
        let p = |n| plural(n, "серия", "серии", "серий");
        assert_eq!(
            [p(1), p(2), p(5), p(11), p(12), p(21), p(22), p(25)],
            [
                "серия",
                "серии",
                "серий",
                "серий",
                "серий",
                "серия",
                "серии",
                "серий"
            ]
        );
    }

    #[test]
    fn remaining_rounds_down_and_says_when_under_a_minute() {
        assert_eq!(remaining(0., 1440.), "осталось 24 мин");
        assert_eq!(remaining(100., 1440.), "осталось 22 мин");
        assert_eq!(remaining(1400., 1440.), "осталось меньше минуты");
        assert_eq!(remaining(10., 0.), "");
    }
}

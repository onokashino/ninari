//! Checks a source's uncertain match against the title's own names.

/// Below this a fuzzy match is another title: a site's search happily returns anything that
/// shares a word with the query.
const MIN_SCORE: f64 = 0.85;

/// Lower case, `ё` as `е`, punctuation as single spaces.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(if c == 'ё' { 'е' } else { c });
        } else if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.truncate(out.trim_end().len());
    out
}

/// Season and part numbers, in order: "3. Часть 2" and "3" are different titles however
/// alike the rest reads.
fn numbers(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|n| !n.is_empty())
        .collect()
}

/// Sørensen-Dice coefficient over character pairs.
fn dice(a: &str, b: &str) -> f64 {
    let pairs = |s: &str| {
        let chars: Vec<char> = s.chars().collect();
        chars.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>()
    };
    let (a, mut b) = (pairs(a), pairs(b));
    let total = a.len() + b.len();
    if total == 0 {
        return 0.0;
    }
    let mut common = 0;
    for p in &a {
        if let Some(i) = b.iter().position(|q| q == p) {
            b.swap_remove(i);
            common += 1;
        }
    }
    2.0 * common as f64 / total as f64
}

/// How well a candidate title matches the closest of the names, 0 when it is another title.
pub fn score(title: &str, names: &[String]) -> f64 {
    let title = normalize(title);
    let wanted = numbers(&title);
    names
        .iter()
        .map(|name| {
            let name = normalize(name);
            if numbers(&name) != wanted {
                0.0
            } else if name == title {
                1.0
            } else {
                dice(&title, &name)
            }
        })
        .filter(|s| *s >= MIN_SCORE)
        .fold(0.0, f64::max)
}

pub fn plausible(title: &str, names: &[String]) -> bool {
    score(title, names) > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mushoku_3_part_2() -> Vec<String> {
        vec![
            "Реинкарнация безработного: История о приключениях в другом мире 3. Часть 2".into(),
            "Mushoku Tensei III: Isekai Ittara Honki Dasu Part 2".into(),
            "無職転生 III ～異世界行ったら本気だす～ 第2クール".into(),
        ]
    }

    #[test]
    fn rejects_another_title_that_shares_a_word() {
        assert!(!plausible(
            "Реинкарнация аристократа: Благословенный с рождения величайшей силой",
            &mushoku_3_part_2()
        ));
        assert!(!plausible(
            "Расхититель гробниц",
            &["Гробница богов 3".into()]
        ));
    }

    #[test]
    fn rejects_another_season_or_part_of_the_same_series() {
        let season_3 =
            vec!["Реинкарнация безработного: История о приключениях в другом мире 3".to_string()];
        assert!(!plausible(
            "Реинкарнация безработного: История о приключениях в другом мире 3. Часть 2",
            &season_3
        ));
        assert!(!plausible(
            "Реинкарнация безработного: История о приключениях в другом мире 3",
            &mushoku_3_part_2()
        ));
    }

    #[test]
    fn accepts_the_same_title_written_differently() {
        assert!(plausible(
            "Реинкарнация безработного - история о приключениях в другом мире 3. Часть 2",
            &mushoku_3_part_2()
        ));
        assert!(plausible(
            "Mushoku Tensei III: Isekai Ittara Honki Dasu (Part 2)",
            &mushoku_3_part_2()
        ));
        assert!(plausible("Её величество", &["Ее величество!".into()]));
    }

    #[test]
    fn prefers_the_closer_title() {
        let names = vec!["Провожающая в последний путь Фрирен".to_string()];
        assert!(
            score("Провожающая в последний путь Фрирен", &names)
                > score("Провожающая в путь Фрирен", &names)
        );
        assert_eq!(score("Кто-то совсем другой", &names), 0.0);
    }
}

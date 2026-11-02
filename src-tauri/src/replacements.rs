use crate::settings::Replacement;

/// Corrects words the transcription gets wrong, using the user's replacement list.
pub fn apply(text: &str, replacements: &[Replacement]) -> String {
    replacements
        .iter()
        .filter(|replacement| !replacement.from.is_empty())
        .fold(text.to_string(), |result, replacement| {
            replace_whole_word(&result, &replacement.from, &replacement.to)
        })
}

/// Swiss spelling differs from German only in writing ss instead of ß.
pub fn to_swiss_spelling(text: &str) -> String {
    text.replace('ß', "ss")
}

fn replace_whole_word(text: &str, from: &str, to: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut copied = 0;

    for (start, matched) in text.match_indices(from) {
        let end = start + matched.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();

        if before.is_some_and(char::is_alphanumeric) || after.is_some_and(char::is_alphanumeric) {
            continue;
        }

        result.push_str(&text[copied..start]);
        result.push_str(to);
        copied = end;
    }

    result.push_str(&text[copied..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replacement(from: &str, to: &str) -> Replacement {
        Replacement {
            from: from.to_string(),
            to: to.to_string(),
        }
    }

    #[test]
    fn test_replaces_whole_word() {
        let list = [replacement("Taurophon", "Taurophone")];
        assert_eq!(apply("Ich nutze Taurophon.", &list), "Ich nutze Taurophone.");
    }

    #[test]
    fn test_does_not_replace_inside_a_word() {
        let list = [replacement("Taurophon", "Taurophone")];
        assert_eq!(apply("Taurophone und Taurophons", &list), "Taurophone und Taurophons");
    }

    #[test]
    fn test_replaces_every_occurrence_and_multiple_words() {
        let list = [replacement("Kuratli und Jäger", "Kuratle & Jäcker")];
        assert_eq!(
            apply("Kuratli und Jäger, Kuratli und Jäger", &list),
            "Kuratle & Jäcker, Kuratle & Jäcker"
        );
    }

    #[test]
    fn test_is_case_sensitive() {
        let list = [replacement("Taurophon", "Taurophone")];
        assert_eq!(apply("taurophon", &list), "taurophon");
    }

    #[test]
    fn test_swiss_spelling_replaces_sharp_s() {
        assert_eq!(to_swiss_spelling("großformatig und Straße"), "grossformatig und Strasse");
    }

    #[test]
    fn test_ignores_empty_source() {
        let list = [replacement("", "x")];
        assert_eq!(apply("Text", &list), "Text");
    }
}

use std::borrow::Cow;

#[allow(dead_code)]
pub(crate) fn parse_token(input: &str) -> (Option<&str>, &str) {
    let mut end_of_token = 0;
    for (i, c) in input.char_indices() {
        if tchar(c) {
            end_of_token = i;
        } else {
            break;
        }
    }

    if end_of_token == 0 {
        (None, input)
    } else {
        (Some(&input[..end_of_token + 1]), &input[end_of_token + 1..])
    }
}

fn tchar(c: char) -> bool {
    matches!(
        c, 'a'..='z'
            | 'A'..='Z'
            | '0'..='9'
            | '!'
            | '#'
            | '$'
            | '%'
            | '&'
            | '\''
            | '*'
            | '+'
            | '-'
            | '.'
            | '^'
            | '_'
            | '`'
            | '|'
            | '~'
    )
}

fn vchar(c: char) -> bool {
    matches!(c as u8, b'\t' | 32..=126 | 128..=255)
}

#[allow(dead_code)]
pub(crate) fn parse_quoted_string(input: &str) -> (Option<Cow<'_, str>>, &str) {
    if !input.starts_with('"') {
        return (None, input);
    }

    let mut end_of_string = None;
    let mut backslashes: Vec<usize> = vec![];

    for (i, c) in input.char_indices().skip(1) {
        if i > 1 && backslashes.last() == Some(&(i - 2)) {
            if !vchar(c) {
                return (None, input);
            }
        } else {
            match c as u8 {
                b'\\' => {
                    backslashes.push(i - 1);
                }

                b'"' => {
                    end_of_string = Some(i + 1);
                    break;
                }

                b'\t' | b' ' | 15 | 35..=91 | 93..=126 | 128..=255 => {}

                _ => return (None, input),
            }
        }
    }

    if let Some(end_of_string) = end_of_string {
        let value = &input[1..end_of_string - 1];

        let value = if backslashes.is_empty() {
            value.into()
        } else {
            backslashes.reverse();

            value
                .char_indices()
                .filter_map(|(i, c)| {
                    if Some(&i) == backslashes.last() {
                        backslashes.pop();
                        None
                    } else {
                        Some(c)
                    }
                })
                .collect::<String>()
                .into()
        };

        (Some(value), &input[end_of_string..])
    } else {
        (None, input)
    }
}

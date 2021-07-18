use std::fmt;

use crate::common::http_types::mime::{Mime, ParamKind, ParamName, ParamValue};

pub(crate) fn parse(input: &str) -> crate::common::http_types::Result<Mime> {
    let input = input.trim_matches(is_http_whitespace_char);

    let (basetype, input) = collect_code_point_sequence_char(input, '/');

    crate::ensure!(!basetype.is_empty(), "MIME type should not be empty");
    crate::ensure!(
        basetype.chars().all(is_http_token_code_point),
        "MIME type should ony contain valid HTTP token code points"
    );

    crate::ensure!(!input.is_empty(), "MIME must contain a sub type");

    let input = &input[1..];

    let (subtype, input) = collect_code_point_sequence_char(input, ';');

    let subtype = subtype.trim_end_matches(is_http_whitespace_char);

    crate::ensure!(!subtype.is_empty(), "MIME sub type should not be empty");
    crate::ensure!(
        subtype.chars().all(is_http_token_code_point),
        "MIME sub type should ony contain valid HTTP token code points"
    );

    let basetype = basetype.to_ascii_lowercase();
    let subtype = subtype.to_ascii_lowercase();
    let mut params = None;

    let mut input = input;
    while !input.is_empty() {
        input = &input[1..];

        input = input.trim_start_matches(is_http_whitespace_char);

        let (parameter_name, new_input) =
            collect_code_point_sequence_slice(input, &[';', '='] as &[char]);
        input = new_input;

        let parameter_name = parameter_name.to_ascii_lowercase();

        if input.is_empty() {
            break;
        } else {
            if input.starts_with(';') {
                continue;
            } else {
                input = &input[1..];
            }
        }

        let parameter_value = if input.starts_with('"') {
            let (parameter_value, new_input) = collect_http_quoted_string(input);
            let (_, new_input) = collect_code_point_sequence_char(new_input, ';');
            input = new_input;
            parameter_value
        } else {
            let (parameter_value, new_input) = collect_code_point_sequence_char(input, ';');
            input = new_input;
            let parameter_value = parameter_value.trim_end_matches(is_http_whitespace_char);
            if parameter_value.is_empty() {
                continue;
            }
            parameter_value.to_owned()
        };

        if !parameter_name.is_empty()
            && parameter_name.chars().all(is_http_token_code_point)
            && parameter_value
                .chars()
                .all(is_http_quoted_string_token_code_point)
        {
            let params = params.get_or_insert_with(Vec::new);
            let name = ParamName(parameter_name.into());
            let value = ParamValue(parameter_value.into());
            if !params.iter().any(|(k, _)| k == &name) {
                params.push((name, value));
            }
        }
    }

    Ok(Mime {
        essence: format!("{}/{}", &basetype, &subtype),
        basetype,
        subtype,
        params: params.map(ParamKind::Vec),
        static_essence: None,
        static_basetype: None,
        static_subtype: None,
    })
}

fn is_http_token_code_point(c: char) -> bool {
    matches!(c,
        '!'
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
        | 'a'..='z'
        | 'A'..='Z'
        | '0'..='9')
}

fn is_http_quoted_string_token_code_point(c: char) -> bool {
    matches!(c, '\t' | ' '..='~' | '\u{80}'..='\u{FF}')
}

fn is_http_whitespace_char(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\t' | ' ')
}

fn collect_code_point_sequence_char(input: &str, delimiter: char) -> (&str, &str) {
    input.split_at(input.find(delimiter).unwrap_or_else(|| input.len()))
}

fn collect_code_point_sequence_slice<'a>(input: &'a str, delimiter: &[char]) -> (&'a str, &'a str) {
    input.split_at(input.find(delimiter).unwrap_or_else(|| input.len()))
}

fn collect_http_quoted_string(mut input: &str) -> (String, &str) {
    let mut value = String::new();
    input = &input[1..];
    loop {
        let (add_value, new_input) =
            collect_code_point_sequence_slice(input, &['"', '\\'] as &[char]);
        value.push_str(add_value);
        let mut chars = new_input.chars();
        if let Some(quote_or_backslash) = chars.next() {
            input = chars.as_str();
            if quote_or_backslash == '\\' {
                if let Some(c) = chars.next() {
                    value.push(c);
                    input = chars.as_str();
                } else {
                    value.push('\\');
                    break;
                }
            } else {
                break;
            }
        } else {
            break;
        }
    }
    (value, input)
}

pub(crate) fn format(mime_type: &Mime, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if let Some(essence) = mime_type.static_essence {
        write!(f, "{}", essence)?
    } else {
        write!(f, "{}", &mime_type.essence)?
    }
    if let Some(params) = &mime_type.params {
        match params {
            ParamKind::Utf8 => write!(f, ";charset=utf-8")?,
            ParamKind::Vec(params) => {
                for (name, value) in params {
                    if value.0.chars().all(is_http_token_code_point) && !value.0.is_empty() {
                        write!(f, ";{}={}", name, value)?;
                    } else {
                        write!(
                            f,
                            ";{}=\"{}\"",
                            name,
                            value
                                .0
                                .chars()
                                .flat_map(|c| match c {
                                    '"' | '\\' => EscapeMimeValue {
                                        state: EscapeMimeValueState::Backslash(c)
                                    },
                                    c => EscapeMimeValue {
                                        state: EscapeMimeValueState::Char(c)
                                    },
                                })
                                .collect::<String>()
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

struct EscapeMimeValue {
    state: EscapeMimeValueState,
}

#[derive(Clone, Debug)]
enum EscapeMimeValueState {
    Done,
    Char(char),
    Backslash(char),
}

impl Iterator for EscapeMimeValue {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self.state {
            EscapeMimeValueState::Done => None,
            EscapeMimeValueState::Char(c) => {
                self.state = EscapeMimeValueState::Done;
                Some(c)
            }
            EscapeMimeValueState::Backslash(c) => {
                self.state = EscapeMimeValueState::Char(c);
                Some('\\')
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.state {
            EscapeMimeValueState::Done => (0, Some(0)),
            EscapeMimeValueState::Char(_) => (1, Some(1)),
            EscapeMimeValueState::Backslash(_) => (2, Some(2)),
        }
    }
}

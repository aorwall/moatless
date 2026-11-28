//! Masks secret values in a command's text before the registry stores it.
//!
//! The stored text reaches the status API, the journal, the backend mirror and
//! the logs, while the child still runs the original. Values are found by the
//! name they are assigned to, since an agent often mints a secret mid-session
//! (`aws codeartifact get-authorization-token`) that no credential list holds.

/// What replaces a masked value.
const MASK: &str = "***";

/// Words that mark a variable or flag name as holding a secret.
const SECRET_WORDS: [&str; 9] = [
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "KEY",
    "APIKEY",
    "CREDENTIAL",
    "CREDENTIALS",
    "AUTH",
];

/// Suffixes that mark a run-together name, such as `GHTOKEN`, as a secret.
const SECRET_SUFFIXES: [&str; 3] = ["TOKEN", "SECRET", "PASSWORD"];

/// `command` with the value of every secret-named `NAME=value` assignment and
/// `--name=value` flag replaced by `***`.
pub(crate) fn redact_secrets(command: &str) -> String {
    let chars: Vec<char> = command.chars().collect();
    let mut out = String::with_capacity(command.len());
    let mut i = 0;
    while i < chars.len() {
        if starts_word(&chars, i)
            && let Some((name_end, value_end)) = secret_assignment(&chars, i)
        {
            out.extend(&chars[i..=name_end]);
            out.push_str(MASK);
            i = value_end;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn starts_word(chars: &[char], i: usize) -> bool {
    i == 0 || chars[i - 1].is_whitespace() || matches!(chars[i - 1], ';' | '&' | '|' | '(')
}

/// The index of the `=` and the end of the value, when the word at `start` is
/// an assignment or a `--flag=` whose name holds a secret.
fn secret_assignment(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let is_flag = chars.get(start) == Some(&'-') && chars.get(start + 1) == Some(&'-');
    let name_start = if is_flag { start + 2 } else { start };
    let first = *chars.get(name_start)?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let mut eq = name_start;
    while eq < chars.len()
        && (chars[eq].is_ascii_alphanumeric() || chars[eq] == '_' || (is_flag && chars[eq] == '-'))
    {
        eq += 1;
    }
    if chars.get(eq) != Some(&'=') {
        return None;
    }
    let name: String = chars[name_start..eq].iter().collect();
    if !names_a_secret(&name) {
        return None;
    }
    Some((eq, value_end(chars, eq + 1)))
}

fn names_a_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase().replace('-', "_");
    upper.split('_').any(|word| {
        SECRET_WORDS.contains(&word) || SECRET_SUFFIXES.iter().any(|suffix| word.ends_with(suffix))
    })
}

/// Where a shell word that begins at `start` ends: past a closing quote, or at
/// the first unquoted whitespace or command separator.
fn value_end(chars: &[char], start: usize) -> usize {
    let mut i = start;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => i += 1,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == '\\' => i += 1,
            None if c.is_whitespace() || matches!(c, ';' | '&' | '|' | ')') => return i,
            None => {}
        }
        i += 1;
    }
    chars.len().min(i)
}

#[cfg(test)]
mod tests {
    use super::redact_secrets;

    #[test]
    fn masks_an_env_assignment_named_as_a_token() {
        assert_eq!(
            redact_secrets("CODEARTIFACT_AUTH_TOKEN=eyJ2ZXIiOjEs.abc ./mvnw verify"),
            "CODEARTIFACT_AUTH_TOKEN=*** ./mvnw verify"
        );
    }

    #[test]
    fn masks_quoted_values_and_exports() {
        assert_eq!(
            redact_secrets(r#"export API_KEY="a b \" c"; DB_PASSWORD='x y' run"#),
            "export API_KEY=***; DB_PASSWORD=*** run"
        );
    }

    #[test]
    fn masks_a_secret_flag() {
        assert_eq!(
            redact_secrets("psql --db-password=hunter2 --host=db"),
            "psql --db-password=*** --host=db"
        );
    }

    #[test]
    fn masks_after_a_separator() {
        assert_eq!(
            redact_secrets("cd x && GITHUB_TOKEN=ghp_1 gh pr list|(GHTOKEN=y z)"),
            "cd x && GITHUB_TOKEN=*** gh pr list|(GHTOKEN=*** z)"
        );
    }

    #[test]
    fn keeps_values_whose_names_hold_no_secret() {
        let command = "SCREENING_TEST_POSTGRES_URL=jdbc:postgresql://127.0.0.1:5432/postgres \
                       MONKEY=1 ./mvnw -o verify > out.log 2>&1";
        assert_eq!(redact_secrets(command), command);
    }

    #[test]
    fn leaves_an_assignment_inside_a_word_alone() {
        assert_eq!(redact_secrets("echo a=TOKEN=b"), "echo a=TOKEN=b");
    }
}

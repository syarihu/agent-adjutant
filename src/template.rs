//! Command templates: `{placeholder}` substitution and POSIX shell quoting.
//!
//! Every escape hatch in this tool is a command template — how to open a tab, how to start a
//! worker, how to notify a human, how to open an editor. They all go through here so the
//! quoting rules are the same everywhere.
//!
//! The rule: **do not quote placeholders in a template.** A value is substituted already
//! shell-quoted, so `-n {title}` survives a title with spaces, quotes and newlines in it.
//! Writing `-n '{title}'` anyway is the obvious mistake, so it is accepted too — the
//! surrounding quotes are dropped rather than nested.

/// How a value is spliced in.
#[derive(Debug, Clone, Copy)]
pub enum Sub<'a> {
    /// Shell-quoted, so the value can contain anything at all.
    Quoted(&'a str),
    /// Inserted verbatim, for a value that already *is* a command line.
    Raw(&'a str),
}

/// POSIX single-quote quoting. `'` ends the literal, escapes itself outside it, and reopens —
/// which is the one form that needs no knowledge of what the shell will do next.
pub fn sh_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn sh_join<S: AsRef<str>>(parts: &[S]) -> String {
    parts
        .iter()
        .map(|p| sh_quote(p.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn contains_placeholder(template: &str, key: &str) -> bool {
    template.contains(&format!("{{{key}}}"))
}

/// One pass over the template: a value that has been substituted is never read again.
///
/// Replacing key by key across the whole string re-read what the previous key had written, so
/// a message containing the literal `{nwo}` got the repository spliced into the middle of its
/// own quoted argument — splitting one argument into two, and, when the repository name had a
/// `$(…)` in it, leaving that outside the quotes that were supposed to contain it.
pub fn render(template: &str, subs: &[(&str, Sub<'_>)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        let Some(end) = rest[at..].find('}').map(|n| at + n) else {
            break;
        };
        let Some((_, sub)) = subs.iter().find(|(key, _)| *key == &rest[at + 1..end]) else {
            // Not a placeholder this call answers for, so it is left alone rather than
            // blanked: `-n {title}` emptied would swallow the next argument.
            out.push_str(&rest[..=at]);
            rest = &rest[at + 1..];
            continue;
        };
        match *sub {
            Sub::Quoted(value) => {
                // A value arrives already quoted, so quotes the template wrapped around the
                // placeholder are dropped rather than nested. Both sides have to agree —
                // `"{title}'` is two literal quotes around a value, not a wrapper.
                let wrapped = matches!(
                    (
                        rest[..at].chars().next_back(),
                        rest[end + 1..].chars().next()
                    ),
                    (Some('\''), Some('\'')) | (Some('"'), Some('"'))
                );
                out.push_str(&rest[..at - usize::from(wrapped)]);
                out.push_str(&sh_quote(value));
                rest = &rest[end + 1 + usize::from(wrapped)..];
            }
            // A raw value is not quoted, so quotes the template put around it are the
            // template author's own and mean something — `sh -c '{command}'` needs them.
            // Stripping them turned that into `sh -c cd`, which silently ran the command in
            // the launcher's directory instead of opening a tab.
            Sub::Raw(value) => {
                out.push_str(&rest[..at]);
                out.push_str(value);
                rest = &rest[end + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_values_are_left_bare_so_templates_stay_readable() {
        assert_eq!(sh_quote("/src/widget"), "/src/widget");
        assert_eq!(sh_quote("WID-957"), "WID-957");
    }

    #[test]
    fn anything_the_shell_would_touch_gets_quoted() {
        assert_eq!(sh_quote("two words"), "'two words'");
        assert_eq!(sh_quote("$(rm -rf /)"), "'$(rm -rf /)'");
        assert_eq!(sh_quote("a`b`"), "'a`b`'");
        assert_eq!(sh_quote(""), "''");
        assert_eq!(sh_quote("画像が潰れる"), "'画像が潰れる'");
    }

    #[test]
    fn a_single_quote_in_the_value_closes_and_reopens() {
        assert_eq!(sh_quote("it's"), r#"'it'\''s'"#);
    }

    #[test]
    fn quoting_survives_a_round_trip_through_a_real_shell() {
        for value in [
            "it's",
            "two words",
            "$(echo pwned)",
            "back\\slash",
            "new\nline",
            "画像が潰れる",
            "a\"b",
        ] {
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("printf %s {}", sh_quote(value)))
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), value, "for {value:?}");
        }
    }

    #[test]
    fn a_template_that_wrapped_its_placeholder_in_quotes_does_not_nest_them() {
        assert_eq!(
            render("claude '{prompt}'", &[("prompt", Sub::Quoted("it's here"))]),
            r#"claude 'it'\''s here'"#
        );
        assert_eq!(
            render("claude \"{prompt}\"", &[("prompt", Sub::Quoted("go"))]),
            "claude go"
        );
    }

    #[test]
    fn quotes_a_template_put_around_a_raw_value_are_the_authors_own() {
        // `sh -c '…'` needs its quotes: without them the shell takes `cd` as $0 and runs
        // the rest in the caller's directory.
        assert_eq!(
            render(
                "sh -c '{command}'",
                &[("command", Sub::Raw("cd /wt && run"))]
            ),
            "sh -c 'cd /wt && run'"
        );
    }

    #[test]
    fn a_raw_substitution_is_a_command_line_and_stays_one() {
        assert_eq!(
            render(
                "tmux new-window -c {cwd} -n {title} {command}",
                &[
                    ("cwd", Sub::Quoted("/src/my widget")),
                    ("title", Sub::Quoted("WID-957")),
                    ("command", Sub::Raw("claude 'go now'")),
                ]
            ),
            "tmux new-window -c '/src/my widget' -n WID-957 claude 'go now'"
        );
    }

    #[test]
    fn an_unused_placeholder_is_left_alone_rather_than_blanked() {
        // Blanking it would turn `-n {title}` into `-n` and swallow the next argument.
        assert_eq!(render("x {a} {b}", &[("a", Sub::Quoted("1"))]), "x 1 {b}");
    }

    #[test]
    fn placeholders_are_detectable_so_callers_can_fall_back() {
        assert!(contains_placeholder("a {command} b", "command"));
        assert!(!contains_placeholder("a {commands} b", "command"));
    }

    /// A substituted value is data, not more template. The notification hook is where this
    /// bites: the message is written by whoever sent the report, and the repository name is
    /// substituted after it, so a report whose subject mentioned `{nwo}` used to have the
    /// repository spliced into the middle of its own argument.
    #[test]
    fn a_value_that_looks_like_a_placeholder_is_not_substituted_again() {
        assert_eq!(
            render(
                "notify {message} {nwo}",
                &[
                    ("message", Sub::Quoted("repo: {nwo}")),
                    ("nwo", Sub::Quoted("my repo")),
                ]
            ),
            "notify 'repo: {nwo}' 'my repo'"
        );
    }

    /// The same, proven where it matters: through a real shell, counting the arguments that
    /// come out the other side. String equality alone would not catch a `$(…)` that ends up
    /// outside the quotes meant to contain it.
    #[test]
    fn an_injected_value_cannot_add_an_argument_or_run_a_command() {
        let rendered = render(
            "printf '%s\\n' {message} {nwo}",
            &[
                ("message", Sub::Quoted("subject: {nwo} {message}")),
                ("nwo", Sub::Quoted("$(echo pwned)/a repo")),
            ],
        );
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&rendered)
            .output()
            .unwrap();
        // Two arguments, both verbatim. Substituted again, the repository name would have
        // landed inside the message's quotes and split it in two, and its `$(…)` would have
        // been outside them — the shell would have printed `pwned/a repo`.
        assert_eq!(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .collect::<Vec<_>>(),
            ["subject: {nwo} {message}", "$(echo pwned)/a repo"],
            "{rendered}"
        );
    }

    #[test]
    fn join_quotes_every_part_independently() {
        assert_eq!(
            sh_join(&["claude", "--permission-mode", "auto", "go now"]),
            "claude --permission-mode auto 'go now'"
        );
    }
}

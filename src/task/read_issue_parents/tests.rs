use super::*;

// GitHub's upper-case enum values are passed in rather than written into the JSON, so that no
// fixture holds a bare upper-case string after a colon: the guard over everything that ships
// reads one as a tracker key.
fn parent_json(number: u64, state: &str, total: u32, completed: u32) -> String {
    format!(
        r#"{{"url":"https://github.com/acme/widget/issues/{number}","number":{number},"title":"The parent","state":"{state}","subIssuesSummary":{{"total":{total},"completed":{completed}}}}}"#
    )
}

#[test]
fn the_query_names_each_issue_by_a_variable() {
    let query = parent_query(2);
    for i in 0..2 {
        assert!(query.contains(&format!("$o{i}:String!,$r{i}:String!,$p{i}:Int!")));
        assert!(query.contains(&format!(
            "i{i}:repository(owner:$o{i},name:$r{i}){{issue(number:$p{i}){{parent{{url number title state subIssuesSummary{{total completed}}}}}}}}"
        )));
    }
    assert!(!query.contains("i2"));
    // The stub gh of the board's tests tells this query by `parent{`.
    assert!(query.contains("parent{"));
}

#[test]
fn an_issue_url_is_read_into_the_names_github_compares_by() {
    let r = issue_ref_of("https://GitHub.com/Acme/Widget/issues/7?x=1#top").unwrap();
    assert_eq!(
        (r.host.as_str(), r.owner.as_str(), r.repo.as_str(), r.number),
        ("github.com", "acme", "widget", 7)
    );
    assert_eq!(issue_ref_of("https://github.com/acme/widget/pull/7"), None);
    assert_eq!(issue_ref_of("https://github.com/-a/widget/issues/7"), None);
    assert_eq!(issue_ref_of("ALPHA-7"), None);
}

#[test]
fn a_parent_and_none_and_a_missing_issue_are_each_their_own_answer() {
    let stdout = format!(
        r#"{{"data":{{"i0":{{"issue":{{"parent":{}}}}},"i1":{{"issue":{{"parent":null}}}},"i2":null}},"errors":[{{"type":"{}","path":["i2"],"message":"Could not resolve to an issue"}}]}}"#,
        parent_json(549, "OPEN", 9, 2),
        "NOT_FOUND"
    );
    let (answers, failed) = answers_of(&stdout, "", 3);
    // One issue that is not found is that issue's business, not the round failing.
    assert_eq!(failed, None);
    let first = answers[0].clone().unwrap().unwrap();
    assert_eq!(first.number, 549);
    assert_eq!(first.url, "https://github.com/acme/widget/issues/549");
    assert_eq!(first.title, "The parent");
    assert!(first.open);
    assert_eq!((first.total, first.completed), (9, 2));
    assert_eq!(answers[1], Ok(None));
    assert_eq!(answers[2], Err("Could not resolve to an issue".to_string()));
}

#[test]
fn a_closed_parent_is_not_open() {
    let stdout = format!(
        r#"{{"data":{{"i0":{{"issue":{{"parent":{}}}}}}}}}"#,
        parent_json(1, "CLOSED", 1, 1)
    );
    let (answers, _) = answers_of(&stdout, "", 1);
    assert!(!answers[0].clone().unwrap().unwrap().open);
}

#[test]
fn an_answer_without_data_is_the_round_failing() {
    let stdout = format!(
        r#"{{"errors":[{{"type":"{}","message":"API rate limit exceeded"}}]}}"#,
        "RATE_LIMITED"
    );
    let (answers, failed) = answers_of(&stdout, "", 2);
    assert_eq!(failed.as_deref(), Some("API rate limit exceeded"));
    assert!(answers.iter().all(|a| a.is_err()));
    let (answers, failed) = answers_of("", "gh: not logged in", 1);
    assert_eq!(failed.as_deref(), Some("gh: not logged in"));
    assert!(answers[0].is_err());
}

#[test]
fn a_transient_error_beside_some_data_still_fails_the_round_and_keeps_the_answers() {
    let stdout = format!(
        r#"{{"data":{{"i0":{{"issue":{{"parent":null}}}}}},"errors":[{{"type":"{}","message":"slow"}}]}}"#,
        "TIMEOUT"
    );
    let (answers, failed) = answers_of(&stdout, "", 1);
    assert_eq!(failed.as_deref(), Some("slow"));
    assert_eq!(answers[0], Ok(None));
}

#[test]
fn an_error_under_an_issue_is_a_failed_read_of_it_and_not_an_answer() {
    // Errors on the parent field, and on what is read beside it, with the issue itself present.
    for path in [
        r#"["i0","issue","parent"]"#,
        r#"["i0","issue","parent","subIssuesSummary"]"#,
        r#"["i0","issue","parent","title"]"#,
    ] {
        let stdout = format!(
            r#"{{"data":{{"i0":{{"issue":{{"parent":null}}}},"i1":{{"issue":{{"parent":null}}}}}},"errors":[{{"type":"{}","path":{path},"message":"field refused"}}]}}"#,
            "FORBIDDEN"
        );
        let (answers, failed) = answers_of(&stdout, "", 2);
        assert_eq!(answers[0], Err("field refused".to_string()), "{path}");
        // The other issue had no error: a parent that is null there is an answer.
        assert_eq!(answers[1], Ok(None), "{path}");
        // Tied to one alias, so not the round failing.
        assert_eq!(failed, None, "{path}");
    }
}

#[test]
fn a_null_parent_with_no_error_is_no_parent() {
    let stdout = r#"{"data":{"i0":{"issue":{"parent":null}}}}"#;
    let (answers, failed) = answers_of(stdout, "", 1);
    assert_eq!(answers[0], Ok(None));
    assert_eq!(failed, None);
}

#[test]
fn the_query_for_an_issue_itself_asks_for_its_own_fields_by_the_same_variables() {
    let query = issue_query(1);
    assert!(query.contains("$o0:String!,$r0:String!,$p0:Int!"));
    assert!(query.contains(
        "i0:repository(owner:$o0,name:$r0){issue(number:$p0){url number title state subIssuesSummary{total completed}}}"
    ));
    // It is the issue that is asked for, not its parent.
    assert!(!query.contains("parent{"));
}

#[test]
fn an_issue_read_on_its_own_is_found_with_its_counts() {
    let stdout = format!(
        r#"{{"data":{{"i0":{{"issue":{}}}}}}}"#,
        parent_json(686, "OPEN", 6, 2)
    );
    let IssueSummaryRead::Found(found) = summary_of(&stdout, "") else {
        panic!("not found");
    };
    assert_eq!((found.number, found.title.as_str()), (686, "The parent"));
    assert_eq!((found.total, found.completed), (6, 2));
}

#[test]
fn an_issue_that_is_not_there_is_told_by_the_error_s_type_not_its_message() {
    let said = |kind: &str, message: &str| {
        format!(
            r#"{{"data":{{"i0":{{"issue":null}}}},"errors":[{{"type":"{kind}","path":["i0","issue"],"message":"{message}"}}]}}"#
        )
    };
    assert_eq!(
        summary_of(&said("NOT_FOUND", "anything at all"), ""),
        IssueSummaryRead::NotFound
    );
    // Another error under the alias is not "no such issue", whatever it says.
    assert_eq!(
        summary_of(&said("FORBIDDEN", "Could not resolve to an issue"), ""),
        IssueSummaryRead::Failed("Could not resolve to an issue".to_string())
    );
}

#[test]
fn an_issue_read_that_came_to_nothing_usable_failed() {
    let transient = format!(
        r#"{{"data":{{"i0":null}},"errors":[{{"type":"{}","message":"API rate limit exceeded"}}]}}"#,
        "RATE_LIMITED"
    );
    assert_eq!(
        summary_of(&transient, ""),
        IssueSummaryRead::Failed("API rate limit exceeded".to_string())
    );
    assert_eq!(
        summary_of("", "gh: not logged in"),
        IssueSummaryRead::Failed("gh: not logged in".to_string())
    );
    assert!(matches!(
        summary_of(r#"{"data":{"i0":{"issue":null}}}"#, ""),
        IssueSummaryRead::Failed(_)
    ));
    assert!(matches!(
        summary_of(r#"{"data":{"i0":{"issue":{"title":"No url"}}}}"#, ""),
        IssueSummaryRead::Failed(_)
    ));
}

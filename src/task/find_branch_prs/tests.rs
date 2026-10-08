use super::*;

fn branch(owner: &str) -> BranchRef {
    BranchRef {
        host: "github.com".to_string(),
        owner: owner.to_string(),
        repo: "r".to_string(),
        branch: "feature".to_string(),
    }
}

// GitHub's upper-case enum values are passed in rather than written into the JSON, so that no
// fixture holds a bare upper-case string after a colon: the guard over everything that ships
// reads one as a tracker key.
fn node(number: u64, state: &str, draft: bool, owner: &str) -> String {
    format!(
        r#"{{"number":{number},"url":"https://github.com/o/r/pull/{number}","state":"{state}","isDraft":{draft},"headRepositoryOwner":{{"login":"{owner}"}}}}"#
    )
}

fn data(repos: &[Vec<String>]) -> String {
    let aliases: Vec<String> = repos
        .iter()
        .enumerate()
        .map(|(i, nodes)| {
            format!(
                r#""b{i}":{{"pullRequests":{{"nodes":[{}]}}}}"#,
                nodes.join(",")
            )
        })
        .collect();
    format!(r#"{{"data":{{{}}}}}"#, aliases.join(","))
}

fn read(repos: &[Vec<String>]) -> Result<Option<BranchPr>, String> {
    let refs = [branch("o")];
    parse_branch_prs(&data(repos), &[&refs[0]]).remove(0)
}

#[test]
fn the_query_names_each_branch_by_a_variable() {
    let query = branch_query(2);
    for i in 0..2 {
        assert!(query.contains(&format!("$o{i}:String!,$r{i}:String!,$h{i}:String!")));
        assert!(query.contains(&format!(
            "b{i}:repository(owner:$o{i},name:$r{i}){{pullRequests(headRefName:$h{i},"
        )));
    }
    assert!(!query.contains("b2"));
    assert!(query.contains("states:[OPEN,MERGED,CLOSED]"));
    assert!(query.contains("orderBy:{field:CREATED_AT,direction:DESC}"));
}

#[test]
fn an_open_pr_beats_a_newer_merged_one() {
    let got = read(&[vec![
        node(9, "MERGED", false, "o"),
        node(7, "OPEN", false, "o"),
    ]]);
    let pr = got.unwrap().unwrap();
    assert_eq!((pr.number, pr.state.as_str()), (7, "open"));
    assert_eq!(pr.url, "https://github.com/o/r/pull/7");
}

#[test]
fn the_newest_is_taken_when_none_is_open_and_a_draft_is_a_draft() {
    let got = read(&[vec![
        node(9, "CLOSED", false, "o"),
        node(7, "MERGED", false, "o"),
    ]]);
    assert_eq!(got.unwrap().unwrap().state, "closed");
    let got = read(&[vec![node(4, "OPEN", true, "o")]]);
    assert_eq!(got.unwrap().unwrap().state, "draft");
}

#[test]
fn a_pr_from_a_fork_is_not_the_branchs() {
    let got = read(&[vec![
        node(9, "OPEN", false, "someone"),
        node(7, "MERGED", false, "O"),
    ]]);
    assert_eq!(got.unwrap().unwrap().number, 7);
    let got = read(&[vec![node(9, "OPEN", false, "someone")]]);
    assert_eq!(got, Ok(None));
}

#[test]
fn a_branch_with_no_pr_has_none() {
    assert_eq!(read(&[vec![]]), Ok(None));
}

#[test]
fn a_repository_that_is_not_there_is_an_error_for_that_branch_alone() {
    let refs = [branch("o"), branch("p")];
    let both = [&refs[0], &refs[1]];
    let stdout = format!(
        r#"{{"data":{{"b0":null,"b1":{{"pullRequests":{{"nodes":[{}]}}}}}},"errors":[{{"type":"NOT_FOUND","path":["b0"],"message":"Could not resolve to a Repository"}}]}}"#,
        node(3, "OPEN", false, "p")
    );
    let got = parse_branch_prs(&stdout, &both);
    assert_eq!(got[0], Err("Could not resolve to a Repository".to_string()));
    assert_eq!(got[1].as_ref().unwrap().as_ref().unwrap().number, 3);
    // That is one repository's business, not the round failing.
    let (_, failed) = answers_of(&stdout, "", &both);
    assert_eq!(failed, None);
}

#[test]
fn a_rate_limit_fails_the_round_and_every_branch_in_it() {
    let refs = [branch("o"), branch("p")];
    let both = [&refs[0], &refs[1]];
    let limited =
        r#"{"data":null,"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded"}]}"#;
    let (answers, failed) = answers_of(limited, "", &both);
    assert_eq!(failed.as_deref(), Some("API rate limit exceeded"));
    assert!(answers.iter().all(|a| a.is_err()));
    let (answers, failed) = answers_of("", "gh: not logged in", &both);
    assert_eq!(failed.as_deref(), Some("gh: not logged in"));
    assert_eq!(answers.len(), 2);
}

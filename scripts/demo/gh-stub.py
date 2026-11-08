#!/usr/bin/env python3
# A gh that never reaches GitHub. It answers the calls the demo board and its scripted agents
# make from the pull requests in prs/, and fails every other call the way gh does offline.
import json, os, re, sys

HERE = os.path.dirname(os.path.realpath(__file__))
ROOT = os.environ.get("DEMO_ROOT", "/tmp/adj-demo")
PRS = json.load(open(os.path.join(HERE, "prs", "prs.json")))
ME = "demo"
args = sys.argv[1:]
with open(os.path.join(ROOT, "gh.log"), "a") as log:
    log.write(" ".join(args)[:400] + "\n")


def out(value):
    sys.stdout.write(value if isinstance(value, str) else json.dumps(value))
    sys.exit(0)


def fail(message="gh: not available in the demo"):
    sys.stderr.write(message + "\n")
    sys.exit(1)


def diff_of(number):
    path = os.path.join(HERE, "prs", f"{number}.diff")
    if not os.path.exists(path):
        return ""
    return open(path).read()


def files_of(number):
    files, cur = [], None
    for line in diff_of(number).splitlines():
        if line.startswith("diff --git "):
            cur = {"path": line.split(" b/", 1)[1], "additions": 0, "deletions": 0, "changeType": "MODIFIED"}
            files.append(cur)
        elif line.startswith("new file"):
            cur["changeType"] = "ADDED"
        elif line.startswith("+") and not line.startswith("+++"):
            cur["additions"] += 1
        elif line.startswith("-") and not line.startswith("---"):
            cur["deletions"] += 1
    return files


def pr(nwo, number):
    return PRS.get(f"{nwo}#{number}")


def url(nwo, number):
    return f"https://github.com/{nwo}/pull/{number}"


def graphql_pr(nwo, number):
    p = pr(nwo, number)
    if not p:
        return None
    files = files_of(number)
    commits = [{"commit": {"oid": p["sha"] if i == len(p["commits"]) - 1 else f"{i:040x}",
                           "committedDate": "2026-11-03T09:00:00Z",
                           "statusCheckRollup": {"state": "SUCCESS", "contexts": {"checkRunCountsByState": [{"state": "SUCCESS", "count": 3}], "statusContextCountsByState": []}}}}
               for i in range(len(p["commits"]))]
    requested = [{"requestedReviewer": {"__typename": "User", "login": ME}}] if p["requested"] else []
    for reviewer in p.get("reviewers", []):
        requested.append({"requestedReviewer": {"__typename": "User", "login": reviewer}})
    return {
        "number": number, "url": url(nwo, number), "title": p["title"], "state": "OPEN", "isDraft": False,
        "author": {"login": p["author"]}, "baseRefName": "main", "headRefName": p["head"], "headRefOid": p["sha"],
        "additions": sum(f["additions"] for f in files), "deletions": sum(f["deletions"] for f in files),
        "changedFiles": len(files), "files": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": files},
        "reviewRequests": {"totalCount": len(requested), "nodes": requested},
        "latestReviews": {"nodes": []}, "myReviews": {"nodes": []},
        "requests": {"nodes": [{"createdAt": "2026-11-03T09:10:00Z", "requestedReviewer": {"login": ME}}] if p["requested"] else []},
        "commits": {"nodes": commits}, "lastCommit": {"nodes": commits[-1:]},
        "reviewDecision": p.get("decision", "REVIEW_REQUIRED"), "latestOpinionatedReviews": {"nodes": []},
    }


def flag_values(flag):
    vals = {}
    for i, a in enumerate(args):
        if a in ("-f", "-F") and i + 1 < len(args) and "=" in args[i + 1]:
            k, v = args[i + 1].split("=", 1)
            vals[k] = v
    return vals


def pr_from_url(u):
    m = re.match(r"https://github\.com/([^/]+/[^/]+)/pull/(\d+)", u)
    return (m.group(1), int(m.group(2))) if m else (None, None)


head = args[:2]
if args[:1] == ["--version"]:
    out("gh version 2.0.0 (demo stub)\n")
if head == ["auth", "status"]:
    out("github.com\n  ✓ Logged in to github.com account demo (demo)\n")
if head == ["api", "user"]:
    out(ME + "\n")
if head == ["search", "prs"]:
    out([{"number": int(k.split("#")[1]), "url": url(*k.split("#")), "repository": {"nameWithOwner": k.split("#")[0]}}
         for k, p in PRS.items() if p["requested"]])
if head == ["api", "graphql"] and "--input" not in args:
    vals = flag_values("-f")
    data = {}
    i = 0
    while f"p{i}" in vals:
        data[f"p{i}"] = {"pullRequest": graphql_pr(f"{vals[f'o{i}']}/{vals[f'r{i}']}", int(vals[f"p{i}"]))}
        i += 1
    out({"data": data})
if head == ["api", "graphql"]:
    # The pending review, one step at a time: find none, make one, add its threads, set its body.
    q = json.loads(sys.stdin.read() or "{}").get("query", "")
    review = {"id": "PRR_demo", "url": "https://github.com/acme/shop-app/pull/42#pullrequestreview-1", "state": "PENDING", "body": ""}
    if "reviews(first:1,states:[PENDING]" in q:
        out({"data": {"repository": {"pullRequest": {"id": "PR_demo", "state": "OPEN", "reviews": {"nodes": []}}}}})
    if "addPullRequestReview(" in q:
        out({"data": {"addPullRequestReview": {"pullRequestReview": review}}})
    if "addPullRequestReviewThread" in q:
        out({"data": {a: {"thread": {"id": f"PRRT_{a}"}} for a in re.findall(r"(t\d+):addPullRequestReviewThread", q)}})
    if "updatePullRequestReview" in q:
        out({"data": {"updatePullRequestReview": {"pullRequestReview": {"id": "PRR_demo"}}}})
    fail("gh: this query is not answered in the demo")
if head == ["pr", "view"]:
    nwo, number = pr_from_url(args[2])
    p = pr(nwo, number) if nwo else None
    if not p:
        fail("no pull requests found")
    out({"number": number, "title": p["title"], "body": p["body"], "author": {"login": p["author"]},
         "baseRefName": "main", "headRefName": p["head"], "headRefOid": p["sha"],
         "additions": sum(f["additions"] for f in files_of(number)), "deletions": sum(f["deletions"] for f in files_of(number)),
         "files": [{"path": f["path"], "additions": f["additions"], "deletions": f["deletions"]} for f in files_of(number)],
         "commits": [{"oid": f"{i:040x}", "messageHeadline": m, "messageBody": ""} for i, m in enumerate(p["commits"])]})
if head == ["pr", "diff"]:
    nwo, number = pr_from_url(args[2])
    if nwo and pr(nwo, number):
        out(diff_of(number))
    fail("no pull requests found")
if args[:2] == ["api", "--paginate"]:
    out("[]")
if args[:1] == ["api"] and any(a.startswith("repos/") and a.endswith("/reviews") for a in args):
    out({"id": 1, "node_id": "PRR_demo", "state": "PENDING", "html_url": "https://github.com/acme/shop-app/pull/42#pullrequestreview-1"})
fail()

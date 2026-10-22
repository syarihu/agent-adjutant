# docs

Documents for working on and using agent-adjutant.

## User & Feature Guides

| Document | Description |
|---|---|
| [board.md](board.md) ([日本語](board.ja.md)) | Comprehensive guide to the web board, resident server, views, task panel, gates, and session operations |
| [jules.md](jules.md) ([日本語](jules.ja.md)) | Handing tasks to Google Jules, automated design planning, and PR review comment relay |
| [configuration.md](configuration.md) ([日本語](configuration.ja.md)) | Advanced runtime configuration, command templates, tmux backend, runners, and session lifecycle |

## Architecture & Development (for contributors and agents)

| Document | Description |
|---|---|
| [architecture.md](architecture.md) | How the crate is layered, what each module holds, the rules that come with the layers, and where a change goes |
| [board-columns.md](board-columns.md) | Design for board columns a team lays out itself, as conditions over facts adjutant knows, and the issues it splits into |
| [releasing.md](releasing.md) | How to cut a release and update the Homebrew formula in the tap |
| [review-checklist.md](review-checklist.md) | What to look for when reviewing a change to this repository, self-review included |
| [session-state.md](session-state.md) | Design for tracking what each agent session is doing (hooks, ledger, injection), and the issues it splits into |

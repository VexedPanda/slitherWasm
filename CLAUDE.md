# Slitherlink

## Git and GitHub

- **Commit identity:** always commit as `VexedPanda <734992+VexedPanda@users.noreply.github.com>`, not the machine's
  default git identity (a work account). For example:
  `git -c user.name=VexedPanda -c user.email=734992+VexedPanda@users.noreply.github.com commit ...`
- **Repository:** work goes to `VexedPanda/slitherWasm` (remote `origin`, base branch `main`). It was copied from
  `VexedPanda/slither` (remote `slither`), a fork of `jonathanolson/slither`; never push or open PRs against either.
- **GitHub account:** the active `gh` account may be a work account without access to the fork. Don't switch it; use
  the VexedPanda token per command:
  ```bash
  export GH_TOKEN=$(gh auth token -u VexedPanda)
  git -c credential.helper= -c "credential.helper=!f(){ echo username=VexedPanda; echo password=$GH_TOKEN; }; f" push -u origin <branch>
  gh pr create --repo VexedPanda/slitherWasm --base main --head <branch> ...
  ```
  Always pass `--repo VexedPanda/slitherWasm` to `gh pr` commands; without it `gh` may pick the wrong repository.
- **Branches and PRs:** use `claude/*` branches. Always open a PR for finished work and enable auto-merge
  (`gh pr merge <n> --repo VexedPanda/slitherWasm --auto --merge`), so it merges with a merge commit once CI (build, lint,
  typecheck, qunit) passes. No approval is needed before pushing, opening or merging; only stop to ask if
  CI fails in a way you can't fix.

## Checks

- `npm run typecheck`, `npm run lint`, and `npx prettier --check <files>` before committing.

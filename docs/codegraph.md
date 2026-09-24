# CodeGraph

[CodeGraph](https://github.com/colbymchenry/codegraph) supplies local source, call paths
and impact context for this repository. Install the verified CLI version once, then
initialize each checkout from its root:

```sh
npm install --global @colbymchenry/codegraph@1.6.0
codegraph --version
codegraph init --yes
codegraph status
```

The npm package includes its own runtime. Keep the npm global executable directory
on `PATH`. Each clone or worktree needs its own ignored `.codegraph/` index.

## Agent connection

Claude reads [`.mcp.json`](../.mcp.json); Codex reads
[`.codex/config.toml`](../.codex/config.toml). Both launch `codegraph serve --mcp`
from the checkout. [Claude settings](../.claude/settings.json) enable the project
server. Trust the checkout and restart the agent after configuration changes.
Repository configuration is already committed; `codegraph install` is unnecessary.

The default MCP tool is `codegraph_explore`. Pass `query` with a question, symbol
or file name; use `projectPath` to identify the checkout when needed. It returns
source and structural relationships. Use IDE semantic tools for exact references
and refactoring when available.

## CLI and review context

```sh
codegraph sync
codegraph explore "entry points and their callers" --max-files 5
codegraph affected --stdin --json
```

The last command reads changed source paths, one per line, from stdin; for example,
pipe `git diff --name-only <base> <head>` into it. Use `codegraph impact <symbol>`
for a symbol's dependents and `codegraph index` to rebuild the index.

MCP watches source edits and catches up on connection. Run `codegraph sync` before
scripted CLI queries. Before handing off a review, stop the writer, rebuild with
`codegraph index`, record `git rev-parse HEAD`, the review base and `codegraph status`,
then capture bounded exploration and affected-test results. Verify Git HEAD and
working-tree state stayed unchanged while capturing. Status describes the index;
it does not attest to a Git commit.

Pass that evidence to reviewers without MCP. Graph output guides discovery; it
does not replace source inspection, required tests or review coverage. Read
stale-file warnings and inspect affected files directly. Missing edges or tests
do not prove absence. If exploration finds nothing, try an indexed symbol name
or read the source directly. Documentation, configuration and unsupported
languages need direct inspection.

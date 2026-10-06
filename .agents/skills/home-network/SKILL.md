---
name: home-network
description: >-
  Work on FRANK, MACINDOZE or Mitch's MacBook Pro through SSH, SFTP or SMB.
  Use for remote debugging, file searches, command execution, file transfers
  and collecting iRacing data from the home network.
---

# Home network

Read the [access guide](../../../docs/home-network.md) for the machine map,
launcher setup, credentials and data-source rules. Run the repository's launcher
from its checkout; use `--help` for the available commands.

1. **Target.** Resolve the requested machine through the guide. For iRacing work,
   apply its source-of-truth rule before interpreting local copies. Run
   `<launcher> <machine> check`; proceed when the reported identity is the intended
   machine and SSH/SFTP both succeed. If it fails, report that machine and the
   connection, authentication or host-key error; keep the password out of output.
2. **Inspect.** Choose the branch that answers the task:
   - Debug a remote failure with `exec`: inspect the relevant processes, services,
     configuration and bounded log excerpts in the destination's native shell.
     Transfer larger logs with `get` for local analysis.
   - Search remote files with `ls`, then a directory-scoped `exec` search. Use
     PowerShell on Windows targets and the Mac's shell on the MacBook. Quote the
     entire remote command as one local argument. A successful search identifies
     exact remote paths; an empty search identifies the directory and filter tried.
   - Collect or deliver files with `get` or `put`. Choose exact destination paths;
     use `--recursive` for directories. For SMB, follow the guide's drive access
     rules. Collection is complete when the expected files and their source
     machine/path are established, with a collection time for changing data.
   - For other remote work, use `exec` within the user's requested scope.
     Interactive `shell` and `sftp` are available when a human needs a terminal.
3. **Verify.** When the task includes a fix, apply it to the intended remote
   machine, then rerun the failing operation there. Report the target, observed
   result and remaining limitation. For retrieval-only work, verify the local
   files before drawing conclusions from them.

Use the existing `HOME_NETWORK_PW` environment variable. When setup or the
credential is missing, follow the guide's preparation steps. Retain host-key
verification and existing filesystem permissions. A remote session carries the
same task scope as local work; the skill itself adds no authority to restart,
delete, deploy or change unrelated resources.
